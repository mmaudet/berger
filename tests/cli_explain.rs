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

//! Integration test for `berger explain <message-id>` (PRD §5.9, §10): it
//! reconstructs the full decision chain of one processed message — its
//! tags, the filters and LLM decision that produced them, the IMAP actions
//! applied, and the webhooks emitted.

use berger::cli::explain::{explain, render};
use berger::storage::database::Database;

/// Seeds the sidecar with one fully-triaged message and returns the
/// database handle. Rows are inserted with raw SQL, exactly as the `explain`
/// command reads them back.
fn seeded_database(message_id: &str) -> Database {
    let database = Database::open(":memory:").unwrap();
    let conn = database.connection();
    conn.execute(
        "INSERT INTO accounts (id, name, bichon_account_id) VALUES (1, 'LINAGORA', 'bichon-1')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO processed_messages \
         (message_id, account_id, bichon_uri, subject, from_email, from_name, date, processed_at, berger_version, config_hash) \
         VALUES (?1, 1, 'https://bichon.example/m/1', 'Validation architecture', 'arnaud@interieur.gouv.fr', 'Arnaud Clair', 1700000000000, '2026-05-19T08:32:15Z', '0.0.1', 'cfg-hash')",
        rusqlite::params![message_id],
    )
    .unwrap();
    for tag in ["cat/work", "a-repondre/pro"] {
        conn.execute(
            "INSERT INTO applied_tags (message_id, tag, applied_at) VALUES (?1, ?2, '2026-05-19T08:32:15Z')",
            rusqlite::params![message_id, tag],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO filter_matches (message_id, filter_type, filter_name, details_json) \
         VALUES (?1, 'sender_in', 'gouv-interieur', '{\"matched\":\"interieur.gouv.fr\"}')",
        rusqlite::params![message_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO llm_decisions \
         (message_id, model, prompt_hash, prompt_text, response_json, tokens_input, tokens_output, latency_ms, cost_usd, called_at) \
         VALUES (?1, 'mistral-small-latest', 'ph-1', 'classify this email', '{\"category\":\"work\",\"needs_reply\":true,\"priority\":5}', 320, 24, 540, 0.0002, '2026-05-19T08:32:14Z')",
        rusqlite::params![message_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO executed_actions \
         (message_id, action_type, target, imap_response, succeeded, error, executed_at) \
         VALUES (?1, 'copy_to', 'Berger/cat/work', 'OK', 1, NULL, '2026-05-19T08:32:15Z')",
        rusqlite::params![message_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO webhook_emissions \
         (message_id, webhook_name, payload_json, http_status, attempts, succeeded, emitted_at, completed_at) \
         VALUES (?1, 'linatwin-draft', '{\"event\":\"berger.tag_applied\"}', 200, 1, 1, '2026-05-19T08:32:15Z', '2026-05-19T08:32:16Z')",
        rusqlite::params![message_id],
    )
    .unwrap();
    database
}

#[tokio::test]
async fn explain_collects_the_full_decision_chain() {
    let database = seeded_database("<abc-def@interieur.gouv.fr>");
    let explanation = explain(database.connection(), "<abc-def@interieur.gouv.fr>")
        .unwrap()
        .expect("the message is in the sidecar");

    assert_eq!(
        explanation.message.subject.as_deref(),
        Some("Validation architecture")
    );
    assert_eq!(explanation.tags, ["a-repondre/pro", "cat/work"]);
    assert_eq!(explanation.filter_matches.len(), 1);
    assert_eq!(explanation.filter_matches[0].filter_name, "gouv-interieur");
    assert_eq!(explanation.llm_decisions.len(), 1);
    assert_eq!(explanation.llm_decisions[0].model, "mistral-small-latest");
    assert_eq!(explanation.executed_actions.len(), 1);
    assert!(explanation.executed_actions[0].succeeded);
    assert_eq!(explanation.webhook_emissions.len(), 1);
    assert_eq!(
        explanation.webhook_emissions[0].webhook_name,
        "linatwin-draft"
    );
}

#[tokio::test]
async fn explain_returns_none_for_an_unknown_message() {
    let database = seeded_database("<known@berger.test>");
    let explanation = explain(database.connection(), "<never-seen@berger.test>").unwrap();
    assert!(
        explanation.is_none(),
        "an unknown Message-ID has no explanation"
    );
}

#[tokio::test]
async fn render_shows_every_section_of_the_decision_chain() {
    let database = seeded_database("<abc-def@interieur.gouv.fr>");
    let explanation = explain(database.connection(), "<abc-def@interieur.gouv.fr>")
        .unwrap()
        .unwrap();
    let text = render(&explanation);

    // The rendered report names the message, every tag, the triggering
    // filter, the LLM model and the applied action.
    assert!(text.contains("<abc-def@interieur.gouv.fr>"), "{text}");
    assert!(text.contains("Validation architecture"), "{text}");
    assert!(text.contains("cat/work"), "{text}");
    assert!(text.contains("a-repondre/pro"), "{text}");
    assert!(text.contains("gouv-interieur"), "{text}");
    assert!(text.contains("mistral-small-latest"), "{text}");
    assert!(text.contains("copy_to"), "{text}");
    assert!(text.contains("linatwin-draft"), "{text}");
}

#[tokio::test]
async fn explain_handles_a_message_with_no_llm_or_webhooks() {
    // A message triaged on native filters alone: no LLM decision, no
    // webhook — explain must still succeed with empty sections.
    let database = Database::open(":memory:").unwrap();
    let conn = database.connection();
    conn.execute(
        "INSERT INTO accounts (id, name, bichon_account_id) VALUES (1, 'acct', 'b-1')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO processed_messages (message_id, account_id, processed_at, berger_version, config_hash) \
         VALUES ('<plain@berger.test>', 1, '2026-05-20T10:00:00Z', '0.0.1', 'h')",
        [],
    )
    .unwrap();

    let explanation = explain(conn, "<plain@berger.test>").unwrap().unwrap();
    assert!(explanation.tags.is_empty());
    assert!(explanation.llm_decisions.is_empty());
    assert!(explanation.webhook_emissions.is_empty());
    // Rendering an empty-section explanation must not panic.
    let text = render(&explanation);
    assert!(text.contains("<plain@berger.test>"));
}
