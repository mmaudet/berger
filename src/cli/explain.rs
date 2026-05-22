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

//! The `explain <message-id>` command: reconstructs the full decision
//! chain of one processed message from the SQLite sidecar (PRD §5.9) —
//! its tags, the filters and LLM decision that produced them, the IMAP
//! actions applied, and the webhooks emitted.

use anyhow::Context;
use rusqlite::{Connection, OptionalExtension};

use crate::cli::db::open_readonly;

/// The `processed_messages` row for the message under inspection.
#[derive(Debug, Clone)]
pub struct MessageRow {
    /// RFC 822 Message-ID.
    pub message_id: String,
    pub subject: Option<String>,
    pub from_email: Option<String>,
    pub from_name: Option<String>,
    /// When Berger triaged the message.
    pub processed_at: String,
    /// Berger version that triaged it.
    pub berger_version: String,
    /// Pointer to the message's Bichon copy.
    pub bichon_uri: Option<String>,
}

/// One `filter_matches` row — a filter that fired and contributed a tag.
#[derive(Debug, Clone)]
pub struct FilterMatchRow {
    /// `list_unsubscribe` | `sender_in` | `llm` | …
    pub filter_type: String,
    /// The filter's name in the YAML.
    pub filter_name: String,
    /// What matched precisely, as JSON.
    pub details_json: Option<String>,
}

/// One `llm_decisions` row — an LLM classification of the message.
#[derive(Debug, Clone)]
pub struct LlmDecisionRow {
    pub model: String,
    /// The full prompt sent to the model.
    pub prompt_text: String,
    /// The raw JSON the model returned.
    pub response_json: String,
    pub tokens_input: Option<i64>,
    pub tokens_output: Option<i64>,
    pub latency_ms: Option<i64>,
    pub cost_usd: Option<f64>,
    pub called_at: String,
}

/// One `executed_actions` row — an IMAP action Berger applied.
#[derive(Debug, Clone)]
pub struct ActionRow {
    /// `copy_to` | `move_to` | `mark_seen` | `mark_flagged`.
    pub action_type: String,
    /// Destination folder, when the action has one.
    pub target: Option<String>,
    pub succeeded: bool,
    pub error: Option<String>,
    pub executed_at: String,
}

/// One `webhook_emissions` row — a webhook Berger POSTed for the message.
#[derive(Debug, Clone)]
pub struct WebhookRow {
    pub webhook_name: String,
    pub http_status: Option<i64>,
    pub attempts: i64,
    pub succeeded: bool,
    pub emitted_at: String,
}

/// The full reconstructed decision chain for one message.
#[derive(Debug, Clone)]
pub struct MessageExplanation {
    /// The `processed_messages` row.
    pub message: MessageRow,
    /// Applied tags, sorted.
    pub tags: Vec<String>,
    /// The filters that fired.
    pub filter_matches: Vec<FilterMatchRow>,
    /// The LLM classification decisions.
    pub llm_decisions: Vec<LlmDecisionRow>,
    /// The IMAP actions applied.
    pub executed_actions: Vec<ActionRow>,
    /// The webhooks emitted.
    pub webhook_emissions: Vec<WebhookRow>,
}

/// Reconstructs the decision chain for the message with `message_id`, or
/// `None` when no such message has been processed.
///
/// Every sidecar table is queried defensively: a message triaged on native
/// filters alone simply yields empty LLM and webhook sections.
///
/// # Errors
/// Returns [`rusqlite::Error`] on any SQLite failure.
pub fn explain(
    conn: &Connection,
    message_id: &str,
) -> Result<Option<MessageExplanation>, rusqlite::Error> {
    let Some(message) = load_message(conn, message_id)? else {
        return Ok(None);
    };
    Ok(Some(MessageExplanation {
        message,
        tags: load_tags(conn, message_id)?,
        filter_matches: load_filter_matches(conn, message_id)?,
        llm_decisions: load_llm_decisions(conn, message_id)?,
        executed_actions: load_executed_actions(conn, message_id)?,
        webhook_emissions: load_webhook_emissions(conn, message_id)?,
    }))
}

/// Loads the `processed_messages` row, or `None` if absent.
fn load_message(
    conn: &Connection,
    message_id: &str,
) -> Result<Option<MessageRow>, rusqlite::Error> {
    conn.query_row(
        "SELECT message_id, subject, from_email, from_name, processed_at, berger_version, bichon_uri \
         FROM processed_messages WHERE message_id = ?1",
        rusqlite::params![message_id],
        |row| {
            Ok(MessageRow {
                message_id: row.get(0)?,
                subject: row.get(1)?,
                from_email: row.get(2)?,
                from_name: row.get(3)?,
                processed_at: row.get(4)?,
                berger_version: row.get(5)?,
                bichon_uri: row.get(6)?,
            })
        },
    )
    .optional()
}

