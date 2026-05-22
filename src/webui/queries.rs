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

//! Read-only SQL queries against the sidecar, feeding the four WebUI pages.
//!
//! Every query reads tables documented in `migrations/V1__initial_schema.sql`
//! through [`Database::connection`](crate::storage::database::Database::connection).
//! Tables other than `processed_messages` and `llm_decisions` may be empty
//! depending on which milestones have been wired in; the queries below
//! tolerate that and simply return zeros or empty vectors.

use rusqlite::{Connection, OptionalExtension};

/// The headline counters shown on the `/` page (PRD §5.7).
#[derive(Debug, Clone, PartialEq)]
pub struct DashboardStats {
    /// Messages processed in the last 24 hours.
    pub processed_24h: i64,
    /// Messages processed in the last 7 days.
    pub processed_7d: i64,
    /// Messages processed in total.
    pub processed_total: i64,
    /// LLM prompt tokens billed in total.
    pub llm_tokens_input: i64,
    /// LLM completion tokens billed in total.
    pub llm_tokens_output: i64,
    /// Estimated cumulative LLM cost, in US dollars.
    pub llm_cost_usd: f64,
    /// Number of LLM cache lookups that were hits.
    pub llm_cache_hits: i64,
    /// Number of LLM cache lookups in total (hits plus misses).
    pub llm_cache_lookups: i64,
    /// Webhooks emitted successfully.
    pub webhooks_succeeded: i64,
    /// Webhooks that exhausted their retries without success.
    pub webhooks_failed: i64,
}

impl DashboardStats {
    /// The LLM cache hit rate as a percentage in `0.0..=100.0`, or `0.0`
    /// when no lookup has been recorded yet.
    pub fn cache_hit_rate_pct(&self) -> f64 {
        if self.llm_cache_lookups == 0 {
            0.0
        } else {
            #[allow(clippy::cast_precision_loss)]
            let rate = self.llm_cache_hits as f64 / self.llm_cache_lookups as f64;
            rate * 100.0
        }
    }
}

/// One row of the `/recent` table — a triaged message with its tags and
/// the actions Berger applied to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentMessage {
    /// RFC 822 Message-ID — the key used by the `/explain/<id>` link.
    pub message_id: String,
    /// The `Subject:` header, when known.
    pub subject: Option<String>,
    /// The sender display name, when known.
    pub from_name: Option<String>,
    /// The sender address, when known.
    pub from_email: Option<String>,
    /// When Berger processed the message (SQLite `TIMESTAMP` text, UTC).
    pub processed_at: String,
    /// The tags applied to the message, in `applied_tags` order.
    pub tags: Vec<String>,
    /// Short labels of the IMAP actions applied (`copy_to → …`, `move_to`).
    pub actions: Vec<String>,
}

/// The full triage of one message, shown on `/explain/<id>` (PRD §5.7).
#[derive(Debug, Clone, PartialEq)]
pub struct MessageExplanation {
    /// RFC 822 Message-ID.
    pub message_id: String,
    /// The `Subject:` header, when known.
    pub subject: Option<String>,
    /// The sender display name, when known.
    pub from_name: Option<String>,
    /// The sender address, when known.
    pub from_email: Option<String>,
    /// When Berger processed the message.
    pub processed_at: String,
    /// The Berger version that processed the message.
    pub berger_version: String,
    /// The tags applied to the message.
    pub tags: Vec<String>,
    /// The native filters and the LLM that produced those tags.
    pub filter_matches: Vec<FilterMatchRow>,
    /// The LLM decisions recorded for the message, newest first.
    pub llm_decisions: Vec<LlmDecisionRow>,
    /// The IMAP actions applied to the message.
    pub actions: Vec<ActionRow>,
    /// The webhooks emitted for the message.
    pub webhooks: Vec<WebhookRow>,
}

/// One row of the `filter_matches` table — why a tag was applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterMatchRow {
    /// The filter family, e.g. `sender_in`, `list_unsubscribe`, `llm`.
    pub filter_type: String,
    /// The filter's name in the YAML configuration.
    pub filter_name: String,
    /// What matched precisely, as a JSON string, when recorded.
    pub details_json: Option<String>,
}

