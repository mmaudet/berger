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
use crate::pipeline::error::PipelineError;
use crate::storage::database::Database;
use crate::storage::processed_messages::ProcessedMessage;

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

/// The outcome of running one message through the pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessOutcome {
    /// The message had already been processed; nothing was done (rule #2).
    AlreadyProcessed,
    /// The message was triaged.
    Processed {
        /// Tags emitted by the native filters.
        tags: Vec<String>,
        /// Number of IMAP actions applied.
        actions_applied: usize,
    },
}

/// The triage pipeline: the compiled filters and the action map, applied
/// to each polled message.
#[derive(Debug)]
pub struct Pipeline {
    filters: Vec<CompiledFilter>,
    actions: BTreeMap<String, TagActions>,
    config_hash: String,
}

impl Pipeline {
    /// Builds a pipeline from compiled filters, the action map, and a hash
    /// of the configuration in force (recorded with every message).
    pub fn new(
        filters: Vec<CompiledFilter>,
        actions: BTreeMap<String, TagActions>,
        config_hash: String,
    ) -> Self {
        Self {
            filters,
            actions,
            config_hash,
        }
    }

    /// Processes one polled message: skips it if already seen (Bichon
    /// coherence rule #2), otherwise downloads its EML, runs the filters,
    /// applies the resolved actions, and records it in the ledger.
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
        let tags = run_filters(&self.filters, &view);
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

        Ok(ProcessOutcome::Processed {
            actions_applied: actions.len(),
            tags,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HeaderMatchSpec;

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