/// Loads the applied tags, sorted alphabetically.
fn load_tags(conn: &Connection, message_id: &str) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt =
        conn.prepare("SELECT tag FROM applied_tags WHERE message_id = ?1 ORDER BY tag")?;
    let rows = stmt.query_map(rusqlite::params![message_id], |row| row.get(0))?;
    rows.collect()
}

/// Loads the `filter_matches` rows in insertion order.
fn load_filter_matches(
    conn: &Connection,
    message_id: &str,
) -> Result<Vec<FilterMatchRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT filter_type, filter_name, details_json FROM filter_matches \
         WHERE message_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map(rusqlite::params![message_id], |row| {
        Ok(FilterMatchRow {
            filter_type: row.get(0)?,
            filter_name: row.get(1)?,
            details_json: row.get(2)?,
        })
    })?;
    rows.collect()
}

/// Loads the `llm_decisions` rows in call order.
fn load_llm_decisions(
    conn: &Connection,
    message_id: &str,
) -> Result<Vec<LlmDecisionRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT model, prompt_text, response_json, tokens_input, tokens_output, latency_ms, cost_usd, called_at \
         FROM llm_decisions WHERE message_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map(rusqlite::params![message_id], |row| {
        Ok(LlmDecisionRow {
            model: row.get(0)?,
            prompt_text: row.get(1)?,
            response_json: row.get(2)?,
            tokens_input: row.get(3)?,
            tokens_output: row.get(4)?,
            latency_ms: row.get(5)?,
            cost_usd: row.get(6)?,
            called_at: row.get(7)?,
        })
    })?;
    rows.collect()
}

/// Loads the `executed_actions` rows in execution order.
fn load_executed_actions(
    conn: &Connection,
    message_id: &str,
) -> Result<Vec<ActionRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT action_type, target, succeeded, error, executed_at FROM executed_actions \
         WHERE message_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map(rusqlite::params![message_id], |row| {
        Ok(ActionRow {
            action_type: row.get(0)?,
            target: row.get(1)?,
            succeeded: row.get(2)?,
            error: row.get(3)?,
            executed_at: row.get(4)?,
        })
    })?;
    rows.collect()
}

/// Loads the `webhook_emissions` rows in emission order.
fn load_webhook_emissions(
    conn: &Connection,
    message_id: &str,
) -> Result<Vec<WebhookRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT webhook_name, http_status, attempts, succeeded, emitted_at FROM webhook_emissions \
         WHERE message_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map(rusqlite::params![message_id], |row| {
        Ok(WebhookRow {
            webhook_name: row.get(0)?,
            http_status: row.get(1)?,
            attempts: row.get(2)?,
            succeeded: row.get(3)?,
            emitted_at: row.get(4)?,
        })
    })?;
    rows.collect()
}

/// Renders a [`MessageExplanation`] as a human-readable, sectioned report.
pub fn render(explanation: &MessageExplanation) -> String {
    let message = &explanation.message;
    let mut out = String::new();

    out.push_str(&format!("Message  {}\n", message.message_id));
    if let Some(subject) = &message.subject {
        out.push_str(&format!("Subject  {subject}\n"));
    }
    out.push_str(&format!("From     {}\n", render_sender(message)));
    out.push_str(&format!(
        "Processed {}  (berger {})\n",
        message.processed_at, message.berger_version
    ));
    if let Some(uri) = &message.bichon_uri {
        out.push_str(&format!("Bichon   {uri}\n"));
    }

    out.push_str("\nTags\n");
    if explanation.tags.is_empty() {
        out.push_str("  (none)\n");
    } else {
        for tag in &explanation.tags {
            out.push_str(&format!("  - {tag}\n"));
        }
    }

    out.push_str("\nFilters matched\n");
    if explanation.filter_matches.is_empty() {
        out.push_str("  (none)\n");
    } else {
        for filter in &explanation.filter_matches {
            out.push_str(&format!(
                "  - {} `{}`",
                filter.filter_type, filter.filter_name
            ));
            if let Some(details) = &filter.details_json {
                out.push_str(&format!("  {details}"));
            }
            out.push('\n');
        }
    }

    out.push_str("\nLLM decisions\n");
    if explanation.llm_decisions.is_empty() {
        out.push_str("  (none)\n");
    } else {
        for decision in &explanation.llm_decisions {
            out.push_str(&format!(
                "  - model {}  at {}\n",
                decision.model, decision.called_at
            ));
            out.push_str(&format!("    response: {}\n", decision.response_json));
            out.push_str(&format!(
                "    tokens in/out: {} / {}   latency: {}   cost: {}\n",
                opt_i64(decision.tokens_input),
                opt_i64(decision.tokens_output),
                decision
                    .latency_ms
                    .map_or_else(|| "-".to_string(), |ms| format!("{ms}ms")),
                decision
                    .cost_usd
                    .map_or_else(|| "-".to_string(), |usd| format!("${usd:.6}")),
            ));
            out.push_str(&format!("    prompt: {}\n", decision.prompt_text));
        }
    }

    out.push_str("\nActions applied\n");
    if explanation.executed_actions.is_empty() {
        out.push_str("  (none)\n");
    } else {
        for action in &explanation.executed_actions {
            let status = if action.succeeded { "ok" } else { "FAILED" };
            out.push_str(&format!("  - [{status}] {}", action.action_type));
            if let Some(target) = &action.target {
                out.push_str(&format!(" -> {target}"));
            }
            out.push_str(&format!("  at {}", action.executed_at));
            if let Some(error) = &action.error {
                out.push_str(&format!("  error: {error}"));
            }
            out.push('\n');
        }
    }

    out.push_str("\nWebhooks emitted\n");
    if explanation.webhook_emissions.is_empty() {
        out.push_str("  (none)\n");
    } else {
        for webhook in &explanation.webhook_emissions {
            let status = if webhook.succeeded { "ok" } else { "FAILED" };
            out.push_str(&format!(
                "  - [{status}] {}  http {}  attempts {}  at {}\n",
                webhook.webhook_name,
                opt_i64(webhook.http_status),
                webhook.attempts,
                webhook.emitted_at,
            ));
        }
    }

    out
}

