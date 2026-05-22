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

//! Pipeline: turning a polled message into tags and applied actions.

pub mod error;

use std::collections::BTreeMap;

use mail_parser::MessageParser;

use crate::actions::resolve::resolve_actions;
use crate::actions::{ActionTarget, apply_actions};
use crate::config::{FilterRule, TagActions};
use crate::filters::error::FilterError;
use crate::filters::{MessageView, NativeFilter};
use crate::ingest::source::MessageSource;
use crate::ingest::types::Envelope;
use crate::llm::classifier::{Classification, Classifier, ClassifyOutcome, MessageContent};
use crate::pipeline::error::PipelineError;
use crate::storage::database::Database;
use crate::storage::processed_messages::ProcessedMessage;
use crate::tags::{LLM_ERROR_TAG, classification_tags};
use crate::webhooks::emitter::WebhookEmitter;
use crate::webhooks::payload::{PayloadContext, WebhookPayload};

/// A [`FilterRule`] compiled into a runnable [`NativeFilter`] together with
/// the tag it emits when it matches.
#[derive(Debug)]
pub struct CompiledFilter {
    pub filter: NativeFilter,
    pub tag: String,
}

/// Parses a raw RFC 822 message into the [`MessageView`] the native filters
/// examine. An unparseable message yields an empty view.
pub fn parse_message_view(eml: &[u8]) -> MessageView {
    let Some(message) = MessageParser::default().parse(eml) else {
        return MessageView {
            from: String::new(),
            subject: String::new(),
            headers: Vec::new(),
        };
    };
    MessageView {
        from: message
            .header_raw("From")
            .unwrap_or_default()
            .trim()
            .to_string(),
        subject: message.subject().unwrap_or_default().to_string(),
        headers: message
            .headers_raw()
            .map(|(name, value)| (name.to_string(), value.trim().to_string()))
            .collect(),
    }
}

/// Extracts the plain-text body of a raw RFC 822 message, for the LLM
/// classifier. An unparseable message, or one with no text part, yields
/// an empty string.
fn parse_body(eml: &[u8]) -> String {
    MessageParser::default()
        .parse(eml)
        .and_then(|message| message.body_text(0).map(|text| text.into_owned()))
        .unwrap_or_default()
}

/// Compiles configured [`FilterRule`]s into runnable [`CompiledFilter`]s.
///
/// # Errors
/// Returns [`FilterError`] if a `subject_regex` or `header_match` pattern
/// is not a valid regex.
pub fn compile_filters(rules: &[FilterRule]) -> Result<Vec<CompiledFilter>, FilterError> {
    let mut compiled = Vec::with_capacity(rules.len());
    for rule in rules {
        let filter = if let Some(senders) = &rule.sender_in {
            NativeFilter::sender_in(senders.clone())
        } else if let Some(pattern) = &rule.subject_regex {
            NativeFilter::subject_regex(pattern)?
        } else if rule.list_unsubscribe == Some(true) {
            NativeFilter::list_unsubscribe()
        } else if let Some(spec) = &rule.header_match {
            NativeFilter::header_match(&spec.header, &spec.pattern)?
        } else {
            // Config validation guarantees exactly one filter type; skip
            // defensively rather than panic if that ever changes.
            continue;
        };
        compiled.push(CompiledFilter {
            filter,
            tag: rule.tag.clone(),
        });
    }
    Ok(compiled)
}

/// Runs every filter over `message` and returns the tags of those that
/// match, de-duplicated and in declaration order.
pub fn run_filters(filters: &[CompiledFilter], message: &MessageView) -> Vec<String> {
    let mut tags = Vec::new();
    for compiled in filters {
        if compiled.filter.matches(message) && !tags.contains(&compiled.tag) {
            tags.push(compiled.tag.clone());
        }
    }
    tags
}

