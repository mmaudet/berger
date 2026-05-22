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

//! Scan aggregation: split the fetched envelopes by folder, run every
//! dimension analyzer over the right partition, and assemble the
//! [`ScanReport`].

use crate::ingest::types::Envelope;
use crate::scan::analyzers::senders::{self, BidirectionalContact, DomainCount, SenderCount};
use crate::scan::folders::{FolderClass, classify_folder};
use crate::scan::headers::ScanHeaders;

/// An inbox message paired with the technical headers parsed from its raw
/// eml — the unit the header-based dimension analyzers (4-7) consume.
#[derive(Debug, Clone, PartialEq)]
pub struct ScannedMessage<'a> {
    /// The message envelope: From, To, Subject, Date, folder.
    pub envelope: &'a Envelope,
    /// The technical headers parsed from the message (PRD v1.1 §4.4).
    pub headers: ScanHeaders,
}

/// The result of a scan: the statistics measured over the inbox, ready to
/// be turned into configuration suggestions and a report.
#[derive(Debug, Clone, PartialEq)]
pub struct ScanReport {
    /// Messages the scan considered — the INBOX and Sent folders.
    pub messages_analyzed: usize,
    /// Messages found in the INBOX.
    pub inbox_messages: usize,
    /// Messages found in a Sent folder.
    pub sent_messages: usize,
    /// Dimension 1: the busiest senders.
    pub top_senders: Vec<SenderCount>,
    /// Dimension 3: the busiest sender domains.
    pub top_domains: Vec<DomainCount>,
    /// Dimension 2: the bidirectional contacts.
    pub bidirectional: Vec<BidirectionalContact>,
}

/// Splits fetched envelopes by folder into `(inbox, sent)`: the INBOX
/// (received mail) and a Sent folder. Everything else — archives, drafts,
/// and Berger's own `Berger/*` folders — is dropped (CLAUDE.md §3.2).
pub fn partition(envelopes: &[Envelope]) -> (Vec<&Envelope>, Vec<&Envelope>) {
    let inbox = envelopes
        .iter()
        .filter(|envelope| folder_class(envelope) == FolderClass::Inbox)
        .collect();
    let sent = envelopes
        .iter()
        .filter(|envelope| folder_class(envelope) == FolderClass::Sent)
        .collect();
    (inbox, sent)
}

/// Runs the dimension analyzers over a scanned `inbox` and the `sent`
/// envelopes, and assembles a [`ScanReport`].
pub fn analyze(inbox: &[ScannedMessage], sent: &[&Envelope]) -> ScanReport {
    let inbox_envelopes: Vec<&Envelope> = inbox.iter().map(|message| message.envelope).collect();
    ScanReport {
        messages_analyzed: inbox.len() + sent.len(),
        inbox_messages: inbox.len(),
        sent_messages: sent.len(),
        top_senders: senders::top_senders(&inbox_envelopes),
        top_domains: senders::top_domains(&inbox_envelopes),
        bidirectional: senders::bidirectional(&inbox_envelopes, sent),
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
            to: to
                .iter()
                .map(|recipient| (*recipient).to_string())
                .collect(),
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

    fn scanned(envelope: &Envelope) -> ScannedMessage<'_> {
        ScannedMessage {
            envelope,
            headers: ScanHeaders::default(),
        }
    }

    #[test]
    fn partition_splits_inbox_and_sent() {
        let envelopes = [
            envelope("INBOX", "a@x.test", &[]),
            envelope("INBOX", "b@x.test", &[]),
            envelope("Sent", "me@x.test", &[]),
            envelope("Archives", "old@x.test", &[]),
        ];
        let (inbox, sent) = partition(&envelopes);
        assert_eq!(inbox.len(), 2);
        assert_eq!(sent.len(), 1);
    }

    #[test]
    fn partition_drops_berger_and_other_folders() {
        let envelopes = [
            envelope("Berger/cat-work", "x@x.test", &[]),
            envelope("Trash", "y@x.test", &[]),
        ];
        let (inbox, sent) = partition(&envelopes);
        assert!(inbox.is_empty());
        assert!(sent.is_empty());
    }

    #[test]
    fn partition_of_an_empty_slice_is_empty() {
        let (inbox, sent) = partition(&[]);
        assert!(inbox.is_empty());
        assert!(sent.is_empty());
    }

    #[test]
    fn analyze_counts_the_partitions() {
        let inbox_envs = [
            envelope("INBOX", "a@x.test", &[]),
            envelope("INBOX", "b@x.test", &[]),
        ];
        let sent_envs = [envelope("Sent", "me@x.test", &["a@x.test"])];
        let inbox: Vec<ScannedMessage> = inbox_envs.iter().map(scanned).collect();
        let sent: Vec<&Envelope> = sent_envs.iter().collect();
        let report = analyze(&inbox, &sent);
        assert_eq!(report.inbox_messages, 2);
        assert_eq!(report.sent_messages, 1);
        assert_eq!(report.messages_analyzed, 3);
    }

    #[test]
    fn analyze_runs_the_sender_dimensions_over_the_inbox() {
        let inbox_envs = [envelope("INBOX", "alice@x.test", &[])];
        let inbox: Vec<ScannedMessage> = inbox_envs.iter().map(scanned).collect();
        let report = analyze(&inbox, &[]);
        assert_eq!(report.top_senders.len(), 1);
        assert_eq!(report.top_senders[0].address, "alice@x.test");
        assert_eq!(report.top_domains[0].domain, "x.test");
    }

    #[test]
    fn analyze_runs_bidirectional_over_inbox_and_sent() {
        let inbox_envs = [envelope("INBOX", "partner@x.test", &[])];
        let sent_envs = [envelope("Sent", "me@x.test", &["partner@x.test"])];
        let inbox: Vec<ScannedMessage> = inbox_envs.iter().map(scanned).collect();
        let sent: Vec<&Envelope> = sent_envs.iter().collect();
        let report = analyze(&inbox, &sent);
        assert_eq!(report.bidirectional.len(), 1);
        assert_eq!(report.bidirectional[0].address, "partner@x.test");
    }

    #[test]
    fn analyze_on_empty_input_is_all_zero() {
        let report = analyze(&[], &[]);
        assert_eq!(report.messages_analyzed, 0);
        assert!(report.top_senders.is_empty());
        assert!(report.top_domains.is_empty());
        assert!(report.bidirectional.is_empty());
    }
}
