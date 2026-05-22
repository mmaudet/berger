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

//! Integration tests for `berger scan` (PRD v1.1 §7): the scan runs over a
//! synthetic 100-message dataset and finds the planted patterns, performs
//! only reads against its source, and emits a configuration the existing
//! v1.0 loader accepts.

use std::collections::HashMap;
use std::sync::Mutex;

use berger::config::BergerConfig;
use berger::ingest::error::IngestError;
use berger::ingest::source::MessageSource;
use berger::ingest::types::{DataPage, EmailSearchRequest, Envelope, MinimalAccount};
use berger::scan::formatter::render_yaml;
use berger::scan::runner::scan;
use berger::scan::suggester::suggest;

/// One hour as epoch milliseconds, for placing a message in a day.
const HOUR_MS: i64 = 3_600_000;

const NEWSLETTER_EML: &[u8] = b"From: digest@news.example\r\nSubject: Weekly digest\r\nList-Unsubscribe: <mailto:unsub@news.example>\r\n\r\nNewsletter body.\r\n";
const MAILING_LIST_EML: &[u8] = b"From: poster@list.rust.example\r\nSubject: [rust-users] topic\r\nList-Id: Rust Users <users.rust.example>\r\n\r\nList body.\r\n";
const SPAM_EML: &[u8] =
    b"From: promo@spammy.example\r\nSubject: WIN a prize\r\nX-Spam-Flag: YES\r\n\r\nSpam body.\r\n";

/// One operation a [`FakeBichon`] was asked to perform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Search,
    Download,
}

/// An in-memory Bichon stand-in: serves a fixed dataset and records every
/// operation, so a test can prove the scan only ever read from it.
struct FakeBichon {
    envelopes: Vec<Envelope>,
    emls: HashMap<String, Vec<u8>>,
    ops: Mutex<Vec<Op>>,
}

impl MessageSource for FakeBichon {
    async fn list_accounts(&self) -> Result<Vec<MinimalAccount>, IngestError> {
        Ok(Vec::new())
    }

    async fn search_messages(
        &self,
        _request: EmailSearchRequest,
    ) -> Result<DataPage<Envelope>, IngestError> {
        self.ops.lock().unwrap().push(Op::Search);
        Ok(DataPage {
            current_page: None,
            page_size: Some(500),
            total_items: self.envelopes.len() as u64,
            items: self.envelopes.clone(),
            total_pages: None,
        })
    }

    async fn download_message(
        &self,
        _account_id: &str,
        envelope_id: &str,
    ) -> Result<Vec<u8>, IngestError> {
        self.ops.lock().unwrap().push(Op::Download);
        Ok(self.emls.get(envelope_id).cloned().unwrap_or_default())
    }
}

/// Builds an envelope with the fields the scan reads.
fn envelope(id: &str, from: &str, mailbox: &str, subject: &str, date: i64) -> Envelope {
    Envelope {
        id: id.to_string(),
        message_id: format!("<{id}@x.test>"),
        account_id: 1,
        account_email: None,
        mailbox_id: 1,
        mailbox_name: Some(mailbox.to_string()),
        uid: 1,
        subject: subject.to_string(),
        preview: String::new(),
        from: from.to_string(),
        to: Vec::new(),
        cc: Vec::new(),
        bcc: Vec::new(),
        date,
        internal_date: date,
        ingest_at: date,
        size: 0,
        thread_id: String::new(),
        attachment_count: 0,
        regular_attachment_count: 0,
        tags: None,
        content_hash: String::new(),
    }
}

/// A minimal RFC 822 message carrying no technical headers.
fn basic_eml(from: &str, subject: &str) -> Vec<u8> {
    format!("From: {from}\r\nSubject: {subject}\r\n\r\nBody.\r\n").into_bytes()
}