/// Merges the native filter `tags` with the LLM classifier's `outcome`:
/// the classification tags on success, the `llm_error` tag on failure
/// (PRD §5.3). Tags already present are not duplicated.
fn merge_llm_tags(mut tags: Vec<String>, outcome: &ClassifyOutcome) -> Vec<String> {
    let llm_tags = match outcome {
        ClassifyOutcome::Classified { classification, .. } => classification_tags(classification),
        ClassifyOutcome::Failed => vec![LLM_ERROR_TAG.to_string()],
    };
    for tag in llm_tags {
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    tags
}

/// The outcome of running one message through the pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessOutcome {
    /// The message had already been processed; nothing was done (rule #2).
    AlreadyProcessed,
    /// The message was triaged.
    Processed {
        /// Tags emitted by the native filters and the LLM classifier.
        tags: Vec<String>,
        /// Number of IMAP actions applied.
        actions_applied: usize,
        /// Number of webhooks emitted (PRD §5.6).
        webhooks_emitted: usize,
    },
}

/// The triage pipeline: the compiled filters, the action map, the optional
/// LLM classifier, and the optional webhook emitter, applied to each polled
/// message.
#[derive(Debug)]
pub struct Pipeline {
    filters: Vec<CompiledFilter>,
    actions: BTreeMap<String, TagActions>,
    config_hash: String,
    classifier: Option<Classifier>,
    webhooks: Option<WebhookEmitter>,
    bichon_base_url: String,
}

impl Pipeline {
    /// Builds a pipeline from compiled filters, the action map, a hash of
    /// the configuration in force (recorded with every message), and an
    /// optional LLM classifier (PRD §5.3).
    ///
    /// Webhook emission is off by default; attach it with
    /// [`with_webhooks`](Pipeline::with_webhooks).
    pub fn new(
        filters: Vec<CompiledFilter>,
        actions: BTreeMap<String, TagActions>,
        config_hash: String,
        classifier: Option<Classifier>,
    ) -> Self {
        Self {
            filters,
            actions,
            config_hash,
            classifier,
            webhooks: None,
            bichon_base_url: String::new(),
        }
    }

    /// Attaches a [`WebhookEmitter`] so a fired tag's `webhook:` action
    /// POSTs the canonical event (PRD §5.6). `bichon_base_url` builds the
    /// payload's `bichon_message_uri`.
    #[must_use]
    pub fn with_webhooks(mut self, emitter: WebhookEmitter, bichon_base_url: String) -> Self {
        self.webhooks = Some(emitter);
        self.bichon_base_url = bichon_base_url;
        self
    }

    /// Processes one polled message: skips it if already seen (Bichon
    /// coherence rule #2), otherwise downloads its EML, runs the native
    /// filters and the LLM classifier, applies the resolved actions, and
    /// records it in the ledger.
    ///
    /// # Errors
    /// Returns [`PipelineError`] if downloading, an IMAP action, or a
    /// storage operation fails. A message is recorded as processed only
    /// after its actions have been applied.
    pub async fn process<S, T>(
        &self,
        envelope: &Envelope,
        account_id: i64,
        source: &S,
        database: &Database,
        target: &mut T,
    ) -> Result<ProcessOutcome, PipelineError>
    where
        S: MessageSource,
        T: ActionTarget,
    {
        if database
            .processed_messages()
            .is_already_processed(&envelope.message_id)?
        {
            return Ok(ProcessOutcome::AlreadyProcessed);
        }

        let eml = source
            .download_message(&envelope.account_id.to_string(), &envelope.id)
            .await?;
        let view = parse_message_view(&eml);
        let mut tags = run_filters(&self.filters, &view);

        // LLM filter (PRD §5.3): classify the message and merge in its tags.
        let mut llm_decision = None;
        let mut classification = None;
        if let Some(classifier) = &self.classifier {
            let content = MessageContent {
                message_id: envelope.message_id.clone(),
                from: envelope.from.clone(),
                subject: envelope.subject.clone(),
                body: parse_body(&eml),
            };
            let outcome = classifier
                .classify(&database.llm_decisions(), &content)
                .await?;
            tags = merge_llm_tags(tags, &outcome);
            if let ClassifyOutcome::Classified {
                classification: produced,
                decision,
            } = outcome
            {
                classification = Some(produced);
                llm_decision = decision;
            }
        }

        let actions = resolve_actions(&tags, &self.actions);
        apply_actions(target, envelope.uid, &actions).await?;

        database.processed_messages().record(&ProcessedMessage {
            message_id: envelope.message_id.clone(),
            account_id,
            bichon_uri: None,
            subject: Some(envelope.subject.clone()),
            from_email: Some(envelope.from.clone()),
            from_name: None,
            date: Some(envelope.date),
            berger_version: env!("CARGO_PKG_VERSION").to_string(),
            config_hash: self.config_hash.clone(),
        })?;

        // The LLM decision references processed_messages, so it is stored
        // only once the message itself has been recorded.
        if let Some(decision) = llm_decision {
            database.llm_decisions().record(&decision)?;
        }

        // Webhooks (PRD §5.6): emitted last, since webhook_emissions has a
        // foreign key onto the message just recorded above.
        let webhooks_emitted = self
            .emit_webhooks(envelope, &eml, &tags, classification.as_ref(), database)
            .await;

        Ok(ProcessOutcome::Processed {
            actions_applied: actions.len(),
            tags,
            webhooks_emitted,
        })
    }

