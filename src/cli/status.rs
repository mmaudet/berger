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

//! The `status` command: a health and statistics summary of the SQLite
//! sidecar (PRD §5.8, §5.9) — table counts, recent activity, cumulative
//! LLM cost, and the IMAP-action and webhook success rates.

use anyhow::Context;
use rusqlite::Connection;

use crate::cli::db::open_readonly;

/// A point-in-time snapshot of the sidecar's counters.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusSummary {
    /// Rows in `accounts`.
    pub accounts: i64,
    /// Rows in `processed_messages` — every message Berger has triaged.
    pub processed_messages: i64,
    /// Rows in `applied_tags`.
    pub applied_tags: i64,
    /// Rows in `filter_matches`.
    pub filter_matches: i64,
    /// Rows in `llm_decisions`.
    pub llm_decisions: i64,
    /// Rows in `executed_actions`.
    pub executed_actions: i64,
    /// Rows in `webhook_emissions`.
    pub webhook_emissions: i64,
    /// Messages processed in the last 24 hours.
    pub processed_last_24h: i64,
    /// Messages processed in the last 7 days.
    pub processed_last_7d: i64,
    /// Cumulative LLM spend in US dollars.
    pub llm_cost_usd: f64,
    /// Cumulative prompt tokens billed.
    pub llm_tokens_input: i64,
    /// Cumulative completion tokens billed.
    pub llm_tokens_output: i64,
    /// IMAP actions that succeeded.
    pub actions_succeeded: i64,
    /// IMAP actions that failed.
    pub actions_failed: i64,
    /// Webhooks that were delivered.
    pub webhooks_succeeded: i64,
    /// Webhooks that were abandoned after their retries.
    pub webhooks_failed: i64,
}

/// Collects the sidecar's counters into a [`StatusSummary`].
///
/// Every aggregate is defensive: an empty table contributes zero, never
/// an error.
///
/// # Errors
/// Returns [`rusqlite::Error`] on any SQLite failure.
pub fn collect(conn: &Connection) -> Result<StatusSummary, rusqlite::Error> {
    Ok(StatusSummary {
        accounts: count(conn, "accounts")?,
        processed_messages: count(conn, "processed_messages")?,
        applied_tags: count(conn, "applied_tags")?,
        filter_matches: count(conn, "filter_matches")?,
        llm_decisions: count(conn, "llm_decisions")?,
        executed_actions: count(conn, "executed_actions")?,
        webhook_emissions: count(conn, "webhook_emissions")?,
        processed_last_24h: processed_since(conn, "-1 day")?,
        processed_last_7d: processed_since(conn, "-7 days")?,
        llm_cost_usd: scalar_f64(
            conn,
            "SELECT COALESCE(SUM(cost_usd), 0.0) FROM llm_decisions",
        )?,
        llm_tokens_input: scalar_i64(
            conn,
            "SELECT COALESCE(SUM(tokens_input), 0) FROM llm_decisions",
        )?,
        llm_tokens_output: scalar_i64(
            conn,
            "SELECT COALESCE(SUM(tokens_output), 0) FROM llm_decisions",
        )?,
        actions_succeeded: scalar_i64(
            conn,
            "SELECT COUNT(*) FROM executed_actions WHERE succeeded = 1",
        )?,
        actions_failed: scalar_i64(
            conn,
            "SELECT COUNT(*) FROM executed_actions WHERE succeeded = 0",
        )?,
        webhooks_succeeded: scalar_i64(
            conn,
            "SELECT COUNT(*) FROM webhook_emissions WHERE succeeded = 1",
        )?,
        webhooks_failed: scalar_i64(
            conn,
            "SELECT COUNT(*) FROM webhook_emissions WHERE succeeded = 0",
        )?,
    })
}

/// Counts every row in `table`. The table name is a fixed string literal
/// passed by [`collect`], never user input, so it is safe to interpolate.
fn count(conn: &Connection, table: &str) -> Result<i64, rusqlite::Error> {
    scalar_i64(conn, &format!("SELECT COUNT(*) FROM {table}"))
}

/// Counts the messages whose `processed_at` falls within `interval` of now
/// (an SQLite modifier such as `-1 day`).
///
/// `datetime()` is applied to both sides so the comparison is robust to the
/// two timestamp spellings the schema can hold — `CURRENT_TIMESTAMP`'s
/// space form and an ISO-8601 `T…Z` form.
fn processed_since(conn: &Connection, interval: &str) -> Result<i64, rusqlite::Error> {
    conn.query_row(
        "SELECT COUNT(*) FROM processed_messages \
         WHERE datetime(processed_at) >= datetime('now', ?1)",
        rusqlite::params![interval],
        |row| row.get(0),
    )
}

