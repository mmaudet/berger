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

//! The LLM classifier: builds the prompt, consults the cache, calls the
//! model, and returns a typed [`Classification`] (PRD §5.3).

use std::hash::{DefaultHasher, Hash, Hasher};

use serde::Deserialize;

use crate::llm::LlmClient;
use crate::storage::error::StorageError;
use crate::storage::llm_decisions::{LlmDecision, LlmDecisionRepository};

/// How many characters of the body are sent to the model.
const MAX_BODY_CHARS: usize = 4000;

/// The typed classification the model is asked to return (PRD §5.3).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Classification {
    /// A short category label, e.g. `work`, `perso`, `newsletter`.
    pub category: String,
    /// Whether the email looks like it expects a personal reply.
    #[serde(default)]
    pub needs_reply: bool,
    /// Urgency on a 1–5 scale (0 when the model omits it).
    #[serde(default)]
    pub priority: u8,
}

/// The email content handed to the classifier.
#[derive(Debug, Clone)]
pub struct MessageContent {
    /// RFC 822 Message-ID — the first half of the cache key.
    pub message_id: String,
    /// The `From:` header, display form.
    pub from: String,
    /// The `Subject:` header.
    pub subject: String,
    /// The plain-text body (truncated before it reaches the model).
    pub body: String,
}

/// The result of classifying one message.
#[derive(Debug)]
pub enum ClassifyOutcome {
    /// The model classified the message.
    Classified {
        classification: Classification,
        /// A decision to persist — `Some` on a cache miss, `None` on a hit.
        /// Boxed to keep the enum variants close in size.
        decision: Option<Box<LlmDecision>>,
    },
    /// The LLM call or its output failed; the pipeline tags the message
    /// `llm_error` and carries on (PRD §5.3 failover).
    Failed,
}

/// Classifies messages through an OpenAI-compatible LLM, consulting the
/// `(message_id, prompt_hash)` cache before every call (PRD §5.3).
#[derive(Debug)]
pub struct Classifier {
    client: LlmClient,
    model: String,
    categories: Vec<String>,
}

impl Classifier {
    /// Builds a classifier over `client`, recording decisions under `model`
    /// and constraining the category to `categories` when non-empty.
    pub fn new(client: LlmClient, model: String, categories: Vec<String>) -> Self {
        Self {
            client,
            model,
            categories,
        }
    }

    /// Classifies one message: a cache lookup first, an LLM call on a miss,
    /// and a failover to [`ClassifyOutcome::Failed`] if the model errors or
    /// returns output that is not valid classification JSON.
    ///
    /// # Errors
    /// Returns [`StorageError`] only on a cache (SQLite) failure. An LLM
    /// failure is reported as [`ClassifyOutcome::Failed`], never an `Err`.
    pub async fn classify(
        &self,
        cache: &LlmDecisionRepository<'_>,
        message: &MessageContent,
    ) -> Result<ClassifyOutcome, StorageError> {
        let system = self.system_prompt();
        let user = user_prompt(message);
        let prompt_hash = hash_prompt(&system, &user);

        if let Some(cached) = cache.find_cached(&message.message_id, &prompt_hash)?
            && let Ok(classification) = serde_json::from_str::<Classification>(&cached)
        {
            return Ok(ClassifyOutcome::Classified {
                classification,
                decision: None,
            });
        }

        let started = std::time::Instant::now();
        let completion = match self.client.complete(&system, &user).await {
            Ok(completion) => completion,
            Err(error) => {
                tracing::warn!(
                    message_id = %message.message_id,
                    error = %error,
                    "LLM classification call failed; failing over to llm_error"
                );
                return Ok(ClassifyOutcome::Failed);
            }
        };
        let latency_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);

        let json = extract_json(&completion.content);
        let Ok(classification) = serde_json::from_str::<Classification>(json) else {
            tracing::warn!(
                message_id = %message.message_id,
                "LLM returned output that is not valid classification JSON"
            );
            return Ok(ClassifyOutcome::Failed);
        };

        let decision = LlmDecision {
            message_id: message.message_id.clone(),
            model: self.model.clone(),
            prompt_hash,
            prompt_text: format!("{system}\n\n{user}"),
            response_json: json.to_string(),
            tokens_input: completion.tokens_input,
            tokens_output: completion.tokens_output,
            latency_ms: Some(latency_ms),
            cost_usd: None,
        };
        Ok(ClassifyOutcome::Classified {
            classification,
            decision: Some(Box::new(decision)),
        })
    }

    /// The system prompt — instructions and the category vocabulary.
    fn system_prompt(&self) -> String {
        let mut prompt = String::from(
            "You are an email triage assistant. Classify the email below and \
             reply with ONLY a JSON object — no markdown fence, no prose:\n\
             {\"category\": \"...\", \"needs_reply\": true|false, \"priority\": 1-5}\n\
             - needs_reply: true if the email expects a personal reply from the reader.\n\
             - priority: 1 (trivial) to 5 (urgent).\n",
        );
        if self.categories.is_empty() {
            prompt.push_str("- category: a short lowercase label for the email.");
        } else {
            prompt.push_str("- category: exactly one of: ");
            prompt.push_str(&self.categories.join(", "));
        }
        prompt
    }
}

