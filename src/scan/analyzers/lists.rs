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

//! Mailing-list detection: dimension 5 of the scan (PRD v1.1 §4.2) — the
//! active mailing lists the inbox subscribes to, recognised by the
//! `List-Id` header and counted per list.

use std::collections::HashMap;

use crate::scan::analyzer::ScannedMessage;

/// Dimension 5: how many of the busiest mailing lists to report. The PRD
/// sets no cap; 50 keeps the report bounded on a large inbox.
const TOP_LISTS: usize = 50;

/// One mailing list the inbox receives mail from (dimension 5).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MailingList {
    /// The normalized list identifier — the `<...>` part of the `List-Id`
    /// header, trimmed and lowercased.
    pub list_id: String,
    /// Messages received from this list.
    pub messages: usize,
}

/// Dimension 5: groups inbox messages by their normalized `List-Id` and
/// returns the busiest mailing lists, most mail first (ties broken by
/// list identifier).
///
/// A message is on a mailing list when its parsed headers carry a
/// `List-Id`. Messages with no `List-Id`, and those whose identifier
/// normalizes to the empty string, are skipped.
pub fn detect_mailing_lists(inbox: &[ScannedMessage]) -> Vec<MailingList> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for message in inbox {
        if let Some(raw) = &message.headers.list_id {
            let identifier = normalize_list_id(raw);
            if !identifier.is_empty() {
                *counts.entry(identifier).or_default() += 1;
            }
        }
    }
    let mut lists: Vec<MailingList> = counts
        .into_iter()
        .map(|(list_id, messages)| MailingList { list_id, messages })
        .collect();
    lists.sort_by(|a, b| {
        b.messages
            .cmp(&a.messages)
            .then_with(|| a.list_id.cmp(&b.list_id))
    });
    lists.truncate(TOP_LISTS);
    lists
}

/// Normalizes a raw `List-Id` header value into a list identifier.
///
/// `List-Id` comes in several shapes — `Rust Users <rust-users.rust-lang.org>`,
/// `<rust-users.rust-lang.org>`, or a bare `rust-users.rust-lang.org`. When
/// a `<...>` segment is present the text between the first `<` and the next
/// `>` is taken; otherwise the whole value is used. The result is trimmed
/// and lowercased.
fn normalize_list_id(raw: &str) -> String {
    let candidate = match (raw.find('<'), raw.find('>')) {
        (Some(open), Some(close)) if open < close => &raw[open + 1..close],
        _ => raw,
    };
    candidate.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::types::Envelope;
    use crate::scan::headers::ScanHeaders;

    fn envelope() -> Envelope {
        Envelope {
            id: String::new(),
            message_id: String::new(),
            account_id: 1,
            account_email: None,
            mailbox_id: 1,
            mailbox_name: None,
            uid: 1,
            subject: String::new(),
            preview: String::new(),
            from: String::new(),
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            date: 0,
            internal_date: 0,
            ingest_at: 0,
            size: 0,
            thread_id: String::new(),
            attachment_count: 0,
            regular_attachment_count: 0,
            tags: None,
            content_hash: String::new(),
        }
    }

    fn on_list<'a>(envelope: &'a Envelope, list_id: Option<&str>) -> ScannedMessage<'a> {
        ScannedMessage {
            envelope,
            headers: ScanHeaders {
                list_id: list_id.map(str::to_string),
                ..ScanHeaders::default()
            },
        }
    }

    #[test]
    fn groups_messages_by_list_id() {
        let envelope = envelope();
        let inbox = [
            on_list(&envelope, Some("<rust-users.rust-lang.org>")),
            on_list(&envelope, Some("<rust-users.rust-lang.org>")),
            on_list(&envelope, Some("<rust-users.rust-lang.org>")),
            on_list(&envelope, Some("<announce.python.org>")),
        ];
        assert_eq!(
            detect_mailing_lists(&inbox),
            vec![
                MailingList {
                    list_id: "rust-users.rust-lang.org".to_string(),
                    messages: 3,
                },
                MailingList {
                    list_id: "announce.python.org".to_string(),
                    messages: 1,
                },
            ]
        );
    }

    #[test]
    fn the_three_list_id_shapes_normalize_to_the_same_identifier() {
        let envelope = envelope();
        let inbox = [
            on_list(&envelope, Some("Rust Users <rust-users.rust-lang.org>")),
            on_list(&envelope, Some("<rust-users.rust-lang.org>")),
            on_list(&envelope, Some("rust-users.rust-lang.org")),
        ];
        let lists = detect_mailing_lists(&inbox);
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].list_id, "rust-users.rust-lang.org");
        assert_eq!(lists[0].messages, 3);
    }

    #[test]
    fn normalizes_case_and_surrounding_whitespace() {
        let envelope = envelope();
        let inbox = [
            on_list(
                &envelope,
                Some("  Rust Users < Rust-Users.Rust-Lang.ORG > "),
            ),
            on_list(&envelope, Some("rust-users.rust-lang.org")),
        ];
        let lists = detect_mailing_lists(&inbox);
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].list_id, "rust-users.rust-lang.org");
        assert_eq!(lists[0].messages, 2);
    }

    #[test]
    fn ignores_messages_with_no_list_id() {
        let envelope = envelope();
        let inbox = [
            on_list(&envelope, Some("<rust-users.rust-lang.org>")),
            on_list(&envelope, None),
            on_list(&envelope, None),
        ];
        let lists = detect_mailing_lists(&inbox);
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].messages, 1);
    }

    #[test]
    fn skips_a_list_id_that_normalizes_to_empty() {
        let envelope = envelope();
        let inbox = [
            on_list(&envelope, Some("<>")),
            on_list(&envelope, Some("   ")),
            on_list(&envelope, Some("<rust-users.rust-lang.org>")),
        ];
        let lists = detect_mailing_lists(&inbox);
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].list_id, "rust-users.rust-lang.org");
    }

    #[test]
    fn sorts_by_message_count_then_breaks_ties_by_list_id() {
        let envelope = envelope();
        let inbox = [
            on_list(&envelope, Some("<zebra.list.test>")),
            on_list(&envelope, Some("<alpha.list.test>")),
        ];
        let lists = detect_mailing_lists(&inbox);
        // Both have one message: ties break on the identifier, ascending.
        assert_eq!(lists[0].list_id, "alpha.list.test");
        assert_eq!(lists[1].list_id, "zebra.list.test");
    }

    #[test]
    fn is_capped_at_the_busiest_lists() {
        let envelope = envelope();
        let identifiers: Vec<String> = (0..TOP_LISTS + 10)
            .map(|n| format!("<list-{n}.test>"))
            .collect();
        let inbox: Vec<ScannedMessage> = identifiers
            .iter()
            .map(|identifier| on_list(&envelope, Some(identifier)))
            .collect();
        assert_eq!(detect_mailing_lists(&inbox).len(), TOP_LISTS);
    }

    #[test]
    fn empty_input_yields_no_lists() {
        assert!(detect_mailing_lists(&[]).is_empty());
    }
}
