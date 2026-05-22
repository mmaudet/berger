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

//! Integration test for the LLM filter (PRD §5.3, milestone J9): a message
//! is classified through a mocked OpenAI-compatible endpoint, and the
//! classification tags drive the IMAP actions — with a failover to the
//! `llm_error` tag when the model call fails.

use std::collections::{BTreeMap, HashSet};

use berger::actions::error::ActionError;
use berger::actions::{ActionTarget, Flag};
use berger::config::TagActions;
use berger::filters::NativeFilter;
use berger::ingest::error::IngestError;
use berger::ingest::source::MessageSource;
use berger::ingest::types::{DataPage, EmailSearchRequest, Envelope, MinimalAccount};
use berger::llm::LlmClient;
use berger::llm::classifier::Classifier;
use berger::pipeline::{CompiledFilter, Pipeline, ProcessOutcome};
use berger::storage::database::Database;
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const EML: &[u8] =
    b"From: colleague@example.test\r\nSubject: Project status\r\n\r\nPlease review the plan.\r\n";

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
        message_id: "<llm-pipeline@berger.test>".to_string(),
        account_id: 1,
        account_email: None,
        mailbox_id: 1,
        mailbox_name: Some("INBOX".to_string()),
        uid: 7,
        subject: "Project status".to_string(),
        preview: String::new(),
        from: "colleague@example.test".to_string(),
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

/// An LLM classifier wired to the mock server.
fn classifier_against(server: &MockServer) -> Classifier {
    let endpoint = format!("{}/v1/chat/completions", server.uri());
    let client = LlmClient::new(&endpoint, "test-model", None).unwrap();
    Classifier::new(client, "test-model".to_string(), vec!["work".to_string()])
}

#[tokio::test]
async fn the_llm_classification_drives_the_actions() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{"message": {"role": "assistant", "content":
                r#"{"category":"work","needs_reply":true,"priority":4}"#}}]
        })))
        .mount(&server)
        .await;

    let database = Database::open(":memory:").unwrap();
    let account_id = database.accounts().insert("acct", "bichon-1").unwrap();

    // No native filters: every tag below is produced by the LLM classifier.
    let mut actions = BTreeMap::new();
    actions.insert(
        "cat/work".to_string(),
        TagActions {
            copy_to: Some("work".to_string()),
            ..TagActions::default()
        },
    );
    let pipeline = Pipeline::new(
        Vec::new(),
        actions,
        "hash".to_string(),
        Some(classifier_against(&server)),
    );

    let source = StaticSource;
    let mut target = CountingTarget::new();
    let outcome = pipeline
        .process(
            &test_envelope(),
            account_id,
            &source,
            &database,
            &mut target,
        )
        .await
        .unwrap();

    match outcome {
        ProcessOutcome::Processed { tags, .. } => {
            assert!(tags.contains(&"cat/work".to_string()), "tags: {tags:?}");
            assert!(tags.contains(&"needs-reply".to_string()), "tags: {tags:?}");
            assert!(
                tags.contains(&"priority-high".to_string()),
                "tags: {tags:?}"
            );
        }
        ProcessOutcome::AlreadyProcessed => panic!("expected Processed"),
    }
    assert!(
        target.applied.iter().any(|op| op.starts_with("copy:")),
        "the cat/work tag must trigger the copy action: {:?}",
        target.applied
    );
}

#[tokio::test]
async fn an_llm_failure_tags_the_message_llm_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let database = Database::open(":memory:").unwrap();
    let account_id = database.accounts().insert("acct", "bichon-1").unwrap();
    let pipeline = Pipeline::new(
        Vec::new(),
        BTreeMap::new(),
        "hash".to_string(),
        Some(classifier_against(&server)),
    );

    let source = StaticSource;
    let mut target = CountingTarget::new();
    let outcome = pipeline
        .process(
            &test_envelope(),
            account_id,
            &source,
            &database,
            &mut target,
        )
        .await
        .unwrap();

    match outcome {
        ProcessOutcome::Processed { tags, .. } => {
            assert_eq!(tags, ["llm_error"], "an LLM failure must fail over");
        }
        ProcessOutcome::AlreadyProcessed => panic!("expected Processed"),
    }
}

#[tokio::test]
async fn process_records_tags_actions_and_filter_matches_in_the_sidecar() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{"message": {"role": "assistant", "content":
                r#"{"category":"work","needs_reply":false,"priority":2}"#}}]
        })))
        .mount(&server)
        .await;

    let database = Database::open(":memory:").unwrap();
    let account_id = database.accounts().insert("acct", "bichon-1").unwrap();

    // A native filter the EML sender matches, with a copy action for its tag.
    let filters = vec![CompiledFilter {
        filter: NativeFilter::sender_in(vec!["example.test".to_string()]),
        filter_type: "sender_in".to_string(),
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
    let pipeline = Pipeline::new(
        filters,
        actions,
        "hash".to_string(),
        Some(classifier_against(&server)),
    );

    let source = StaticSource;
    let mut target = CountingTarget::new();
    pipeline
        .process(
            &test_envelope(),
            account_id,
            &source,
            &database,
            &mut target,
        )
        .await
        .unwrap();

    let conn = database.connection();
    let tags: Vec<String> = conn
        .prepare("SELECT tag FROM applied_tags ORDER BY tag")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        tags,
        ["cat/test", "cat/work"],
        "applied_tags must record every tag"
    );

    let executed: Vec<(String, Option<String>)> = conn
        .prepare("SELECT action_type, target FROM executed_actions")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        executed,
        [("copy_to".to_string(), Some("test".to_string()))],
        "executed_actions must record the IMAP action"
    );

    let filter_types: Vec<String> = conn
        .prepare("SELECT filter_type FROM filter_matches ORDER BY filter_type")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        filter_types,
        ["llm", "sender_in"],
        "filter_matches must record the native filter and the LLM"
    );
}
