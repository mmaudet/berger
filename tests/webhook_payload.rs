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

//! Integration test for webhook emission (PRD §5.6, milestone J10).
//!
//! Drives a full `Pipeline::process` whose fired tag carries a `webhook:`
//! action, captures the JSON the pipeline POSTs to a mock endpoint, and
//! asserts it is *strictly* the canonical schema of PRD §5.6 — every field
//! present, no field extra, every type as specified (CLAUDE.md §4.4).

use std::collections::{BTreeMap, HashSet};

use berger::actions::error::ActionError;
use berger::actions::{ActionTarget, Flag};
use berger::config::{FilterRule, TagActions};
use berger::ingest::error::IngestError;
use berger::ingest::source::MessageSource;
use berger::ingest::types::{DataPage, EmailSearchRequest, Envelope, MinimalAccount};
use berger::llm::LlmClient;
use berger::llm::classifier::Classifier;
use berger::pipeline::{Pipeline, ProcessOutcome, compile_filters};
use berger::storage::database::Database;
use berger::webhooks::config::WebhookConfig;
use berger::webhooks::emitter::WebhookEmitter;
use serde::Deserialize;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A realistic RFC 822 message: a `From:` with a display name, a plain-text
/// body, and an HTML alternative.
const EML: &[u8] = b"From: Arnaud Clair <arnaud.clair@interieur.gouv.fr>\r\n\
To: Michel-Marie Maudet <michel-marie@linagora.com>\r\n\
Subject: Validation architecture Zero Trust RAG\r\n\
Content-Type: multipart/alternative; boundary=\"sep\"\r\n\
\r\n\
--sep\r\n\
Content-Type: text/plain\r\n\
\r\n\
Bonjour Michel-Marie, peux-tu valider l'architecture ?\r\n\
--sep\r\n\
Content-Type: text/html\r\n\
\r\n\
<p>Bonjour Michel-Marie, peux-tu valider l'architecture ?</p>\r\n\
--sep--\r\n";

/// A `MessageSource` that always returns the canned [`EML`].
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

/// An `ActionTarget` that accepts every operation — the IMAP side is not
/// under test here.
struct NoopTarget {
    existing: HashSet<String>,
}

impl ActionTarget for NoopTarget {
    async fn folder_exists(&mut self, folder: &str) -> Result<bool, ActionError> {
        Ok(self.existing.contains(folder))
    }

    async fn create_folder(&mut self, folder: &str) -> Result<(), ActionError> {
        self.existing.insert(folder.to_string());
        Ok(())
    }

    async fn copy_message(&mut self, _uid: u32, _folder: &str) -> Result<(), ActionError> {
        Ok(())
    }

    async fn move_message(&mut self, _uid: u32, _folder: &str) -> Result<(), ActionError> {
        Ok(())
    }

    async fn add_flag(&mut self, _uid: u32, _flag: Flag) -> Result<(), ActionError> {
        Ok(())
    }
}

/// The test envelope — its fields populate the webhook payload's `account`,
/// `message.id`, `message.thread_id`, `message.date` and recipients.
fn test_envelope() -> Envelope {
    Envelope {
        id: "abc-def".to_string(),
        message_id: "<abc-def@interieur.gouv.fr>".to_string(),
        account_id: 1,
        account_email: Some("michel-marie@linagora.com".to_string()),
        mailbox_id: 1,
        mailbox_name: Some("INBOX".to_string()),
        uid: 1234,
        subject: "Validation architecture Zero Trust RAG".to_string(),
        preview: String::new(),
        from: "Arnaud Clair <arnaud.clair@interieur.gouv.fr>".to_string(),
        to: vec!["Michel-Marie Maudet <michel-marie@linagora.com>".to_string()],
        cc: Vec::new(),
        bcc: Vec::new(),
        date: 1_779_165_280_000,
        internal_date: 1_779_165_280_000,
        ingest_at: 1_779_165_280_000,
        size: 0,
        thread_id: "thread-xyz".to_string(),
        attachment_count: 0,
        regular_attachment_count: 0,
        tags: None,
        content_hash: String::new(),
    }
}