/// Runs a single-column, single-row query returning an integer.
fn scalar_i64(conn: &Connection, sql: &str) -> Result<i64, rusqlite::Error> {
    conn.query_row(sql, [], |row| row.get(0))
}

/// Runs a single-column, single-row query returning a float.
fn scalar_f64(conn: &Connection, sql: &str) -> Result<f64, rusqlite::Error> {
    conn.query_row(sql, [], |row| row.get(0))
}

/// Renders a [`StatusSummary`] as a human-readable report.
pub fn render(summary: &StatusSummary) -> String {
    let mut out = String::from("Berger status\n");

    out.push_str("\nMessages processed\n");
    out.push_str(&format!("  total      {}\n", summary.processed_messages));
    out.push_str(&format!("  last 24h   {}\n", summary.processed_last_24h));
    out.push_str(&format!("  last 7d    {}\n", summary.processed_last_7d));

    out.push_str("\nLLM\n");
    out.push_str(&format!("  decisions  {}\n", summary.llm_decisions));
    out.push_str(&format!(
        "  tokens     {} in / {} out\n",
        summary.llm_tokens_input, summary.llm_tokens_output
    ));
    out.push_str(&format!("  cost       ${:.6}\n", summary.llm_cost_usd));

    out.push_str("\nIMAP actions\n");
    out.push_str(&format!("  succeeded  {}\n", summary.actions_succeeded));
    out.push_str(&format!("  failed     {}\n", summary.actions_failed));

    out.push_str("\nWebhooks\n");
    out.push_str(&format!("  succeeded  {}\n", summary.webhooks_succeeded));
    out.push_str(&format!("  failed     {}\n", summary.webhooks_failed));

    out.push_str("\nSidecar tables\n");
    out.push_str(&format!("  accounts            {}\n", summary.accounts));
    out.push_str(&format!(
        "  processed_messages  {}\n",
        summary.processed_messages
    ));
    out.push_str(&format!("  applied_tags        {}\n", summary.applied_tags));
    out.push_str(&format!(
        "  filter_matches      {}\n",
        summary.filter_matches
    ));
    out.push_str(&format!(
        "  llm_decisions       {}\n",
        summary.llm_decisions
    ));
    out.push_str(&format!(
        "  executed_actions    {}\n",
        summary.executed_actions
    ));
    out.push_str(&format!(
        "  webhook_emissions   {}\n",
        summary.webhook_emissions
    ));

    out
}

/// Loads the configuration to locate the sidecar, then prints its status
/// summary.
///
/// # Errors
/// Returns an error if the configuration cannot be loaded, or the database
/// cannot be opened or queried.
pub fn run(config_path: &str) -> anyhow::Result<()> {
    let database_path = crate::cli::db::database_path(config_path)?;
    let conn = open_readonly(&database_path)?;
    let summary = collect(&conn).context("collecting sidecar statistics")?;
    print!("{}", render(&summary));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::database::Database;

    fn empty_summary() -> StatusSummary {
        collect(Database::open(":memory:").unwrap().connection()).unwrap()
    }

    #[test]
    fn collect_on_a_fresh_database_is_all_zero() {
        let summary = empty_summary();
        assert_eq!(summary.accounts, 0);
        assert_eq!(summary.processed_messages, 0);
        assert_eq!(summary.processed_last_24h, 0);
        assert_eq!(summary.llm_decisions, 0);
        assert!((summary.llm_cost_usd - 0.0).abs() < 1e-9);
    }

    #[test]
    fn count_reports_inserted_rows() {
        let db = Database::open(":memory:").unwrap();
        let conn = db.connection();
        conn.execute(
            "INSERT INTO accounts (name, bichon_account_id) VALUES ('a', 'b')",
            [],
        )
        .unwrap();
        assert_eq!(count(conn, "accounts").unwrap(), 1);
    }

    #[test]
    fn scalar_f64_reads_a_float() {
        let db = Database::open(":memory:").unwrap();
        assert!((scalar_f64(db.connection(), "SELECT 1.5").unwrap() - 1.5).abs() < 1e-9);
    }

    #[test]
    fn render_lists_every_sidecar_table() {
        let text = render(&empty_summary());
        for table in [
            "accounts",
            "processed_messages",
            "applied_tags",
            "filter_matches",
            "llm_decisions",
            "executed_actions",
            "webhook_emissions",
        ] {
            assert!(text.contains(table), "status report should mention {table}");
        }
    }

    #[test]
    fn render_shows_the_time_windows() {
        let text = render(&empty_summary());
        assert!(text.contains("last 24h"));
        assert!(text.contains("last 7d"));
    }
}
