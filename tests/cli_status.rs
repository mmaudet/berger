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

//! Integration test for `berger status` (PRD §5.8, §5.9, §10): a
//! health/stats summary of the sidecar — table counts, recent activity,
//! cumulative LLM cost, and webhook success rate.

use berger::cli::status::{collect, render};
use berger::storage::database::Database;

/// Seeds the sidecar with two processed messages — one within the last
/// day, one well in the past — plus tags, LLM decisions, actions and
/// webhooks, and returns the database handle.
fn seeded_database() -> Database {
    let database = Database::open(":memory:").unwrap();
    let conn = database.connection();
    conn.execute(
        "INSERT INTO accounts (id, name, bichon_account_id) VALUES (1, 'LINAGORA', 'b-1')",
        [],
    )
    .unwrap();

    // A message processed just now — counts towards the 24h and 7d windows.
    conn.execute(
        "INSERT INTO processed_messages (message_id, account_id, processed_at, berger_version, config_hash) \
         VALUES ('<recent@berger.test>', 1, datetime('now'), '0.0.1', 'h')",
        [],
    )
    .unwrap();
    // A message processed a year ago — outside both windows.
    conn.execute(
        "INSERT INTO processed_messages (message_id, account_id, processed_at, berger_version, config_hash) \
         VALUES ('<old@berger.test>', 1, datetime('now', '-365 days'), '0.0.1', 'h')",
        [],
    )
    .unwrap();

    for (message, tag) in [
        ("<recent@berger.test>", "cat/work"),
        ("<old@berger.test>", "newsletter"),
    ] {
        conn.execute(
            "INSERT INTO applied_tags (message_id, tag, applied_at) VALUES (?1, ?2, datetime('now'))",
            rusqlite::params![message, tag],
        )
        .unwrap();
    }

    conn.execute(
        "INSERT INTO llm_decisions \
         (message_id, model, prompt_hash, prompt_text, response_json, tokens_input, tokens_output, latency_ms, cost_usd, called_at) \
         VALUES ('<recent@berger.test>', 'mistral-small-latest', 'ph', 'p', '{}', 100, 20, 400, 0.0030, datetime('now'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO llm_decisions \
         (message_id, model, prompt_hash, prompt_text, response_json, tokens_input, tokens_output, latency_ms, cost_usd, called_at) \
         VALUES ('<old@berger.test>', 'mistral-small-latest', 'ph2', 'p', '{}', 50, 10, 300, 0.0020, datetime('now'))",
        [],
    )
    .unwrap();

    // One successful action, one failed.
    conn.execute(
        "INSERT INTO executed_actions (message_id, action_type, target, succeeded, executed_at) \
         VALUES ('<recent@berger.test>', 'copy_to', 'Berger/cat/work', 1, datetime('now'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO executed_actions (message_id, action_type, target, succeeded, error, executed_at) \
         VALUES ('<old@berger.test>', 'move_to', 'Berger/news', 0, 'failed', datetime('now'))",
        [],
    )
    .unwrap();

    // One successful webhook, one failed.
    conn.execute(
        "INSERT INTO webhook_emissions (message_id, webhook_name, payload_json, http_status, attempts, succeeded, emitted_at) \
         VALUES ('<recent@berger.test>', 'linatwin-draft', '{}', 200, 1, 1, datetime('now'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO webhook_emissions (message_id, webhook_name, payload_json, http_status, attempts, succeeded, emitted_at) \
         VALUES ('<old@berger.test>', 'hermes-push-urgent', '{}', 500, 3, 0, datetime('now'))",
        [],
    )
    .unwrap();

    database
}

#[tokio::test]
async fn collect_counts_every_table() {
    let summary = collect(seeded_database().connection()).unwrap();
    assert_eq!(summary.accounts, 1);
    assert_eq!(summary.processed_messages, 2);
    assert_eq!(summary.applied_tags, 2);
    assert_eq!(summary.llm_decisions, 2);
    assert_eq!(summary.executed_actions, 2);
    assert_eq!(summary.webhook_emissions, 2);
}

#[tokio::test]
async fn collect_counts_recent_activity_in_the_time_windows() {
    let summary = collect(seeded_database().connection()).unwrap();
    // Only `<recent@...>` falls inside the windows; `<old@...>` is a year old.
    assert_eq!(summary.processed_last_24h, 1, "one message in the last day");
    assert_eq!(summary.processed_last_7d, 1, "one message in the last week");
}

#[tokio::test]
async fn collect_sums_llm_cost_and_webhook_outcomes() {
    let summary = collect(seeded_database().connection()).unwrap();
    // 0.0030 + 0.0020 cumulative LLM spend.
    assert!(
        (summary.llm_cost_usd - 0.005).abs() < 1e-9,
        "cost was {}",
        summary.llm_cost_usd
    );
    assert_eq!(summary.llm_tokens_input, 150);
    assert_eq!(summary.llm_tokens_output, 30);
    assert_eq!(summary.webhooks_succeeded, 1);
    assert_eq!(summary.webhooks_failed, 1);
    assert_eq!(summary.actions_succeeded, 1);
    assert_eq!(summary.actions_failed, 1);
}

#[tokio::test]
async fn collect_on_an_empty_sidecar_reports_all_zeros() {
    let database = Database::open(":memory:").unwrap();
    let summary = collect(database.connection()).unwrap();
    assert_eq!(summary.processed_messages, 0);
    assert_eq!(summary.llm_decisions, 0);
    assert!((summary.llm_cost_usd - 0.0).abs() < 1e-9);
    assert_eq!(summary.webhooks_succeeded, 0);
}

#[tokio::test]
async fn render_shows_the_headline_figures() {
    let summary = collect(seeded_database().connection()).unwrap();
    let text = render(&summary);
    assert!(text.contains("Messages processed"), "{text}");
    assert!(text.contains("last 24h"), "{text}");
    assert!(text.contains("LLM"), "{text}");
    // The cumulative cost is shown with a dollar sign.
    assert!(text.contains("$0.005"), "{text}");
    assert!(text.contains("Webhooks"), "{text}");
}
