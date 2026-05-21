// Berger: open-source email triage daemon.
// Copyright (C) 2026 Michel-Marie Maudet
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Integration test for Bichon coherence rule #2 — Message-ID idempotence.
//!
//! Submits the same Message-ID through the pipeline three times and checks
//! the IMAP actions run exactly once (CLAUDE.md §4.4).

use std::collections::{BTreeMap, HashSet};

use berger::actions::error::ActionError;
use berger::actions::{ActionTarget, Flag};
use berger::config::TagActions;
use berger::filters::NativeFilter;
use berger::ingest::error::IngestError;
use berger::ingest::source::MessageSource;
use berger::ingest::types::{DataPage, EmailSearchRequest, Envelope, MinimalAccount};
use berger::pipeline::{CompiledFilter, Pipeline, ProcessOutcome};
use berger::storage::database::Database;

const EML: &[u8] = b"From: noreply@github.com\r\nSubject: A notification\r\n\r\nBody.\r\n";

/// A `MessageSource` that always hands back the same canned EML.
struct StaticSource;

impl MessageSource for StaticSource {
    async fn list_accounts(&self) -> Result<Vec<MinimalAccount>, IngestError> {
        Ok(Vec::new())
    }

    async fn search_messages(
        &self,
        _request: EmailSearchRequest,
    ) -> Result<DataPage<Envelope>, IngestError> {
        Ok(DataPage {
            current_page: None,
            page_size: None,
            total_items: 0,
            items: Vec::new(),
            total_pages: None,
        })
    }

    async fn download_message(
        &self,
        _account_id: &str,
        _envelope_id: &str,
    ) -> Result<Vec<u8>, IngestError> {
        Ok(EML.to_vec())
    }
}

/// An `ActionTarget` that records every copy / move / flag operation.
struct CountingTarget {
    existing: HashSet<String>,
    applied: Vec<String>,
}

impl CountingTarget {
    fn new() -> Self {
        Self {
            existing: HashSet::new(),
            applied: Vec::new(),
        }
    }
}

impl ActionTarget for CountingTarget {
    async fn folder_exists(&mut self, folder: &str) -> Result<bool, ActionError> {
        Ok(self.existing.contains(folder))
    }

    async fn create_folder(&mut self, folder: &str) -> Result<(), ActionError> {
        self.existing.insert(folder.to_string());
        Ok(())
    }

    async fn copy_message(&mut self, uid: u32, folder: &str) -> Result<(), ActionError> {
        self.applied.push(format!("copy:{uid}:{folder}"));
        Ok(())
    }

    async fn move_message(&mut self, uid: u32, folder: &str) -> Result<(), ActionError> {
        self.applied.push(format!("move:{uid}:{folder}"));
        Ok(())
    }

    async fn add_flag(&mut self, uid: u32, flag: Flag) -> Result<(), ActionError> {
        self.applied.push(format!("flag:{uid}:{flag:?}"));
        Ok(())
    }
}

fn test_envelope() -> Envelope {
    Envelope {
        id: "envelope-1".to_string(),
        message_id: "<idempotence@berger.test>".to_string(),
        account_id: 1,
        account_email: None,
        mailbox_id: 1,
        mailbox_name: Some("INBOX".to_string()),
        uid: 7,
        subject: "A notification".to_string(),
        preview: String::new(),
        from: "noreply@github.com".to_string(),
        to: Vec::new(),
        cc: Vec::new(),
        bcc: Vec::new(),
        date: 1_700_000_000_000,
        internal_date: 1_700_000_000_000,
        ingest_at: 1_700_000_000_000,
        size: 0,
        thread_id: String::new(),
        attachment_count: 0,
        regular_attachment_count: 0,
        tags: None,
        content_hash: String::new(),
    }
}

#[tokio::test]
async fn the_same_message_id_is_processed_only_once() {
    let database = Database::open(":memory:").unwrap();
    let account_id = database
        .accounts()
        .insert("test-account", "bichon-1")
        .unwrap();

    // A sender_in filter the test message matches, mapped to a copy action.
    let filters = vec![CompiledFilter {
        filter: NativeFilter::sender_in(vec!["github.com".to_string()]),
        tag: "cat/test".to_string(),
    }];
    let mut actions = BTreeMap::new();
    actions.insert(
        "cat/test".to_string(),
        TagActions {
            copy_to: Some("test".to_string()),
            ..TagActions::default()
        },
    );
    let pipeline = Pipeline::new(filters, actions, "test-config-hash".to_string());

    let source = StaticSource;
    let mut target = CountingTarget::new();
    let envelope = test_envelope();

    // First pass: the message is triaged.
    let first = pipeline
        .process(&envelope, account_id, &source, &database, &mut target)
        .await
        .unwrap();
    assert!(matches!(first, ProcessOutcome::Processed { .. }));

    // Two more passes of the very same Message-ID: both are skipped.
    for _ in 0..2 {
        let outcome = pipeline
            .process(&envelope, account_id, &source, &database, &mut target)
            .await
            .unwrap();
        assert_eq!(outcome, ProcessOutcome::AlreadyProcessed);
    }

    // The copy ran exactly once across the three submissions.
    let copies = target
        .applied
        .iter()
        .filter(|op| op.starts_with("copy:"))
        .count();
    assert_eq!(copies, 1, "the IMAP copy must run exactly once");
}