/// One row of the `llm_decisions` table — an LLM call's audit record.
#[derive(Debug, Clone, PartialEq)]
pub struct LlmDecisionRow {
    /// The model that produced the decision.
    pub model: String,
    /// The full prompt sent to the model.
    pub prompt_text: String,
    /// The raw JSON the model returned.
    pub response_json: String,
    /// Prompt tokens billed, when reported.
    pub tokens_input: Option<i64>,
    /// Completion tokens billed, when reported.
    pub tokens_output: Option<i64>,
    /// Round-trip latency, in milliseconds, when recorded.
    pub latency_ms: Option<i64>,
    /// Estimated cost, in US dollars, when known.
    pub cost_usd: Option<f64>,
    /// When the model was called.
    pub called_at: String,
}

/// One row of the `executed_actions` table — an IMAP action Berger ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionRow {
    /// The action, e.g. `copy_to`, `move_to`, `mark_seen`.
    pub action_type: String,
    /// The destination folder, for `copy_to` / `move_to`.
    pub target: Option<String>,
    /// Whether the action succeeded.
    pub succeeded: bool,
    /// The failure message, when the action failed.
    pub error: Option<String>,
    /// When the action ran.
    pub executed_at: String,
}

/// One row of the `webhook_emissions` table — a webhook Berger POSTed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookRow {
    /// The webhook's name in the YAML configuration.
    pub webhook_name: String,
    /// The final HTTP status code, when a response was received.
    pub http_status: Option<i64>,
    /// How many POST attempts were made.
    pub attempts: i64,
    /// Whether the webhook ultimately succeeded.
    pub succeeded: bool,
    /// When the webhook was first emitted.
    pub emitted_at: String,
}

/// Reads the headline counters for the `/` dashboard.
///
/// # Errors
/// Returns a [`rusqlite::Error`] on a SQLite failure.
pub fn dashboard_stats(conn: &Connection) -> Result<DashboardStats, rusqlite::Error> {
    // processed_at is a CURRENT_TIMESTAMP text column ('YYYY-MM-DD HH:MM:SS'
    // UTC); SQLite compares those windows lexicographically against datetime().
    let processed_24h = conn.query_row(
        "SELECT COUNT(*) FROM processed_messages \
         WHERE processed_at >= datetime('now', '-1 day')",
        [],
        |row| row.get(0),
    )?;
    let processed_7d = conn.query_row(
        "SELECT COUNT(*) FROM processed_messages \
         WHERE processed_at >= datetime('now', '-7 days')",
        [],
        |row| row.get(0),
    )?;
    let processed_total = conn.query_row("SELECT COUNT(*) FROM processed_messages", [], |row| {
        row.get(0)
    })?;

    let (llm_tokens_input, llm_tokens_output, llm_cost_usd, llm_cache_lookups): (
        i64,
        i64,
        f64,
        i64,
    ) = conn.query_row(
        "SELECT COALESCE(SUM(tokens_input), 0), COALESCE(SUM(tokens_output), 0), \
                COALESCE(SUM(cost_usd), 0.0), COUNT(*) \
         FROM llm_decisions",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    // A cache hit reuses an existing (message_id, prompt_hash) row: the same
    // pair therefore appears more than once. Hits = total rows minus the
    // count of distinct pairs.
    let distinct_prompts: i64 = conn.query_row(
        "SELECT COUNT(*) FROM (SELECT DISTINCT message_id, prompt_hash FROM llm_decisions)",
        [],
        |row| row.get(0),
    )?;
    let llm_cache_hits = (llm_cache_lookups - distinct_prompts).max(0);

    let (webhooks_succeeded, webhooks_failed): (i64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(succeeded), 0), COALESCE(SUM(NOT succeeded), 0) \
         FROM webhook_emissions",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    Ok(DashboardStats {
        processed_24h,
        processed_7d,
        processed_total,
        llm_tokens_input,
        llm_tokens_output,
        llm_cost_usd,
        llm_cache_hits,
        llm_cache_lookups,
        webhooks_succeeded,
        webhooks_failed,
    })
}

/// Reads the most recently triaged messages, newest first, for `/recent`.
/// `limit` caps the number of rows (PRD §5.7 asks for the last 50).
///
/// # Errors
/// Returns a [`rusqlite::Error`] on a SQLite failure.
pub fn recent_messages(
    conn: &Connection,
    limit: usize,
) -> Result<Vec<RecentMessage>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT message_id, subject, from_name, from_email, processed_at \
         FROM processed_messages \
         ORDER BY processed_at DESC, rowid DESC \
         LIMIT ?1",
    )?;
    let rows = stmt.query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |row| {
        Ok(RecentMessage {
            message_id: row.get(0)?,
            subject: row.get(1)?,
            from_name: row.get(2)?,
            from_email: row.get(3)?,
            processed_at: row.get(4)?,
            tags: Vec::new(),
            actions: Vec::new(),
        })
    })?;
    let mut messages: Vec<RecentMessage> = rows.collect::<Result<_, _>>()?;
    for message in &mut messages {
        message.tags = tags_for(conn, &message.message_id)?;
        message.actions = action_labels_for(conn, &message.message_id)?;
    }
    Ok(messages)
}

