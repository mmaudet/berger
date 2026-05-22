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

//! Integration test for `berger dry-run` (PRD §5.7): a single poll cycle
//! that applies no IMAP actions and records nothing — it only reports the
//! tags and actions Berger *would* apply.

use std::collections::BTreeMap;

use berger::cli::dry_run::{MessagePlan, plan};
use berger::config::TagActions;
use berger::ingest::error::IngestError;
use berger::ingest::source::MessageSource;
use berger::ingest::types::{DataPage, EmailSearchRequest, Envelope, MinimalAccount};
use berger::pipeline::compile_filters;
use berger::storage::database::Database;

/// A `MessageSource` that hands back a fixed page of envelopes and a fixed
/// EML body, and counts every `download_message` call so the test can prove
/// the dry run still reads, but only reads.
struct FakeSource {
    envelopes: Vec<Envelope>,
    eml: Vec<u8>,
}

impl MessageSource for FakeSource {
    async fn list_accounts(&self) -> Result<Vec<MinimalAccount>, IngestError> {
        Ok(Vec::new())
    }

    async fn search_messages(
        &self,
        _request: EmailSearchRequest,
    ) -> Result<DataPage<Envelope>, IngestError> {
        Ok(DataPage {
            current_page: Some(1),
            page_size: Some(200),
            total_items: self.envelopes.len() as u64,
            items: self.envelopes.clone(),
            total_pages: Some(1),
        })
    }

    async fn download_message(
        &self,
        _account_id: &str,
        _envelope_id: &str,
    ) -> Result<Vec<u8>, IngestError> {
        Ok(self.eml.clone())
    }
}

fn envelope(message_id: &str, mailbox: &str) -> Envelope {
    Envelope {
        id: format!("env-{message_id}"),
        message_id: message_id.to_string(),
        account_id: 1,
        account_email: None,
        mailbox_id: 1,
        mailbox_name: Some(mailbox.to_string()),
        uid: 7,
        subject: "Build passed".to_string(),
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

/// A filter that tags github.com senders `notif/github`, with a `move_to`
/// action wired for that tag.
fn filters_and_actions() -> (
    Vec<berger::pipeline::CompiledFilter>,
    BTreeMap<String, TagActions>,
) {
    use berger::config::FilterRule;
    let rules = [FilterRule {
        sender_in: Some(vec!["github.com".to_string()]),
        subject_regex: None,
        list_unsubscribe: None,
        header_match: None,
        tag: "notif/github".to_string(),
    }];
    let filters = compile_filters(&rules).unwrap();
    let mut actions = BTreeMap::new();
    actions.insert(
        "notif/github".to_string(),
        TagActions {
            move_to: Some("notifs/github".to_string()),
            mark_seen: true,
            ..TagActions::default()
        },
    );
    (filters, actions)
}

const EML: &[u8] =
    b"From: noreply@github.com\r\nSubject: Build passed\r\n\r\nThe build is green.\r\n";

#[tokio::test]
async fn dry_run_plans_the_tags_and_actions_without_touching_anything() {
    let database = Database::open(":memory:").unwrap();
    let source = FakeSource {
        envelopes: vec![envelope("<build@github.com>", "INBOX")],
        eml: EML.to_vec(),
    };
    let (filters, actions) = filters_and_actions();

    let plans = plan(&source, &[1], &filters, &actions, database.connection())
        .await
        .unwrap();

    assert_eq!(plans.len(), 1, "one account polled");
    let account_plan = &plans[0];
    assert_eq!(account_plan.messages.len(), 1);
    match &account_plan.messages[0] {
        MessagePlan::WouldProcess {
            message_id,
            tags,
            actions,
            ..
        } => {
            assert_eq!(message_id, "<build@github.com>");
            assert_eq!(tags, &["notif/github".to_string()]);
            // notif/github -> move_to: notifs/github + mark_seen.
            assert!(actions.iter().any(|a| a.contains("move_to")));
            assert!(actions.iter().any(|a| a.contains("mark_seen")));
        }
        MessagePlan::WouldSkip { .. } => panic!("a fresh message must be planned, not skipped"),
    }

    // The dry run recorded nothing: the sidecar is still empty.
    let processed: i64 = database
        .connection()
        .query_row("SELECT COUNT(*) FROM processed_messages", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(processed, 0, "dry-run must not record any message");
    let actions_logged: i64 = database
        .connection()
        .query_row("SELECT COUNT(*) FROM executed_actions", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(actions_logged, 0, "dry-run must not log any action");
}

#[tokio::test]
async fn dry_run_skips_a_message_already_in_the_sidecar() {
    // Seed processed_messages so the envelope looks already-triaged.
    let database = Database::open(":memory:").unwrap();
    let conn = database.connection();
    conn.execute(
        "INSERT INTO accounts (id, name, bichon_account_id) VALUES (1, 'acct', 'b-1')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO processed_messages (message_id, account_id, processed_at, berger_version, config_hash) \
         VALUES ('<seen@github.com>', 1, datetime('now'), '0.0.1', 'h')",
        [],
    )
    .unwrap();

    let source = FakeSource {
        envelopes: vec![envelope("<seen@github.com>", "INBOX")],
        eml: EML.to_vec(),
    };
    let (filters, actions) = filters_and_actions();
    let plans = plan(&source, &[1], &filters, &actions, conn).await.unwrap();

    match &plans[0].messages[0] {
        MessagePlan::WouldSkip { message_id, .. } => {
            assert_eq!(message_id, "<seen@github.com>");
        }
        MessagePlan::WouldProcess { .. } => {
            panic!("an already-processed message must be planned as skipped")
        }
    }
}

#[tokio::test]
async fn dry_run_ignores_messages_in_bergers_own_folders() {
    // The poller drops Berger/* (rule #1); a dry run sees nothing to plan.
    let database = Database::open(":memory:").unwrap();
    let source = FakeSource {
        envelopes: vec![envelope("<copied@github.com>", "Berger/notifs/github")],
        eml: EML.to_vec(),
    };
    let (filters, actions) = filters_and_actions();
    let plans = plan(&source, &[1], &filters, &actions, database.connection())
        .await
        .unwrap();
    assert!(
        plans[0].messages.is_empty(),
        "a message in a Berger/* folder must not be planned"
    );
}
