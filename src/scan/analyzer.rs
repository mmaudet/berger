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

//! Scan aggregation: partition the fetched envelopes by folder, run every
//! dimension analyzer, and assemble the [`ScanReport`].

use crate::ingest::types::Envelope;
use crate::scan::analyzers::senders::{self, BidirectionalContact, DomainCount, SenderCount};
use crate::scan::folders::{FolderClass, classify_folder};

/// The result of a scan: the statistics measured over the inbox, ready to
/// be turned into configuration suggestions and a report.
#[derive(Debug, Clone, PartialEq)]
pub struct ScanReport {
    /// Messages the scan considered — the INBOX and Sent folders. Berger's
    /// own folders, and everything else, are excluded.
    pub messages_analyzed: usize,
    /// Messages found in the INBOX.
    pub inbox_messages: usize,
    /// Messages found in a Sent folder.
    pub sent_messages: usize,
    /// Dimension 1: the busiest senders.
    pub top_senders: Vec<SenderCount>,
    /// Dimension 3: the busiest sender domains.
    pub top_domains: Vec<DomainCount>,
    /// Dimension 2: the bidirectional contacts. Empty when no Sent mail
    /// was found in the window (degraded mode, PRD v1.1 §5.4).
    pub bidirectional: Vec<BidirectionalContact>,
}

/// Runs the scan's dimension analyzers over `envelopes` — the raw result
/// of a windowed fetch — and assembles a [`ScanReport`].
///
/// Envelopes are partitioned by folder: the INBOX feeds the sender and
/// domain dimensions, a Sent folder feeds the bidirectional dimension,
/// and everything else — archives, drafts, Berger's own folders — is
/// ignored.
pub fn analyze(envelopes: &[Envelope]) -> ScanReport {
    let inbox: Vec<&Envelope> = envelopes
        .iter()
        .filter(|envelope| folder_class(envelope) == FolderClass::Inbox)
        .collect();
    let sent: Vec<&Envelope> = envelopes
        .iter()
        .filter(|envelope| folder_class(envelope) == FolderClass::Sent)
        .collect();

    ScanReport {
        messages_analyzed: inbox.len() + sent.len(),
        inbox_messages: inbox.len(),
        sent_messages: sent.len(),
        top_senders: senders::top_senders(&inbox),
        top_domains: senders::top_domains(&inbox),
        bidirectional: senders::bidirectional(&inbox, &sent),
    }
}

/// The folder class of `envelope`, treating an absent `mailbox_name` as an
/// unknown — and therefore ignored — folder.
fn folder_class(envelope: &Envelope) -> FolderClass {
    classify_folder(envelope.mailbox_name.as_deref().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(mailbox: &str, from: &str, to: &[&str]) -> Envelope {
        Envelope {
            id: String::new(),
            message_id: String::new(),
            account_id: 1,
            account_email: None,
            mailbox_id: 1,
            mailbox_name: Some(mailbox.to_string()),
            uid: 1,
            subject: String::new(),
            preview: String::new(),
            from: from.to_string(),
            to: to.iter().map(|recipient| (*recipient).to_string()).collect(),
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

    #[test]
    fn analyze_partitions_inbox_and_sent() {
        let envelopes = vec![
            envelope("INBOX", "a@x.test", &[]),
            envelope("INBOX", "b@x.test", &[]),
            envelope("Sent", "me@home.test", &["a@x.test"]),
            envelope("Archives", "old@x.test", &[]),
        ];
        let report = analyze(&envelopes);
        assert_eq!(report.inbox_messages, 2);
        assert_eq!(report.sent_messages, 1);
        assert_eq!(report.messages_analyzed, 3);
    }

    #[test]
    fn analyze_runs_the_sender_dimensions_over_the_inbox_only() {
        let envelopes = vec![
            envelope("INBOX", "alice@x.test", &[]),
            envelope("Sent", "me@home.test", &["alice@x.test"]),
        ];
        let report = analyze(&envelopes);
        // The Sent message's own sender (me@home.test) is not a received
        // sender, so it never reaches the sender or domain dimensions.
        assert_eq!(report.top_senders.len(), 1);
        assert_eq!(report.top_senders[0].address, "alice@x.test");
        assert_eq!(report.top_domains.len(), 1);
        assert_eq!(report.top_domains[0].domain, "x.test");
    }

    #[test]
    fn analyze_runs_bidirectional_over_inbox_and_sent() {
        let envelopes = vec![
            envelope("INBOX", "partner@x.test", &[]),
            envelope("Sent", "me@home.test", &["partner@x.test"]),
        ];
        let report = analyze(&envelopes);
        assert_eq!(report.bidirectional.len(), 1);
        assert_eq!(report.bidirectional[0].address, "partner@x.test");
    }

    #[test]
    fn analyze_ignores_berger_and_other_folders() {
        let envelopes = vec![
            envelope("Berger/cat-work", "triaged@x.test", &[]),
            envelope("Trash", "deleted@x.test", &[]),
        ];
        let report = analyze(&envelopes);
        assert_eq!(report.messages_analyzed, 0);
        assert!(report.top_senders.is_empty());
    }

    #[test]
    fn analyze_on_an_empty_window_is_all_zero() {
        let report = analyze(&[]);
        assert_eq!(report.messages_analyzed, 0);
        assert_eq!(report.inbox_messages, 0);
        assert_eq!(report.sent_messages, 0);
        assert!(report.top_senders.is_empty());
        assert!(report.top_domains.is_empty());
        assert!(report.bidirectional.is_empty());
    }

    #[test]
    fn analyze_without_a_sent_folder_leaves_bidirectional_empty() {
        let envelopes = vec![envelope("INBOX", "partner@x.test", &[])];
        let report = analyze(&envelopes);
        assert_eq!(report.sent_messages, 0);
        assert!(report.bidirectional.is_empty());
    }
}