/// Reconstructs the full triage of one message for `/explain/<id>`.
///
/// # Errors
/// Returns a [`rusqlite::Error`] on a SQLite failure. Returns `Ok(None)`
/// when no message with `message_id` is recorded in the sidecar.
pub fn message_explanation(
    conn: &Connection,
    message_id: &str,
) -> Result<Option<MessageExplanation>, rusqlite::Error> {
    let header = conn
        .query_row(
            "SELECT subject, from_name, from_email, processed_at, berger_version \
             FROM processed_messages WHERE message_id = ?1",
            [message_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((subject, from_name, from_email, processed_at, berger_version)) = header else {
        return Ok(None);
    };

    Ok(Some(MessageExplanation {
        message_id: message_id.to_string(),
        subject,
        from_name,
        from_email,
        processed_at,
        berger_version,
        tags: tags_for(conn, message_id)?,
        filter_matches: filter_matches_for(conn, message_id)?,
        llm_decisions: llm_decisions_for(conn, message_id)?,
        actions: actions_for(conn, message_id)?,
        webhooks: webhooks_for(conn, message_id)?,
    }))
}

/// The tags applied to a message, in insertion (`rowid`) order.
fn tags_for(conn: &Connection, message_id: &str) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = conn
        .prepare("SELECT tag FROM applied_tags WHERE message_id = ?1 ORDER BY applied_at, rowid")?;
    let rows = stmt.query_map([message_id], |row| row.get(0))?;
    rows.collect()
}

/// Short, human-readable labels for the IMAP actions applied to a message.
fn action_labels_for(conn: &Connection, message_id: &str) -> Result<Vec<String>, rusqlite::Error> {
    Ok(actions_for(conn, message_id)?
        .into_iter()
        .map(|action| match action.target {
            Some(target) => format!("{} → {target}", action.action_type),
            None => action.action_type,
        })
        .collect())
}

/// The `filter_matches` rows for a message, in insertion order.
fn filter_matches_for(
    conn: &Connection,
    message_id: &str,
) -> Result<Vec<FilterMatchRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT filter_type, filter_name, details_json \
         FROM filter_matches WHERE message_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map([message_id], |row| {
        Ok(FilterMatchRow {
            filter_type: row.get(0)?,
            filter_name: row.get(1)?,
            details_json: row.get(2)?,
        })
    })?;
    rows.collect()
}