    /// Emits every webhook a fired tag's `webhook:` action references, for
    /// a message that has just been recorded (PRD §5.6).
    ///
    /// Returns the count of webhooks for which a POST was attempted. Webhook
    /// delivery is fire-and-forget: a failed POST — or a failure to even
    /// build the payload — is logged, never propagated, so it cannot fail
    /// the triage of a message whose IMAP actions already succeeded.
    async fn emit_webhooks(
        &self,
        envelope: &Envelope,
        eml: &[u8],
        tags: &[String],
        classification: Option<&Classification>,
        database: &Database,
    ) -> usize {
        let Some(emitter) = &self.webhooks else {
            return 0;
        };
        if emitter.is_empty() {
            return 0;
        }

        // The distinct webhook names referenced by the message's tags,
        // collected in tag (declaration) order.
        let mut names: Vec<&str> = Vec::new();
        for tag in tags {
            if let Some(name) = self
                .actions
                .get(tag)
                .and_then(|tag_actions| tag_actions.webhook.as_deref())
                && !names.contains(&name)
            {
                names.push(name);
            }
        }
        if names.is_empty() {
            return 0;
        }

        let payload = WebhookPayload::build(
            &PayloadContext {
                envelope,
                eml,
                tags,
                filters_matched: tags,
                classification,
                bichon_base_url: &self.bichon_base_url,
            },
            now_epoch_ms(),
        );

        let mut emitted = 0;
        for name in names {
            // A webhook's own `when:` filter can narrow which tags fire it.
            match emitter.webhook(name) {
                Some(webhook) if !webhook.fires_for(tags) => {
                    tracing::debug!(
                        webhook = %name,
                        message_id = %envelope.message_id,
                        "webhook skipped: its `when:` filter excludes this message"
                    );
                    continue;
                }
                None => {
                    tracing::warn!(
                        webhook = %name,
                        message_id = %envelope.message_id,
                        "a tag action references a webhook absent from the configuration"
                    );
                    continue;
                }
                Some(_) => {}
            }
            match emitter.emit(name, &payload, database).await {
                Ok(_) => emitted += 1,
                Err(error) => tracing::error!(
                    webhook = %name,
                    message_id = %envelope.message_id,
                    error = %error,
                    "failed to emit a webhook"
                ),
            }
        }
        emitted
    }
}

