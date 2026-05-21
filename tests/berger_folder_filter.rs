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

//! Integration test for the first Bichon coherence rule (CLAUDE.md §3.2,
//! §4.4): a message that lives in one of Berger's own `Berger/*` folders
//! must never be handed back to the pipeline — otherwise Berger loops
//! forever on its own `copy_to` / `move_to` output.
//!
//! It drives a real [`BichonClient`] over HTTP against a mocked Bichon.

use berger::ingest::bichon_client::BichonClient;
use berger::ingest::poller::{Watermark, poll_account};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Builds one Bichon `Envelope` JSON object.
fn envelope_json(message_id: &str, mailbox_name: &str, date: i64) -> serde_json::Value {
    json!({
        "id": format!("env-{date}"),
        "message_id": message_id,
        "account_id": 1,
        "mailbox_id": 1,
        "mailbox_name": mailbox_name,
        "uid": 1,
        "subject": "",
        "preview": "",
        "from": "",
        "to": [],
        "cc": [],
        "bcc": [],
        "date": date,
        "internal_date": date,
        "ingest_at": date,
        "size": 0,
        "thread_id": "",
        "attachment_count": 0,
        "regular_attachment_count": 0,
        "content_hash": ""
    })
}

#[tokio::test]
async fn messages_in_berger_folders_are_never_handed_to_the_pipeline() {
    let server = MockServer::start().await;

    // Bichon's index holds the original INBOX message plus the two copies
    // Berger itself wrote into `Berger/*` folders on a previous cycle —
    // all three carry the same RFC 822 Message-ID.
    Mock::given(method("POST"))
        .and(path("/api/v1/search-messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "current_page": 1,
            "page_size": 200,
            "total_items": 3,
            "total_pages": 1,
            "items": [
                envelope_json("<original@example.test>", "INBOX", 100),
                envelope_json("<original@example.test>", "Berger/cat-work", 110),
                envelope_json("<original@example.test>", "INBOX.Berger.cat-work", 120),
            ]
        })))
        .mount(&server)
        .await;

    let client = BichonClient::new(server.uri(), "test-token").unwrap();
    let outcome = poll_account(&client, 1, Watermark::at(0)).await.unwrap();

    // Only the INBOX copy survives the read-side filter; the two copies in
    // Berger's own folders are dropped, so they are never re-processed.
    assert_eq!(outcome.envelopes.len(), 1);
    assert_eq!(outcome.envelopes[0].mailbox_name.as_deref(), Some("INBOX"));

    // The watermark still advanced past the Berger-folder copies (date 120),
    // so the next poll will not re-fetch them either.
    assert_eq!(outcome.watermark, Watermark::at(120));
}
