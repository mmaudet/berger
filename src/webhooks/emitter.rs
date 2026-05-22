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

//! The webhook emitter: renders the payload, POSTs it with a bounded retry
//! budget, and records the emission in the sidecar (PRD §5.6).

use std::collections::BTreeMap;

use handlebars::Handlebars;

use crate::storage::database::Database;
use crate::storage::webhook_emissions::WebhookEmission;
use crate::webhooks::config::WebhookConfig;
use crate::webhooks::error::WebhookError;
use crate::webhooks::payload::WebhookPayload;

/// The MIME type of the POSTed body.
const CONTENT_TYPE_JSON: &str = "application/json";

/// The outcome of one webhook delivery — the audit data the caller records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryReport {
    /// The exact JSON body that was POSTed.
    pub payload_json: String,
    /// HTTP status of the last attempt, when a response was received.
    pub http_status: Option<u16>,
    /// Number of attempts made (1 on a first-try success).
    pub attempts: u32,
    /// Whether the webhook ultimately succeeded.
    pub succeeded: bool,
}

/// Emits Berger's `berger.tag_applied` webhooks (PRD §5.6).
///
/// Holds the registry of named webhooks and a shared HTTP client. A tag's
/// `webhook:` action names one of these; [`emit`](WebhookEmitter::emit)
/// renders the payload, POSTs it with retries, and records the result.
pub struct WebhookEmitter {
    webhooks: BTreeMap<String, WebhookConfig>,
    http: reqwest::Client,
}