// --- The canonical PRD §5.6 schema, as strict deserialization types. ---
//
// `deny_unknown_fields` makes the test fail if the payload carries a field
// the PRD does not list; every field below being non-`Option` makes it fail
// if the payload omits one. Together they pin the schema exactly.

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CanonicalPayload {
    event: String,
    berger_version: String,
    timestamp: String,
    account: String,
    tags: Vec<String>,
    filters_matched: Vec<String>,
    message: CanonicalMessage,
    classification: Option<CanonicalClassification>,
    bichon_message_uri: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CanonicalMessage {
    id: String,
    thread_id: String,
    from: CanonicalAddress,
    to: Vec<CanonicalAddress>,
    cc: Vec<CanonicalAddress>,
    subject: String,
    date: String,
    body_text: String,
    body_html: String,
    has_attachments: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CanonicalAddress {
    name: String,
    email: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CanonicalClassification {
    category: String,
    needs_reply: bool,
    priority: u8,
}

/// A `sender_in` filter on the sender's domain, tagged `delegate/christelle`.
fn sender_filter() -> FilterRule {
    FilterRule {
        sender_in: Some(vec!["interieur.gouv.fr".to_string()]),
        subject_regex: None,
        list_unsubscribe: None,
        header_match: None,
        tag: "delegate/christelle".to_string(),
    }
}

#[tokio::test]
async fn an_emitted_webhook_carries_the_canonical_payload() {
    // A mock endpoint that records every request body it receives.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webhook/berger/delegate"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let database = Database::open(":memory:").unwrap();
    let account_id = database.accounts().insert("LINAGORA", "bichon-1").unwrap();

    // The `delegate/christelle` tag (from the native filter) carries a
    // `webhook:` action naming `hermes-forward-christelle`.
    let mut actions = BTreeMap::new();
    actions.insert(
        "delegate/christelle".to_string(),
        TagActions {
            copy_to: Some("delegate/christelle".to_string()),
            webhook: Some("hermes-forward-christelle".to_string()),
            ..TagActions::default()
        },
    );

    let webhook: WebhookConfig = serde_yaml_ng::from_str(&format!(
        "name: hermes-forward-christelle\nurl: {}/webhook/berger/delegate",
        server.uri()
    ))
    .unwrap();
    let emitter = WebhookEmitter::new(vec![webhook]).unwrap();

    let filters = compile_filters(&[sender_filter()]).unwrap();
    let pipeline = Pipeline::new(filters, actions, "cfg-hash".to_string(), None)
        .with_webhooks(emitter, "https://bichon.linagora.io".to_string());

    let source = StaticSource;
    let mut target = NoopTarget {
        existing: HashSet::new(),
    };

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

    // The pipeline reports exactly one webhook emitted.
    match outcome {
        ProcessOutcome::Processed {
            webhooks_emitted, ..
        } => assert_eq!(webhooks_emitted, 1, "exactly one webhook must be emitted"),
        ProcessOutcome::AlreadyProcessed => panic!("expected Processed"),
    }

    // --- Capture the JSON the pipeline actually POSTed. ---
    let requests = server
        .received_requests()
        .await
        .expect("the mock server records requests");
    assert_eq!(requests.len(), 1, "exactly one POST must have been sent");
    let body = &requests[0].body;

    // It must parse cleanly into the strict canonical schema: a missing or
    // an extra field fails here.
    let payload: CanonicalPayload =
        serde_json::from_slice(body).expect("the body must be the canonical §5.6 schema");

    // --- Every field, against the PRD §5.6 contract. ---
    assert_eq!(payload.event, "berger.tag_applied");
    assert_eq!(payload.berger_version, env!("CARGO_PKG_VERSION"));
    // The timestamp is RFC 3339 UTC: `YYYY-MM-DDTHH:MM:SSZ`.
    assert!(
        is_rfc3339_utc(&payload.timestamp),
        "timestamp `{}` is not RFC 3339 UTC",
        payload.timestamp
    );
    assert_eq!(payload.account, "michel-marie@linagora.com");
    assert_eq!(payload.tags, ["delegate/christelle"]);
    assert!(
        !payload.filters_matched.is_empty(),
        "the firing filter must be reported in filters_matched"
    );

    let message = &payload.message;
    assert_eq!(message.id, "<abc-def@interieur.gouv.fr>");
    assert_eq!(message.thread_id, "thread-xyz");
    assert_eq!(message.from.name, "Arnaud Clair");
    assert_eq!(message.from.email, "arnaud.clair@interieur.gouv.fr");
    assert_eq!(message.to.len(), 1);
    assert_eq!(message.to[0].name, "Michel-Marie Maudet");
    assert_eq!(message.to[0].email, "michel-marie@linagora.com");
    assert!(message.cc.is_empty());
    assert_eq!(message.subject, "Validation architecture Zero Trust RAG");
    assert!(is_rfc3339_utc(&message.date), "message.date not RFC 3339");
    assert!(
        message.body_text.contains("Bonjour Michel-Marie"),
        "body_text: {:?}",
        message.body_text
    );
    assert!(
        message.body_html.contains("<p>Bonjour Michel-Marie"),
        "body_html: {:?}",
        message.body_html
    );
    assert!(!message.has_attachments);

    // No LLM ran in this pipeline, so `classification` is JSON `null`.
    assert!(
        payload.classification.is_none(),
        "classification must be null when no LLM ran"
    );

    assert_eq!(
        payload.bichon_message_uri,
        "https://bichon.linagora.io/api/v1/messages/abc-def"
    );

    // The emission is recorded in the sidecar's audit table (PRD §5.9).
    assert_eq!(database.webhook_emissions().count().unwrap(), 1);
}

#[tokio::test]
async fn the_payload_carries_the_classification_when_an_llm_ran() {
    // One mock server serves both roles, routed by path: `/llm` is the
    // OpenAI-compatible endpoint, `/webhook` receives the emission.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/llm"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{"message": {"role": "assistant", "content":
                r#"{"category":"work","needs_reply":true,"priority":5}"#}}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/webhook"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let database = Database::open(":memory:").unwrap();
    let account_id = database.accounts().insert("LINAGORA", "bichon-1").unwrap();

    // The LLM yields `cat/work`; that tag carries the `webhook:` action.
    let mut actions = BTreeMap::new();
    actions.insert(
        "cat/work".to_string(),
        TagActions {
            webhook: Some("linatwin-draft".to_string()),
            ..TagActions::default()
        },
    );
    let webhook: WebhookConfig = serde_yaml_ng::from_str(&format!(
        "name: linatwin-draft\nurl: {}/webhook",
        server.uri()
    ))
    .unwrap();
    let emitter = WebhookEmitter::new(vec![webhook]).unwrap();

    let llm_client = LlmClient::new(&format!("{}/llm", server.uri()), "test-model", None).unwrap();
    let classifier = Classifier::new(
        llm_client,
        "test-model".to_string(),
        vec!["work".to_string()],
    );

    // No native filters: the only tag comes from the LLM classifier.
    let pipeline = Pipeline::new(
        Vec::new(),
        actions,
        "cfg-hash".to_string(),
        Some(classifier),
    )
    .with_webhooks(emitter, "https://bichon.linagora.io".to_string());

    let source = StaticSource;
    let mut target = NoopTarget {
        existing: HashSet::new(),
    };
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

    let requests = server
        .received_requests()
        .await
        .expect("the mock server records requests");
    let webhook_body = requests
        .iter()
        .find(|request| request.url.path() == "/webhook")
        .expect("the webhook must have been POSTed");
    let payload: CanonicalPayload = serde_json::from_slice(&webhook_body.body)
        .expect("the body must be the canonical §5.6 schema");

    // The `classification` block must be present and reflect the LLM output.
    let classification = payload
        .classification
        .expect("classification must be populated when an LLM ran");
    assert_eq!(classification.category, "work");
    assert!(classification.needs_reply);
    assert_eq!(classification.priority, 5);
    // The classification tags appear alongside the category tag.
    assert!(payload.tags.contains(&"cat/work".to_string()));
    assert!(payload.tags.contains(&"needs-reply".to_string()));
    assert!(payload.tags.contains(&"priority-high".to_string()));
}

#[tokio::test]
async fn a_tag_with_no_webhook_action_emits_nothing() {
    // The mock endpoint must never be called: there is no `webhook:` action.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let database = Database::open(":memory:").unwrap();
    let account_id = database.accounts().insert("LINAGORA", "bichon-1").unwrap();

    // The fired tag carries only a `copy_to` — no webhook.
    let mut actions = BTreeMap::new();
    actions.insert(
        "delegate/christelle".to_string(),
        TagActions {
            copy_to: Some("delegate/christelle".to_string()),
            ..TagActions::default()
        },
    );
    let webhook: WebhookConfig =
        serde_yaml_ng::from_str(&format!("name: unused\nurl: {}/never", server.uri())).unwrap();
    let emitter = WebhookEmitter::new(vec![webhook]).unwrap();

    let filters = compile_filters(&[sender_filter()]).unwrap();
    let pipeline = Pipeline::new(filters, actions, "cfg-hash".to_string(), None)
        .with_webhooks(emitter, "https://bichon.linagora.io".to_string());

    let source = StaticSource;
    let mut target = NoopTarget {
        existing: HashSet::new(),
    };
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
        ProcessOutcome::Processed {
            webhooks_emitted, ..
        } => assert_eq!(webhooks_emitted, 0),
        ProcessOutcome::AlreadyProcessed => panic!("expected Processed"),
    }
    assert_eq!(database.webhook_emissions().count().unwrap(), 0);
}

#[tokio::test]
async fn a_webhook_failure_does_not_fail_the_message() {
    // The endpoint always 500s; the retry budget is exhausted. Emission is
    // fire-and-forget (PRD §5.6) — the message must still be triaged.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let database = Database::open(":memory:").unwrap();
    let account_id = database.accounts().insert("LINAGORA", "bichon-1").unwrap();

    let mut actions = BTreeMap::new();
    actions.insert(
        "delegate/christelle".to_string(),
        TagActions {
            webhook: Some("flaky".to_string()),
            ..TagActions::default()
        },
    );
    // `fixed` backoff keeps the two retry waits short.
    let webhook: WebhookConfig = serde_yaml_ng::from_str(&format!(
        "name: flaky\nurl: {}/hook\nretry:\n  max_attempts: 2\n  backoff: fixed",
        server.uri()
    ))
    .unwrap();
    let emitter = WebhookEmitter::new(vec![webhook]).unwrap();

    let filters = compile_filters(&[sender_filter()]).unwrap();
    let pipeline = Pipeline::new(filters, actions, "cfg-hash".to_string(), None)
        .with_webhooks(emitter, "https://bichon.linagora.io".to_string());

    let source = StaticSource;
    let mut target = NoopTarget {
        existing: HashSet::new(),
    };

    // The pipeline must return Ok despite the webhook failing every attempt.
    let outcome = pipeline
        .process(
            &test_envelope(),
            account_id,
            &source,
            &database,
            &mut target,
        )
        .await
        .expect("a webhook failure must not fail the pipeline");

    match outcome {
        ProcessOutcome::Processed { tags, .. } => {
            assert_eq!(tags, ["delegate/christelle"]);
        }
        ProcessOutcome::AlreadyProcessed => panic!("expected Processed"),
    }

    // The failed emission is still audited.
    let succeeded: bool = database
        .connection()
        .query_row("SELECT succeeded FROM webhook_emissions", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(
        !succeeded,
        "the exhausted emission must be recorded as failed"
    );
}

/// A loose RFC 3339 UTC check: `YYYY-MM-DDTHH:MM:SSZ`, second precision.
fn is_rfc3339_utc(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 20
        && text.ends_with('Z')
        && &text[4..5] == "-"
        && &text[7..8] == "-"
        && &text[10..11] == "T"
        && &text[13..14] == ":"
        && &text[16..17] == ":"
        && bytes[..4].iter().all(u8::is_ascii_digit)
}