/// The user prompt — the email itself, with the body truncated.
fn user_prompt(message: &MessageContent) -> String {
    let body: String = message.body.chars().take(MAX_BODY_CHARS).collect();
    format!(
        "From: {}\nSubject: {}\n\n{}",
        message.from, message.subject, body
    )
}

/// A stable fingerprint of the prompt — the second half of the cache key.
fn hash_prompt(system: &str, user: &str) -> String {
    let mut hasher = DefaultHasher::new();
    system.hash(&mut hasher);
    user.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Extracts the first `{ … }` span, so a model that wraps its JSON in a
/// markdown fence or prose can still be parsed.
fn extract_json(text: &str) -> &str {
    match (text.find('{'), text.rfind('}')) {
        (Some(start), Some(end)) if start <= end => &text[start..=end],
        _ => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::database::Database;
    use crate::storage::processed_messages::ProcessedMessage;
    use serde_json::json;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn db_with_message(message_id: &str) -> Database {
        let db = Database::open(":memory:").unwrap();
        let account_id = db.accounts().insert("LINAGORA", "bichon-1").unwrap();
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

    fn classifier_for(server: &MockServer) -> Classifier {
        let endpoint = format!("{}/v1/chat/completions", server.uri());
        let client = LlmClient::new(&endpoint, "test-model", None).unwrap();
        Classifier::new(client, "test-model".to_string(), Vec::new())
    }

    fn sample_message(message_id: &str) -> MessageContent {
        MessageContent {
            message_id: message_id.to_string(),
            from: "colleague@example.test".to_string(),
            subject: "Project update".to_string(),
            body: "Please review the attached document.".to_string(),
        }
    }

    fn reply_with(content: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{"message": {"role": "assistant", "content": content}}]
        }))
    }

    #[tokio::test]
    async fn classify_returns_the_models_classification() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(reply_with(
                r#"{"category":"work","needs_reply":true,"priority":4}"#,
            ))
            .mount(&server)
            .await;
        let db = db_with_message("<m@x>");

        let outcome = classifier_for(&server)
            .classify(&db.llm_decisions(), &sample_message("<m@x>"))
            .await
            .unwrap();

        match outcome {
            ClassifyOutcome::Classified {
                classification,
                decision,
            } => {
                assert_eq!(classification.category, "work");
                assert!(classification.needs_reply);
                assert_eq!(classification.priority, 4);
                assert!(decision.is_some(), "a cache miss must yield a decision");
            }
            ClassifyOutcome::Failed => panic!("expected a classification"),
        }
    }

    #[tokio::test]
    async fn classify_records_the_token_usage_in_the_decision() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"role": "assistant", "content":
                    r#"{"category":"work","needs_reply":false,"priority":2}"#}}],
                "usage": {"prompt_tokens": 200, "completion_tokens": 18}
            })))
            .mount(&server)
            .await;
        let db = db_with_message("<m@x>");

        let outcome = classifier_for(&server)
            .classify(&db.llm_decisions(), &sample_message("<m@x>"))
            .await
            .unwrap();

        let ClassifyOutcome::Classified {
            decision: Some(decision),
            ..
        } = outcome
        else {
            panic!("a cache miss must yield a decision");
        };
        assert_eq!(decision.tokens_input, Some(200));
        assert_eq!(decision.tokens_output, Some(18));
    }

    #[tokio::test]
    async fn classify_reuses_a_cached_decision() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(reply_with(
                r#"{"category":"perso","needs_reply":false,"priority":2}"#,
            ))
            .expect(1) // called once; the second classify must hit the cache
            .mount(&server)
            .await;
        let db = db_with_message("<m@x>");
        let classifier = classifier_for(&server);
        let message = sample_message("<m@x>");

        let first = classifier
            .classify(&db.llm_decisions(), &message)
            .await
            .unwrap();
        let ClassifyOutcome::Classified {
            decision: Some(decision),
            ..
        } = first
        else {
            panic!("first call must be a cache miss");
        };
        db.llm_decisions().record(&decision).unwrap();

        let second = classifier
            .classify(&db.llm_decisions(), &message)
            .await
            .unwrap();
        assert!(matches!(
            second,
            ClassifyOutcome::Classified { decision: None, .. }
        ));
    }

    #[tokio::test]
    async fn classify_fails_over_when_the_llm_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let db = db_with_message("<m@x>");

        let outcome = classifier_for(&server)
            .classify(&db.llm_decisions(), &sample_message("<m@x>"))
            .await
            .unwrap();

        assert!(matches!(outcome, ClassifyOutcome::Failed));
    }

    #[tokio::test]
    async fn classify_fails_over_on_unparseable_output() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(reply_with("I cannot classify this email."))
            .mount(&server)
            .await;
        let db = db_with_message("<m@x>");

        let outcome = classifier_for(&server)
            .classify(&db.llm_decisions(), &sample_message("<m@x>"))
            .await
            .unwrap();

        assert!(matches!(outcome, ClassifyOutcome::Failed));
    }
}