/// Formats the sender as `Name <email>`, falling back gracefully when
/// either part is missing.
fn render_sender(message: &MessageRow) -> String {
    match (&message.from_name, &message.from_email) {
        (Some(name), Some(email)) => format!("{name} <{email}>"),
        (None, Some(email)) => email.clone(),
        (Some(name), None) => name.clone(),
        (None, None) => "(unknown)".to_string(),
    }
}

/// Formats an optional integer, showing `-` when absent.
fn opt_i64(value: Option<i64>) -> String {
    value.map_or_else(|| "-".to_string(), |n| n.to_string())
}

/// Loads the configuration to locate the sidecar, then prints the decision
/// chain for `message_id`.
///
/// # Errors
/// Returns an error if the configuration cannot be loaded, or the database
/// cannot be opened or queried.
pub fn run(config_path: &str, message_id: &str) -> anyhow::Result<()> {
    let database_path = crate::cli::db::database_path(config_path)?;
    let conn = open_readonly(&database_path)?;
    match explain(&conn, message_id).context("reconstructing the decision chain")? {
        Some(explanation) => print!("{}", render(&explanation)),
        None => println!("No processed message with Message-ID `{message_id}` in the sidecar."),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message_row() -> MessageRow {
        MessageRow {
            message_id: "<m@berger.test>".to_string(),
            subject: Some("Subject line".to_string()),
            from_email: Some("a@example.test".to_string()),
            from_name: Some("Sender".to_string()),
            processed_at: "2026-05-20T10:00:00Z".to_string(),
            berger_version: "0.0.1".to_string(),
            bichon_uri: None,
        }
    }

    fn empty_explanation() -> MessageExplanation {
        MessageExplanation {
            message: message_row(),
            tags: Vec::new(),
            filter_matches: Vec::new(),
            llm_decisions: Vec::new(),
            executed_actions: Vec::new(),
            webhook_emissions: Vec::new(),
        }
    }

    #[test]
    fn render_sender_combines_name_and_email() {
        assert_eq!(render_sender(&message_row()), "Sender <a@example.test>");
    }

    #[test]
    fn render_sender_falls_back_to_email_only() {
        let mut row = message_row();
        row.from_name = None;
        assert_eq!(render_sender(&row), "a@example.test");
    }

    #[test]
    fn render_sender_handles_a_fully_unknown_sender() {
        let mut row = message_row();
        row.from_name = None;
        row.from_email = None;
        assert_eq!(render_sender(&row), "(unknown)");
    }

    #[test]
    fn opt_i64_shows_a_dash_for_none() {
        assert_eq!(opt_i64(None), "-");
        assert_eq!(opt_i64(Some(42)), "42");
    }

    #[test]
    fn render_marks_empty_sections_as_none() {
        let text = render(&empty_explanation());
        // Every section header is present, each with a "(none)" placeholder.
        assert!(text.contains("Tags\n  (none)"));
        assert!(text.contains("Filters matched\n  (none)"));
        assert!(text.contains("LLM decisions\n  (none)"));
        assert!(text.contains("Actions applied\n  (none)"));
        assert!(text.contains("Webhooks emitted\n  (none)"));
    }

    #[test]
    fn render_marks_a_failed_action() {
        let mut explanation = empty_explanation();
        explanation.executed_actions.push(ActionRow {
            action_type: "copy_to".to_string(),
            target: Some("Berger/work".to_string()),
            succeeded: false,
            error: Some("mailbox is read-only".to_string()),
            executed_at: "2026-05-20T10:00:01Z".to_string(),
        });
        let text = render(&explanation);
        assert!(text.contains("[FAILED] copy_to"));
        assert!(text.contains("mailbox is read-only"));
    }
}