impl std::fmt::Debug for WebhookEmitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Webhook headers may hold secrets — list only the names.
        f.debug_struct("WebhookEmitter")
            .field("webhooks", &self.webhooks.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl WebhookEmitter {
    /// Builds an emitter over the configured webhooks, keyed by name.
    ///
    /// # Errors
    /// Returns [`WebhookError::Config`] if two webhooks share a name or the
    /// HTTP client cannot be built.
    pub fn new(webhooks: Vec<WebhookConfig>) -> Result<Self, WebhookError> {
        let mut registry = BTreeMap::new();
        for webhook in webhooks {
            if registry
                .insert(webhook.name.clone(), webhook.clone())
                .is_some()
            {
                return Err(WebhookError::Config(format!(
                    "duplicate webhook name `{}`",
                    webhook.name
                )));
            }
        }
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|error| WebhookError::Config(error.to_string()))?;
        Ok(Self {
            webhooks: registry,
            http,
        })
    }

    /// Whether any webhook is configured — lets the pipeline skip the work
    /// of building a payload when there is nothing to emit.
    pub fn is_empty(&self) -> bool {
        self.webhooks.is_empty()
    }

    /// Looks a webhook up by name.
    pub fn webhook(&self, name: &str) -> Option<&WebhookConfig> {
        self.webhooks.get(name)
    }

    /// Emits the webhook named `name` for `payload`, then records the
    /// emission in `database`.
    ///
    /// Emission is fire-and-forget (PRD §5.6): a webhook that exhausts its
    /// retry budget is logged and audited, never reported as an `Err`. An
    /// `Err` here means a *local* failure — an unknown webhook name, a
    /// payload that would not serialise, a broken template, or a sidecar
    /// write that failed — i.e. a Berger bug or misconfiguration, not a
    /// flaky consumer.
    ///
    /// # Errors
    /// Returns [`WebhookError`] on the local failures listed above.
    pub async fn emit(
        &self,
        name: &str,
        payload: &WebhookPayload,
        database: &Database,
    ) -> Result<DeliveryReport, WebhookError> {
        let webhook = self.webhooks.get(name).ok_or_else(|| {
            WebhookError::Config(format!("tag action references unknown webhook `{name}`"))
        })?;
        let body = render_body(webhook, payload)?;
        let report = self.deliver(webhook, &body).await;

        database.webhook_emissions().record(&WebhookEmission {
            message_id: payload.message.id.clone(),
            webhook_name: webhook.name.clone(),
            payload_json: report.payload_json.clone(),
            http_status: report.http_status.map(i64::from),
            attempts: i64::from(report.attempts),
            succeeded: report.succeeded,
        })?;
        Ok(report)
    }

    /// POSTs `body` to `webhook`, retrying transient failures up to the
    /// configured budget with the configured backoff (PRD §5.6).
    async fn deliver(&self, webhook: &WebhookConfig, body: &str) -> DeliveryReport {
        let max_attempts = webhook.retry.max_attempts.max(1);
        let mut last_status = None;
        for attempt in 1..=max_attempts {
            match self.post_once(webhook, body).await {
                PostResult::Success { status } => {
                    tracing::info!(
                        webhook = %webhook.name,
                        status,
                        attempt,
                        "webhook delivered"
                    );
                    return DeliveryReport {
                        payload_json: body.to_string(),
                        http_status: Some(status),
                        attempts: attempt,
                        succeeded: true,
                    };
                }
                PostResult::Retryable { status, reason } => {
                    last_status = status;
                    if attempt < max_attempts {
                        let delay = webhook.backoff_delay(attempt);
                        tracing::warn!(
                            webhook = %webhook.name,
                            attempt,
                            reason = %reason,
                            retry_in_secs = delay.as_secs(),
                            "webhook delivery failed; retrying"
                        );
                        tokio::time::sleep(delay).await;
                    }
                }
                PostResult::Permanent { status, reason } => {
                    // A 4xx will not improve on retry — stop immediately.
                    tracing::error!(
                        webhook = %webhook.name,
                        attempt,
                        status,
                        reason = %reason,
                        "webhook delivery failed permanently; giving up"
                    );
                    return DeliveryReport {
                        payload_json: body.to_string(),
                        http_status: Some(status),
                        attempts: attempt,
                        succeeded: false,
                    };
                }
            }
        }
        tracing::error!(
            webhook = %webhook.name,
            attempts = max_attempts,
            "webhook delivery abandoned after exhausting the retry budget"
        );
        DeliveryReport {
            payload_json: body.to_string(),
            http_status: last_status,
            attempts: max_attempts,
            succeeded: false,
        }
    }

    /// Performs one HTTP POST and classifies its outcome.
    async fn post_once(&self, webhook: &WebhookConfig, body: &str) -> PostResult {
        let method = match reqwest::Method::from_bytes(webhook.method.as_bytes()) {
            Ok(method) => method,
            Err(_) => {
                return PostResult::Permanent {
                    status: 0,
                    reason: format!("invalid HTTP method `{}`", webhook.method),
                };
            }
        };
        let mut request = self
            .http
            .request(method, &webhook.url)
            .header(reqwest::header::CONTENT_TYPE, CONTENT_TYPE_JSON)
            .body(body.to_string());
        for (name, value) in &webhook.headers {
            request = request.header(name, value);
        }

        match request.send().await {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    PostResult::Success {
                        status: status.as_u16(),
                    }
                } else if is_retryable_status(status) {
                    PostResult::Retryable {
                        status: Some(status.as_u16()),
                        reason: format!("HTTP {status}"),
                    }
                } else {
                    PostResult::Permanent {
                        status: status.as_u16(),
                        reason: format!("HTTP {status}"),
                    }
                }
            }
            // A transport error (connection refused, timeout, DNS) is
            // transient — worth retrying.
            Err(error) => PostResult::Retryable {
                status: None,
                reason: error.to_string(),
            },
        }
    }
}

/// The outcome of a single HTTP POST attempt.
enum PostResult {
    /// A 2xx response.
    Success { status: u16 },
    /// A transient failure — a 5xx, a 429, or a transport error.
    Retryable { status: Option<u16>, reason: String },
    /// A permanent failure — a 4xx (other than 429) or a bad method.
    Permanent { status: u16, reason: String },
}

/// Whether an HTTP status warrants a retry: server errors (5xx) and
/// "too many requests" (429) are transient; other 4xx are not (PRD §5.6).
fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS
}