/// The current time as epoch milliseconds, for stamping a webhook payload.
fn now_epoch_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or(0),
    )
    .unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HeaderMatchSpec;
    use crate::llm::classifier::Classification;

    const SAMPLE_EML: &[u8] = b"From: Alice <alice@example.test>\r\nSubject: Quarterly invoice\r\nList-Unsubscribe: <mailto:unsub@example.test>\r\n\r\nBody text.\r\n";

    fn filter_rule(
        tag: &str,
        sender_in: Option<&[&str]>,
        subject_regex: Option<&str>,
        list_unsubscribe: bool,
        header_match: Option<(&str, &str)>,
    ) -> FilterRule {
        FilterRule {
            sender_in: sender_in.map(|s| s.iter().map(|x| x.to_string()).collect()),
            subject_regex: subject_regex.map(str::to_string),
            list_unsubscribe: list_unsubscribe.then_some(true),
            header_match: header_match.map(|(header, pattern)| HeaderMatchSpec {
                header: header.to_string(),
                pattern: pattern.to_string(),
            }),
            tag: tag.to_string(),
        }
    }

    #[test]
    fn parse_message_view_extracts_from_subject_and_headers() {
        let view = parse_message_view(SAMPLE_EML);
        assert!(view.from.contains("alice@example.test"));
        assert_eq!(view.subject, "Quarterly invoice");
        assert!(
            view.headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("List-Unsubscribe"))
        );
    }

    #[test]
    fn parse_body_extracts_the_text_body() {
        let body = parse_body(SAMPLE_EML);
        assert!(body.contains("Body text."), "got: {body:?}");
    }

    #[test]
    fn merge_llm_tags_appends_the_classification_tags() {
        let outcome = ClassifyOutcome::Classified {
            classification: Classification {
                category: "work".to_string(),
                needs_reply: true,
                priority: 5,
            },
            decision: None,
        };
        let tags = merge_llm_tags(vec!["notif/github".to_string()], &outcome);
        assert!(tags.contains(&"notif/github".to_string()));
        assert!(tags.contains(&"cat/work".to_string()));
        assert!(tags.contains(&"needs-reply".to_string()));
        assert!(tags.contains(&"priority-high".to_string()));
    }

    #[test]
    fn merge_llm_tags_uses_llm_error_on_failure() {
        let tags = merge_llm_tags(Vec::new(), &ClassifyOutcome::Failed);
        assert_eq!(tags, ["llm_error"]);
    }

    #[test]
    fn compile_filters_builds_one_compiled_filter_per_rule() {
        let rules = [
            filter_rule("notif/github", Some(&["github.com"]), None, false, None),
            filter_rule("cat/finance", None, Some("(?i)facture"), false, None),
            filter_rule("newsletter", None, None, true, None),
            filter_rule("spam", None, None, false, Some(("X-Spam-Flag", "(?i)yes"))),
        ];
        let compiled = compile_filters(&rules).unwrap();
        assert_eq!(compiled.len(), 4);
        assert_eq!(compiled[0].tag, "notif/github");
        assert_eq!(compiled[3].tag, "spam");
    }

    #[test]
    fn compile_filters_rejects_an_invalid_regex() {
        let rules = [filter_rule("bad", None, Some("[unclosed"), false, None)];
        assert!(compile_filters(&rules).is_err());
    }

    #[test]
    fn run_filters_emits_tags_for_matching_filters() {
        let filters = vec![
            CompiledFilter {
                filter: NativeFilter::sender_in(vec!["github.com".to_string()]),
                tag: "notif/github".to_string(),
            },
            CompiledFilter {
                filter: NativeFilter::subject_regex("(?i)facture").unwrap(),
                tag: "cat/finance".to_string(),
            },
        ];
        let message = MessageView {
            from: "noreply@github.com".to_string(),
            subject: "Bonjour".to_string(),
            headers: Vec::new(),
        };
        assert_eq!(run_filters(&filters, &message), ["notif/github"]);
    }

    #[test]
    fn run_filters_deduplicates_repeated_tags() {
        let filters = vec![
            CompiledFilter {
                filter: NativeFilter::sender_in(vec!["github.com".to_string()]),
                tag: "notif".to_string(),
            },
            CompiledFilter {
                filter: NativeFilter::subject_regex("(?i)build").unwrap(),
                tag: "notif".to_string(),
            },
        ];
        let message = MessageView {
            from: "ci@github.com".to_string(),
            subject: "Build passed".to_string(),
            headers: Vec::new(),
        };
        assert_eq!(run_filters(&filters, &message), ["notif"]);
    }
}
