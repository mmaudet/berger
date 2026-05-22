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

//! The `dry-run` command (PRD §5.7): runs one poll cycle, applies **no**
//! IMAP actions and records **nothing**, and prints the tags and actions
//! Berger *would* apply.
//!
//! This is a deliberately minimal re-implementation of the poll logic, kept
//! separate from `run` so a dry run can never reach the IMAP writeback or
//! the sidecar's write path. It runs the native filters only — they are
//! free and deterministic; the LLM classifier is an external, billable call
//! and is therefore not invoked, only noted.

use std::collections::BTreeMap;

use anyhow::Context;
use rusqlite::Connection;

use crate::actions::Action;
use crate::actions::resolve::resolve_actions;
use crate::config::{BergerConfig, TagActions};
use crate::ingest::bichon_client::BichonClient;
use crate::ingest::poller::{Watermark, poll_account};
use crate::ingest::source::MessageSource;
use crate::ingest::types::Envelope;
use crate::pipeline::{CompiledFilter, compile_filters, parse_message_view, run_filters};

/// What a dry run would do with one polled message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessagePlan {
    /// The message would be triaged.
    WouldProcess {
        /// RFC 822 Message-ID.
        message_id: String,
        /// The `Subject:` header.
        subject: String,
        /// Tags the native filters would apply.
        tags: Vec<String>,
        /// Human-readable descriptions of the IMAP actions that would run.
        actions: Vec<String>,
    },
    /// The message is already in the sidecar; it would be skipped
    /// (Bichon coherence rule #2).
    WouldSkip {
        /// RFC 822 Message-ID.
        message_id: String,
        /// The `Subject:` header.
        subject: String,
    },
}

/// The dry-run plan for one account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountPlan {
    /// The Bichon account id that was polled.
    pub account_id: u64,
    /// The plan for each message the poll returned.
    pub messages: Vec<MessagePlan>,
}

/// Plans one poll cycle for every account in `account_ids`, without any
/// side effect: it polls the `source`, runs the native filters, resolves
/// the actions, and reports — it never writes to `conn` and never touches
/// IMAP.
///
/// The polling watermark is read from `conn` (so the dry run sees the same
/// window the daemon would) but never advanced or persisted.
///
/// # Errors
/// Returns an error if polling or downloading a message fails, or a
/// sidecar read fails.
pub async fn plan<S: MessageSource>(
    source: &S,
    account_ids: &[u64],
    filters: &[CompiledFilter],
    actions: &BTreeMap<String, TagActions>,
    conn: &Connection,
) -> anyhow::Result<Vec<AccountPlan>> {
    let mut plans = Vec::with_capacity(account_ids.len());
    for &account_id in account_ids {
        plans.push(plan_account(source, account_id, filters, actions, conn).await?);
    }
    Ok(plans)
}

/// Plans one poll cycle for a single account.
async fn plan_account<S: MessageSource>(
    source: &S,
    account_id: u64,
    filters: &[CompiledFilter],
    actions: &BTreeMap<String, TagActions>,
    conn: &Connection,
) -> anyhow::Result<AccountPlan> {
    // Read-only: the watermark is consulted, never advanced or saved.
    let watermark = read_watermark(conn, account_id)?;
    let outcome = poll_account(source, account_id, watermark)
        .await
        .with_context(|| format!("polling Bichon account {account_id}"))?;

    let mut messages = Vec::with_capacity(outcome.envelopes.len());
    for envelope in &outcome.envelopes {
        messages.push(plan_message(source, envelope, filters, actions, conn).await?);
    }
    Ok(AccountPlan {
        account_id,
        messages,
    })
}

/// Plans the triage of one polled message.
async fn plan_message<S: MessageSource>(
    source: &S,
    envelope: &Envelope,
    filters: &[CompiledFilter],
    actions: &BTreeMap<String, TagActions>,
    conn: &Connection,
) -> anyhow::Result<MessagePlan> {
    // Idempotence (rule #2): an already-processed message would be skipped.
    if is_already_processed(conn, &envelope.message_id)? {
        return Ok(MessagePlan::WouldSkip {
            message_id: envelope.message_id.clone(),
            subject: envelope.subject.clone(),
        });
    }

    let eml = source
        .download_message(&envelope.account_id.to_string(), &envelope.id)
        .await
        .with_context(|| format!("downloading message `{}`", envelope.message_id))?;
    let view = parse_message_view(&eml);
    let tags = run_filters(filters, &view);
    let resolved = resolve_actions(&tags, actions);

    Ok(MessagePlan::WouldProcess {
        message_id: envelope.message_id.clone(),
        subject: envelope.subject.clone(),
        tags,
        actions: resolved.iter().map(describe_action).collect(),
    })
}

/// Reads an account's persisted polling watermark, falling back to "now"
/// (no back-fill, PRD §6) when the account has none — or no row — yet.
fn read_watermark(conn: &Connection, account_id: u64) -> anyhow::Result<Watermark> {
    let stored: Option<String> = conn
        .query_row(
            "SELECT last_cursor FROM accounts WHERE bichon_account_id = ?1",
            rusqlite::params![account_id.to_string()],
            |row| row.get(0),
        )
        .ok()
        .flatten();
    Ok(stored
        .and_then(|text| text.parse::<i64>().ok())
        .map_or_else(Watermark::starting_now, Watermark::at))
}