/// The `llm_decisions` rows for a message, newest call first.
fn llm_decisions_for(
    conn: &Connection,
    message_id: &str,
) -> Result<Vec<LlmDecisionRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT model, prompt_text, response_json, tokens_input, tokens_output, \
                latency_ms, cost_usd, called_at \
         FROM llm_decisions WHERE message_id = ?1 ORDER BY called_at DESC, id DESC",
    )?;
    let rows = stmt.query_map([message_id], |row| {
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

/// The `executed_actions` rows for a message, in execution order.
fn actions_for(conn: &Connection, message_id: &str) -> Result<Vec<ActionRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT action_type, target, succeeded, error, executed_at \
         FROM executed_actions WHERE message_id = ?1 ORDER BY executed_at, id",
    )?;
    let rows = stmt.query_map([message_id], |row| {
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

/// The `webhook_emissions` rows for a message, in emission order.
fn webhooks_for(conn: &Connection, message_id: &str) -> Result<Vec<WebhookRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT webhook_name, http_status, attempts, succeeded, emitted_at \
         FROM webhook_emissions WHERE message_id = ?1 ORDER BY emitted_at, id",
    )?;
    let rows = stmt.query_map([message_id], |row| {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::database::Database;
    use crate::storage::llm_decisions::LlmDecision;
    use crate::storage::processed_messages::ProcessedMessage;

    /// A database with one account and `count` processed messages, all
    /// stamped "now" by `record()`.
    fn db_with_messages(count: usize) -> Database {
        let db = Database::open(":memory:").unwrap();
        let account_id = db.accounts().insert("LINAGORA", "bichon-1").unwrap();
        for index in 0..count {
            db.processed_messages()
                .record(&ProcessedMessage {
                    message_id: format!("<m{index}@test>"),
                    account_id,
                    bichon_uri: None,
                    subject: Some(format!("Subject {index}")),
                    from_email: Some("sender@test".to_string()),
                    from_name: Some("Sender".to_string()),
                    date: None,
                    berger_version: "0.0.1".to_string(),
                    config_hash: "cfg".to_string(),
                })
                .unwrap();
        }
        db
    }

    /// Backdates one already-recorded message by a SQLite time modifier,
    /// e.g. `"-3 hours"` or `"-10 days"`.
    fn backdate(db: &Database, message_id: &str, modifier: &str) {
        db.connection()
            .execute(
                "UPDATE processed_messages \
                 SET processed_at = datetime('now', ?2) WHERE message_id = ?1",
                rusqlite::params![message_id, modifier],
            )
            .unwrap();
    }

    fn record_llm(db: &Database, message_id: &str, prompt_hash: &str) {
        db.llm_decisions()
            .record(&LlmDecision {
                message_id: message_id.to_string(),
                model: "test-model".to_string(),
                prompt_hash: prompt_hash.to_string(),
                prompt_text: "classify this".to_string(),
                response_json: r#"{"category":"work"}"#.to_string(),
                tokens_input: Some(100),
                tokens_output: Some(20),
                latency_ms: Some(150),
                cost_usd: Some(0.001),
            })
            .unwrap();
    }

    #[test]
    fn dashboard_stats_on_an_empty_database_are_all_zero() {
        let db = Database::open(":memory:").unwrap();
        let stats = dashboard_stats(db.connection()).unwrap();
        assert_eq!(stats.processed_24h, 0);
        assert_eq!(stats.processed_7d, 0);
        assert_eq!(stats.processed_total, 0);
        assert_eq!(stats.llm_tokens_input, 0);
        assert_eq!(stats.llm_cost_usd, 0.0);
        assert_eq!(stats.webhooks_succeeded, 0);
        assert_eq!(stats.cache_hit_rate_pct(), 0.0);
    }

    #[test]
    fn dashboard_stats_count_messages_in_their_time_windows() {
        // m0 stays "now"; the others are backdated well clear of the
        // window boundaries so the counts are unambiguous.
        let db = db_with_messages(4);
        backdate(&db, "<m1@test>", "-3 hours"); // inside 24h and 7d
        backdate(&db, "<m2@test>", "-3 days"); // inside 7d, outside 24h
        backdate(&db, "<m3@test>", "-30 days"); // outside both
        let stats = dashboard_stats(db.connection()).unwrap();
        assert_eq!(stats.processed_total, 4);
        // m0 (now) and m1 (3h ago) are within the last 24 hours.
        assert_eq!(stats.processed_24h, 2);
        // m0, m1 and m2 (3d ago) are within the last 7 days; m3 is not.
        assert_eq!(stats.processed_7d, 3);
    }

    #[test]
    fn dashboard_stats_sum_llm_tokens_and_cost() {
        let db = db_with_messages(2);
        record_llm(&db, "<m0@test>", "hash-a");
        record_llm(&db, "<m1@test>", "hash-b");
        let stats = dashboard_stats(db.connection()).unwrap();
        assert_eq!(stats.llm_tokens_input, 200);
        assert_eq!(stats.llm_tokens_output, 40);
        assert!((stats.llm_cost_usd - 0.002).abs() < 1e-9);
    }

    #[test]
    fn cache_hit_rate_counts_repeated_prompt_pairs_as_hits() {
        let db = db_with_messages(1);
        // Same (message_id, prompt_hash) recorded twice: one miss, one hit.
        record_llm(&db, "<m0@test>", "hash-a");
        record_llm(&db, "<m0@test>", "hash-a");
        let stats = dashboard_stats(db.connection()).unwrap();
        assert_eq!(stats.llm_cache_lookups, 2);
        assert_eq!(stats.llm_cache_hits, 1);
        assert!((stats.cache_hit_rate_pct() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn recent_messages_are_returned_newest_first() {
        let db = db_with_messages(3);
        // Backdate the others so m0 is unambiguously the most recent.
        backdate(&db, "<m1@test>", "-1 day");
        backdate(&db, "<m2@test>", "-2 days");
        let recent = recent_messages(db.connection(), 50).unwrap();
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].message_id, "<m0@test>");
        assert_eq!(recent[0].subject.as_deref(), Some("Subject 0"));
        assert_eq!(recent[2].message_id, "<m2@test>");
    }

    #[test]
    fn recent_messages_honours_the_limit() {
        let db = db_with_messages(5);
        let recent = recent_messages(db.connection(), 2).unwrap();
        assert_eq!(recent.len(), 2);
    }

    #[test]
    fn recent_messages_on_an_empty_database_is_empty() {
        let db = Database::open(":memory:").unwrap();
        assert!(recent_messages(db.connection(), 50).unwrap().is_empty());
    }

    #[test]
    fn recent_messages_carries_applied_tags() {
        let db = db_with_messages(1);
        db.connection()
            .execute(
                "INSERT INTO applied_tags (message_id, tag, applied_at) \
                 VALUES ('<m0@test>', 'cat/work', CURRENT_TIMESTAMP)",
                [],
            )
            .unwrap();
        let recent = recent_messages(db.connection(), 50).unwrap();
        assert_eq!(recent[0].tags, ["cat/work"]);
    }

    #[test]
    fn recent_messages_carries_action_labels() {
        let db = db_with_messages(1);
        db.connection()
            .execute(
                "INSERT INTO executed_actions \
                 (message_id, action_type, target, succeeded, executed_at) \
                 VALUES ('<m0@test>', 'copy_to', 'cat/work', 1, CURRENT_TIMESTAMP)",
                [],
            )
            .unwrap();
        let recent = recent_messages(db.connection(), 50).unwrap();
        assert_eq!(recent[0].actions, ["copy_to → cat/work"]);
    }

    #[test]
    fn message_explanation_returns_none_for_an_unknown_message() {
        let db = Database::open(":memory:").unwrap();
        assert!(
            message_explanation(db.connection(), "<ghost@test>")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn message_explanation_reconstructs_the_full_chain() {
        let db = db_with_messages(1);
        let conn = db.connection();
        conn.execute(
            "INSERT INTO applied_tags (message_id, tag, applied_at) \
             VALUES ('<m0@test>', 'cat/work', CURRENT_TIMESTAMP)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO filter_matches (message_id, filter_type, filter_name, details_json) \
             VALUES ('<m0@test>', 'sender_in', 'work-senders', '{\"matched\":\"a@b\"}')",
            [],
        )
        .unwrap();
        record_llm(&db, "<m0@test>", "hash-a");
        conn.execute(
            "INSERT INTO executed_actions \
             (message_id, action_type, target, succeeded, executed_at) \
             VALUES ('<m0@test>', 'copy_to', 'cat/work', 1, CURRENT_TIMESTAMP)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO webhook_emissions \
             (message_id, webhook_name, payload_json, http_status, attempts, succeeded, emitted_at) \
             VALUES ('<m0@test>', 'linatwin-draft', '{}', 200, 1, 1, CURRENT_TIMESTAMP)",
            [],
        )
        .unwrap();

        let explanation = message_explanation(conn, "<m0@test>").unwrap().unwrap();
        assert_eq!(explanation.message_id, "<m0@test>");
        assert_eq!(explanation.subject.as_deref(), Some("Subject 0"));
        assert_eq!(explanation.tags, ["cat/work"]);
        assert_eq!(explanation.filter_matches.len(), 1);
        assert_eq!(explanation.filter_matches[0].filter_type, "sender_in");
        assert_eq!(explanation.llm_decisions.len(), 1);
        assert_eq!(explanation.llm_decisions[0].model, "test-model");
        assert_eq!(explanation.actions.len(), 1);
        assert_eq!(explanation.actions[0].action_type, "copy_to");
        assert!(explanation.actions[0].succeeded);
        assert_eq!(explanation.webhooks.len(), 1);
        assert_eq!(explanation.webhooks[0].webhook_name, "linatwin-draft");
        assert_eq!(explanation.webhooks[0].http_status, Some(200));
    }

    #[test]
    fn message_explanation_of_a_bare_message_has_empty_chain_sections() {
        // A message processed before any later milestone wired its tables.
        let db = db_with_messages(1);
        let explanation = message_explanation(db.connection(), "<m0@test>")
            .unwrap()
            .unwrap();
        assert!(explanation.tags.is_empty());
        assert!(explanation.filter_matches.is_empty());
        assert!(explanation.llm_decisions.is_empty());
        assert!(explanation.actions.is_empty());
        assert!(explanation.webhooks.is_empty());
        assert_eq!(explanation.berger_version, "0.0.1");
    }
}