/// Renders the body to POST: the canonical payload as JSON, or — when the
/// webhook declares a `template` — that Handlebars template rendered
/// against the canonical payload (PRD §5.6).
fn render_body(webhook: &WebhookConfig, payload: &WebhookPayload) -> Result<String, WebhookError> {
    match &webhook.template {
        None => Ok(serde_json::to_string(payload)?),
        Some(template) => {
            let handlebars = Handlebars::new();
            handlebars
                .render_template(template, payload)
                .map_err(|error| WebhookError::Template(error.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::types::Envelope;
    use crate::storage::processed_messages::ProcessedMessage;
    use crate::webhooks::payload::PayloadContext;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const EML: &[u8] = b"From: a@x.test\r\nSubject: s\r\n\r\nbody\r\n";

    fn test_envelope() -> Envelope {
        Envelope {
            id: "env-1".to_string(),
            message_id: "<emit@berger.test>".to_string(),
            account_id: 1,
            account_email: Some("user@example.test".to_string()),
            mailbox_id: 1,
            mailbox_name: Some("INBOX".to_string()),
            uid: 1,
            subject: "Hello".to_string(),
            preview: String::new(),
            from: "a@x.test".to_string(),
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            date: 0,
            internal_date: 0,
            ingest_at: 0,
            size: 0,
            thread_id: "t".to_string(),
            attachment_count: 0,
            regular_attachment_count: 0,
            tags: None,
            content_hash: String::new(),
        }
    }

    fn test_payload(envelope: &Envelope) -> WebhookPayload {
        let tags = ["cat/urgent".to_string()];
        WebhookPayload::build(
            &PayloadContext {
                envelope,
                eml: EML,
                tags: &tags,
                filters_matched: &[],
                classification: None,
                bichon_base_url: "https://bichon.test",
            },
            0,
        )
    }

    fn db_with_message(message_id: &str) -> Database {
        let db = Database::open(":memory:").unwrap();
        let account_id = db.accounts().insert("acct", "bichon-1").unwrap();
        db.processed_messages()
            .record(&ProcessedMessage {
                message_id: message_id.to_string(),
                account_id,
                bichon_uri: None,
                subject: None,
                from_email: None,
                from_name: None,
                date: None,
                berger_version: "0.0.1".to_string(),
                config_hash: "cfg".to_string(),
            })
            .unwrap();
        db
    }

    fn webhook(name: &str, url: String) -> WebhookConfig {
        serde_yaml_ng::from_str(&format!("name: {name}\nurl: {url}")).unwrap()
    }

    #[test]
    fn new_rejects_duplicate_webhook_names() {
        let webhooks = vec![
            webhook("dup", "https://a.test".to_string()),
            webhook("dup", "https://b.test".to_string()),
        ];
        assert!(matches!(
            WebhookEmitter::new(webhooks),
            Err(WebhookError::Config(_))
        ));
    }

    #[test]
    fn an_emitter_with_no_webhooks_is_empty() {
        assert!(WebhookEmitter::new(Vec::new()).unwrap().is_empty());
    }

    #[tokio::test]
    async fn emit_posts_the_canonical_payload_and_records_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/hook"))
            .and(header("content-type", "application/json"))
            .and(body_string_contains("berger.tag_applied"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let db = db_with_message("<emit@berger.test>");
        let emitter =
            WebhookEmitter::new(vec![webhook("hook", format!("{}/hook", server.uri()))]).unwrap();
        let envelope = test_envelope();

        let report = emitter
            .emit("hook", &test_payload(&envelope), &db)
            .await
            .unwrap();

        assert!(report.succeeded);
        assert_eq!(report.attempts, 1);
        assert_eq!(report.http_status, Some(200));
        assert_eq!(db.webhook_emissions().count().unwrap(), 1);
    }

    #[tokio::test]
    async fn emit_sends_the_configured_headers() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header("authorization", "Bearer tok"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let db = db_with_message("<emit@berger.test>");
        let config: WebhookConfig = serde_yaml_ng::from_str(&format!(
            "name: hook\nurl: {}/hook\nheaders:\n  Authorization: \"Bearer tok\"",
            server.uri()
        ))
        .unwrap();
        let emitter = WebhookEmitter::new(vec![config]).unwrap();
        let envelope = test_envelope();

        let report = emitter
            .emit("hook", &test_payload(&envelope), &db)
            .await
            .unwrap();
        assert!(report.succeeded);
    }

    #[tokio::test]
    async fn emit_rejects_an_unknown_webhook_name() {
        let db = db_with_message("<emit@berger.test>");
        let emitter = WebhookEmitter::new(Vec::new()).unwrap();
        let envelope = test_envelope();

        let result = emitter
            .emit("does-not-exist", &test_payload(&envelope), &db)
            .await;
        assert!(matches!(result, Err(WebhookError::Config(_))));
        // A local failure must not write an audit row.
        assert_eq!(db.webhook_emissions().count().unwrap(), 0);
    }

    #[tokio::test]
    async fn emit_retries_a_transient_failure_then_succeeds() {
        let server = MockServer::start().await;
        // First attempt: 503. Second attempt: 200.
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let db = db_with_message("<emit@berger.test>");
        // `fixed` backoff: a 1 s wait between attempts keeps the test quick.
        let config: WebhookConfig = serde_yaml_ng::from_str(&format!(
            "name: hook\nurl: {}/hook\nretry:\n  max_attempts: 3\n  backoff: fixed",
            server.uri()
        ))
        .unwrap();
        let emitter = WebhookEmitter::new(vec![config]).unwrap();
        let envelope = test_envelope();

        let report = emitter
            .emit("hook", &test_payload(&envelope), &db)
            .await
            .unwrap();

        assert!(report.succeeded, "the retry must eventually succeed");
        assert_eq!(report.attempts, 2);
        assert_eq!(report.http_status, Some(200));
    }

    #[tokio::test]
    async fn emit_gives_up_after_the_retry_budget_and_records_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .expect(3) // max_attempts: all consumed
            .mount(&server)
            .await;

        let db = db_with_message("<emit@berger.test>");
        let config: WebhookConfig = serde_yaml_ng::from_str(&format!(
            "name: hook\nurl: {}/hook\nretry:\n  max_attempts: 3\n  backoff: fixed",
            server.uri()
        ))
        .unwrap();
        let emitter = WebhookEmitter::new(vec![config]).unwrap();
        let envelope = test_envelope();

        // Fire-and-forget: an exhausted budget is Ok(report), never Err.
        let report = emitter
            .emit("hook", &test_payload(&envelope), &db)
            .await
            .unwrap();

        assert!(!report.succeeded);
        assert_eq!(report.attempts, 3);
        assert_eq!(report.http_status, Some(500));
        // The failed emission is still audited.
        let succeeded: bool = db
            .connection()
            .query_row("SELECT succeeded FROM webhook_emissions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(!succeeded);
    }

    #[tokio::test]
    async fn emit_does_not_retry_a_client_error() {
        let server = MockServer::start().await;
        // A 404 is permanent: exactly one attempt, no retry.
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;

        let db = db_with_message("<emit@berger.test>");
        let config: WebhookConfig = serde_yaml_ng::from_str(&format!(
            "name: hook\nurl: {}/hook\nretry:\n  max_attempts: 3",
            server.uri()
        ))
        .unwrap();
        let emitter = WebhookEmitter::new(vec![config]).unwrap();
        let envelope = test_envelope();

        let report = emitter
            .emit("hook", &test_payload(&envelope), &db)
            .await
            .unwrap();

        assert!(!report.succeeded);
        assert_eq!(report.attempts, 1, "a 4xx must not be retried");
        assert_eq!(report.http_status, Some(404));
    }

    #[tokio::test]
    async fn emit_renders_a_custom_handlebars_template() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("subject=Hello"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let db = db_with_message("<emit@berger.test>");
        let config: WebhookConfig = serde_yaml_ng::from_str(&format!(
            "name: hook\nurl: {}/hook\ntemplate: \"subject={{{{message.subject}}}}\"",
            server.uri()
        ))
        .unwrap();
        let emitter = WebhookEmitter::new(vec![config]).unwrap();
        let envelope = test_envelope();

        let report = emitter
            .emit("hook", &test_payload(&envelope), &db)
            .await
            .unwrap();

        assert!(report.succeeded);
        assert_eq!(
            report.payload_json, "subject=Hello",
            "the custom template must be rendered, not the canonical JSON"
        );
    }

    #[test]
    fn render_body_defaults_to_the_canonical_json() {
        let envelope = test_envelope();
        let webhook = webhook("hook", "https://x.test".to_string());
        let body = render_body(&webhook, &test_payload(&envelope)).unwrap();
        assert!(body.contains("\"event\":\"berger.tag_applied\""));
    }

    #[test]
    fn render_body_reports_a_broken_template() {
        let envelope = test_envelope();
        let config: WebhookConfig =
            serde_yaml_ng::from_str("name: hook\nurl: https://x.test\ntemplate: \"{{ unclosed\"")
                .unwrap();
        assert!(matches!(
            render_body(&config, &test_payload(&envelope)),
            Err(WebhookError::Template(_))
        ));
    }
}