/// Returns whether a message is already in the idempotency ledger.
fn is_already_processed(conn: &Connection, message_id: &str) -> Result<bool, rusqlite::Error> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM processed_messages WHERE message_id = ?1)",
        rusqlite::params![message_id],
        |row| row.get(0),
    )
}

/// A short, human-readable description of an [`Action`] for the report.
fn describe_action(action: &Action) -> String {
    match action {
        Action::CopyTo(folder) => format!("copy_to Berger/{folder}"),
        Action::MoveTo(folder) => format!("move_to Berger/{folder}"),
        Action::MarkSeen => "mark_seen".to_string(),
        Action::MarkFlagged => "mark_flagged".to_string(),
    }
}

/// Renders the per-account plans as a human-readable report.
pub fn render(plans: &[AccountPlan], llm_configured: bool) -> String {
    let mut out = String::from("Berger dry-run — no IMAP action applied, nothing recorded.\n");
    if llm_configured {
        out.push_str(
            "Note: an LLM classifier is configured; a real run would also apply its tags.\n\
             The dry run reports native-filter tags only (the LLM is a billable call).\n",
        );
    }

    let mut total_process = 0_usize;
    let mut total_skip = 0_usize;
    for account in plans {
        out.push_str(&format!("\nAccount {}\n", account.account_id));
        if account.messages.is_empty() {
            out.push_str("  (no new messages)\n");
            continue;
        }
        for message in &account.messages {
            match message {
                MessagePlan::WouldProcess {
                    message_id,
                    subject,
                    tags,
                    actions,
                } => {
                    total_process += 1;
                    out.push_str(&format!("  would process  {message_id}\n"));
                    out.push_str(&format!("    subject: {subject}\n"));
                    out.push_str(&format!("    tags:    {}\n", join_or_none(tags)));
                    out.push_str(&format!("    actions: {}\n", join_or_none(actions)));
                }
                MessagePlan::WouldSkip {
                    message_id,
                    subject,
                } => {
                    total_skip += 1;
                    out.push_str(&format!(
                        "  would skip     {message_id}  (already processed)\n"
                    ));
                    out.push_str(&format!("    subject: {subject}\n"));
                }
            }
        }
    }

    out.push_str(&format!(
        "\n{total_process} message(s) would be processed, {total_skip} skipped.\n"
    ));
    out
}

/// Joins `items` with `, `, or yields `(none)` when empty.
fn join_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "(none)".to_string()
    } else {
        items.join(", ")
    }
}

/// Loads the configuration, runs one dry poll cycle and prints the plan.
///
/// # Errors
/// Returns an error if the configuration cannot be loaded, the Bichon
/// client or the filters cannot be built, or polling fails.
pub async fn run(config_path: &str) -> anyhow::Result<()> {
    let raw = std::fs::read_to_string(config_path)
        .with_context(|| format!("reading config `{config_path}`"))?;
    let config = BergerConfig::parse(&raw).context("parsing the configuration")?;

    let bichon = BichonClient::new(
        config.bichon.base_url.clone(),
        config.bichon.api_token.expose(),
    )
    .context("building the Bichon client")?;
    let filters = compile_filters(&config.filters).context("compiling the filters")?;

    // The dry run only reads the sidecar (for the watermark and the
    // idempotency check). Open it read-only when it exists; otherwise run
    // against a throwaway in-memory database so a first-ever dry run still
    // works — and still records nothing on disk.
    let conn = open_sidecar_readonly_or_memory(&config.database.path)?;

    let account_ids = parse_account_ids(&config)?;
    let plans = plan(&bichon, &account_ids, &filters, &config.actions, &conn).await?;

    print!("{}", render(&plans, config.llm.is_some()));
    Ok(())
}

/// Opens the sidecar read-only when it exists, or an empty in-memory
/// database otherwise — so a dry run never creates the sidecar on disk.
///
/// The in-memory fallback carries just the two tables a dry run reads
/// (`accounts` for the watermark, `processed_messages` for the idempotency
/// check), both empty: every message is then planned as new and the
/// watermark anchors at "now".
fn open_sidecar_readonly_or_memory(path: &str) -> anyhow::Result<Connection> {
    if std::path::Path::new(path).exists() {
        return crate::cli::db::open_readonly(path);
    }
    let conn =
        Connection::open_in_memory().context("opening an in-memory sidecar for the dry run")?;
    // Schema for the two tables the read path touches; matches V1's
    // definitions. Nothing is written to disk.
    conn.execute_batch(
        "CREATE TABLE accounts (\
            id INTEGER PRIMARY KEY, \
            name TEXT NOT NULL UNIQUE, \
            bichon_account_id TEXT NOT NULL, \
            last_cursor TEXT, \
            last_polled_at TIMESTAMP, \
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP\
         );\
         CREATE TABLE processed_messages (\
            message_id TEXT PRIMARY KEY, \
            account_id INTEGER, \
            bichon_uri TEXT, \
            subject TEXT, \
            from_email TEXT, \
            from_name TEXT, \
            date TIMESTAMP, \
            processed_at TIMESTAMP NOT NULL, \
            berger_version TEXT NOT NULL, \
            config_hash TEXT NOT NULL\
         );",
    )
    .context("preparing the in-memory dry-run sidecar")?;
    Ok(conn)
}