/// A 100-message dataset with deliberately planted patterns: 40 GitHub
/// notifications, 20 newsletters, 15 mailing-list posts, 10 spam messages,
/// 5 colleague messages, and 10 Sent messages.
fn synthetic_inbox() -> FakeBichon {
    let mut envelopes = Vec::new();
    let mut emls = HashMap::new();

    for n in 0..40 {
        let id = format!("gh-{n}");
        envelopes.push(envelope(
            &id,
            "noreply@github.com",
            "INBOX",
            "GitHub activity",
            9 * HOUR_MS,
        ));
        emls.insert(id, basic_eml("noreply@github.com", "GitHub activity"));
    }
    for n in 0..20 {
        let id = format!("nl-{n}");
        envelopes.push(envelope(
            &id,
            "digest@news.example",
            "INBOX",
            "Weekly digest",
            10 * HOUR_MS,
        ));
        emls.insert(id, NEWSLETTER_EML.to_vec());
    }
    for n in 0..15 {
        let id = format!("ml-{n}");
        envelopes.push(envelope(
            &id,
            "poster@list.rust.example",
            "INBOX",
            "[rust-users] topic",
            11 * HOUR_MS,
        ));
        emls.insert(id, MAILING_LIST_EML.to_vec());
    }
    for n in 0..10 {
        let id = format!("sp-{n}");
        envelopes.push(envelope(
            &id,
            "promo@spammy.example",
            "INBOX",
            "WIN a prize",
            3 * HOUR_MS,
        ));
        emls.insert(id, SPAM_EML.to_vec());
    }
    for n in 0..5 {
        let id = format!("co-{n}");
        envelopes.push(envelope(
            &id,
            "colleague@linagora.example",
            "INBOX",
            "Project sync",
            14 * HOUR_MS,
        ));
        emls.insert(id, basic_eml("colleague@linagora.example", "Project sync"));
    }
    for n in 0..10 {
        let id = format!("sent-{n}");
        envelopes.push(envelope(
            &id,
            "me@linagora.example",
            "Sent",
            "Re: Project sync",
            15 * HOUR_MS,
        ));
    }

    FakeBichon {
        envelopes,
        emls,
        ops: Mutex::new(Vec::new()),
    }
}

#[tokio::test]
async fn scan_finds_the_planted_patterns() {
    let bichon = synthetic_inbox();
    let report = scan(&bichon, &[1], 0).await.unwrap();

    assert_eq!(report.messages_analyzed, 100);
    assert_eq!(report.inbox_messages, 90);
    assert_eq!(report.sent_messages, 10);

    // GitHub is the dominant sender domain — 40 of the 90 inbox messages.
    assert_eq!(report.top_domains[0].domain, "github.com");
    assert_eq!(report.top_domains[0].messages_received, 40);

    // The List-* and X-Spam-Flag headers drive dimensions 4, 5 and 7.
    assert!(
        !report.newsletters.is_empty(),
        "the List-Unsubscribe messages should be detected as newsletters"
    );
    assert!(
        !report.mailing_lists.is_empty(),
        "the List-Id messages should be detected as mailing lists"
    );
    assert_eq!(report.spam.flagged, 10);
}

#[tokio::test]
async fn the_scan_performs_only_reads() {
    let bichon = synthetic_inbox();
    scan(&bichon, &[1], 0).await.unwrap();

    let ops = bichon.ops.lock().unwrap();
    // The scan's entire I/O is one search plus one download per inbox
    // message — every operation is a read. `ReadOnlyMessageSource` exposes
    // no mutating method, and the scan holds no LLM client and no IMAP
    // connection, so there is nothing else it could have called.
    assert_eq!(ops.iter().filter(|op| **op == Op::Search).count(), 1);
    assert_eq!(ops.iter().filter(|op| **op == Op::Download).count(), 90);
    assert!(ops.iter().all(|op| matches!(op, Op::Search | Op::Download)));
}

#[tokio::test]
async fn the_suggested_config_parses_with_the_v1_loader() {
    let bichon = synthetic_inbox();
    let report = scan(&bichon, &[1], 0).await.unwrap();
    let suggestions = suggest(&report, 5);
    let yaml = render_yaml(&report, &suggestions, 30);

    let config_text = format!(
        "bichon:\n  base_url: \"https://b.test\"\n  api_token: \"t\"\n\
         database:\n  path: \"berger.db\"\n\
         accounts:\n  - name: \"A\"\n    bichon_account_id: \"1\"\n    imap:\n      host: \"h\"\n      user: \"u\"\n      password: \"p\"\n\
         {yaml}"
    );
    let config = BergerConfig::parse(&config_text).expect("the suggested YAML must parse");
    assert!(
        !config.filters.is_empty(),
        "the scan should suggest at least one filter rule"
    );
}