/// Parses each account's `bichon_account_id` into the numeric id the
/// Bichon search API expects.
fn parse_account_ids(config: &BergerConfig) -> anyhow::Result<Vec<u64>> {
    config
        .accounts
        .iter()
        .map(|account| {
            account.bichon_account_id.parse::<u64>().with_context(|| {
                format!(
                    "account `{}` has a non-numeric bichon_account_id",
                    account.name
                )
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_action_renders_each_primitive() {
        assert_eq!(
            describe_action(&Action::CopyTo("work".to_string())),
            "copy_to Berger/work"
        );
        assert_eq!(
            describe_action(&Action::MoveTo("junk".to_string())),
            "move_to Berger/junk"
        );
        assert_eq!(describe_action(&Action::MarkSeen), "mark_seen");
        assert_eq!(describe_action(&Action::MarkFlagged), "mark_flagged");
    }

    #[test]
    fn join_or_none_handles_the_empty_case() {
        assert_eq!(join_or_none(&[]), "(none)");
        assert_eq!(join_or_none(&["a".to_string(), "b".to_string()]), "a, b");
    }

    #[test]
    fn render_states_that_nothing_was_applied() {
        let text = render(&[], false);
        assert!(text.contains("no IMAP action applied"));
        assert!(text.contains("nothing recorded"));
    }

    #[test]
    fn render_notes_a_configured_llm() {
        let with_llm = render(&[], true);
        assert!(with_llm.contains("LLM classifier is configured"));
        let without_llm = render(&[], false);
        assert!(!without_llm.contains("LLM classifier is configured"));
    }

    #[test]
    fn render_counts_processed_and_skipped_messages() {
        let plans = vec![AccountPlan {
            account_id: 1,
            messages: vec![
                MessagePlan::WouldProcess {
                    message_id: "<a@x>".to_string(),
                    subject: "A".to_string(),
                    tags: vec!["notif".to_string()],
                    actions: vec!["mark_seen".to_string()],
                },
                MessagePlan::WouldSkip {
                    message_id: "<b@x>".to_string(),
                    subject: "B".to_string(),
                },
            ],
        }];
        let text = render(&plans, false);
        assert!(text.contains("1 message(s) would be processed, 1 skipped"));
        assert!(text.contains("would process  <a@x>"));
        assert!(text.contains("would skip     <b@x>"));
    }

    #[test]
    fn render_reports_an_account_with_no_new_messages() {
        let plans = vec![AccountPlan {
            account_id: 42,
            messages: Vec::new(),
        }];
        let text = render(&plans, false);
        assert!(text.contains("Account 42"));
        assert!(text.contains("(no new messages)"));
    }

    #[test]
    fn parse_account_ids_rejects_a_non_numeric_id() {
        let config = BergerConfig::parse(
            r#"
bichon:
  base_url: "https://bichon.example"
  api_token: "tok"
database:
  path: "berger.db"
accounts:
  - name: "Bad"
    bichon_account_id: "not-a-number"
    imap:
      host: "imap.example"
      user: "berger"
      password: "pw"
"#,
        )
        .unwrap();
        assert!(parse_account_ids(&config).is_err());
    }

    #[test]
    fn parse_account_ids_parses_numeric_ids() {
        let config = BergerConfig::parse(
            r#"
bichon:
  base_url: "https://bichon.example"
  api_token: "tok"
database:
  path: "berger.db"
accounts:
  - name: "LINAGORA"
    bichon_account_id: "8525922389589073"
    imap:
      host: "imap.example"
      user: "berger"
      password: "pw"
"#,
        )
        .unwrap();
        assert_eq!(
            parse_account_ids(&config).unwrap(),
            vec![8_525_922_389_589_073]
        );
    }

    #[test]
    fn read_watermark_falls_back_to_now_for_an_unknown_account() {
        let database = crate::storage::database::Database::open(":memory:").unwrap();
        // No accounts row: read_watermark must not error, it anchors at now.
        let watermark = read_watermark(database.connection(), 999).unwrap();
        let now = Watermark::starting_now().as_epoch_ms();
        // Anchored within a generous window of the current time.
        assert!((now - watermark.as_epoch_ms()).abs() < 60_000);
    }

    #[test]
    fn read_watermark_reads_a_persisted_value() {
        let database = crate::storage::database::Database::open(":memory:").unwrap();
        let conn = database.connection();
        conn.execute(
            "INSERT INTO accounts (name, bichon_account_id, last_cursor) VALUES ('a', '5', '1700000000000')",
            [],
        )
        .unwrap();
        let watermark = read_watermark(conn, 5).unwrap();
        assert_eq!(watermark.as_epoch_ms(), 1_700_000_000_000);
    }
}
