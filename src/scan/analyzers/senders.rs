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

//! Sender analysis: dimensions 1, 2 and 3 of the scan (PRD v1.1 §4.2) —
//! the busiest senders, the recurring sender domains, and the contacts
//! the user exchanges mail with in both directions.

use std::collections::HashMap;

use crate::ingest::types::Envelope;
use crate::scan::address::{domain_of, extract_address};

/// Dimension 1: how many of the busiest senders to report.
const TOP_SENDERS: usize = 50;

/// Dimension 2: how many of the busiest bidirectional contacts to report.
const TOP_BIDIRECTIONAL: usize = 30;

/// Dimension 3: how many of the busiest domains to report. The PRD sets no
/// cap; 50 keeps the report bounded on a large inbox.
const TOP_DOMAINS: usize = 50;

/// One sender and how much mail the inbox received from them (dimension 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SenderCount {
    /// The sender's lowercased email address.
    pub address: String,
    /// Messages received from this sender.
    pub messages_received: usize,
}

/// One domain and how much inbound mail came from it (dimension 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainCount {
    /// The lowercased domain.
    pub domain: String,
    /// Messages received from this domain.
    pub messages_received: usize,
}

/// A contact the user both receives from and writes to (dimension 2).
#[derive(Debug, Clone, PartialEq)]
pub struct BidirectionalContact {
    /// The contact's lowercased email address.
    pub address: String,
    /// Messages received from this contact (INBOX).
    pub messages_received: usize,
    /// Messages the user sent to this contact (Sent folder).
    pub messages_sent_to: usize,
    /// `messages_sent_to / messages_received` — how two-way the exchange
    /// is. Always finite: a bidirectional contact has at least one
    /// received message.
    pub bidirectional_ratio: f64,
}

/// Dimension 1: counts inbound mail per sender address and returns the
/// busiest senders, most mail first (ties broken by address).
pub fn top_senders(inbox: &[&Envelope]) -> Vec<SenderCount> {
    let mut senders: Vec<SenderCount> = count_from(inbox)
        .into_iter()
        .map(|(address, messages_received)| SenderCount {
            address,
            messages_received,
        })
        .collect();
    senders.sort_by(|a, b| {
        b.messages_received
            .cmp(&a.messages_received)
            .then_with(|| a.address.cmp(&b.address))
    });
    senders.truncate(TOP_SENDERS);
    senders
}

/// Dimension 3: counts inbound mail per sender domain and returns the
/// busiest domains, most mail first (ties broken by domain).
pub fn top_domains(inbox: &[&Envelope]) -> Vec<DomainCount> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for envelope in inbox {
        if let Some(address) = extract_address(&envelope.from)
            && let Some(domain) = domain_of(&address)
        {
            *counts.entry(domain.to_string()).or_default() += 1;
        }
    }
    let mut domains: Vec<DomainCount> = counts
        .into_iter()
        .map(|(domain, messages_received)| DomainCount {
            domain,
            messages_received,
        })
        .collect();
    domains.sort_by(|a, b| {
        b.messages_received
            .cmp(&a.messages_received)
            .then_with(|| a.domain.cmp(&b.domain))
    });
    domains.truncate(TOP_DOMAINS);
    domains
}

/// Dimension 2: cross-references the senders of `inbox` mail with the
/// recipients of `sent` mail and returns the contacts present on both
/// sides — the user's genuine two-way correspondents — busiest first
/// (by total messages exchanged, ties broken by address).
pub fn bidirectional(inbox: &[&Envelope], sent: &[&Envelope]) -> Vec<BidirectionalContact> {
    let received = count_from(inbox);

    let mut sent_to: HashMap<String, usize> = HashMap::new();
    for envelope in sent {
        for recipient in &envelope.to {
            if let Some(address) = extract_address(recipient) {
                *sent_to.entry(address).or_default() += 1;
            }
        }
    }

    let mut contacts: Vec<BidirectionalContact> = received
        .into_iter()
        .filter_map(|(address, messages_received)| {
            let messages_sent_to = *sent_to.get(&address)?;
            let bidirectional_ratio = messages_sent_to as f64 / messages_received as f64;
            Some(BidirectionalContact {
                address,
                messages_received,
                messages_sent_to,
                bidirectional_ratio,
            })
        })
        .collect();
    contacts.sort_by(|a, b| {
        (b.messages_received + b.messages_sent_to)
            .cmp(&(a.messages_received + a.messages_sent_to))
            .then_with(|| a.address.cmp(&b.address))
    });
    contacts.truncate(TOP_BIDIRECTIONAL);
    contacts
}

/// Counts inbound mail per parseable sender address.
fn count_from(inbox: &[&Envelope]) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for envelope in inbox {
        if let Some(address) = extract_address(&envelope.from) {
            *counts.entry(address).or_default() += 1;
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(from: &str, to: &[&str]) -> Envelope {
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

    fn refs(envelopes: &[Envelope]) -> Vec<&Envelope> {
        envelopes.iter().collect()
    }

    #[test]
    fn top_senders_counts_inbound_mail_per_sender() {
        let envelopes = vec![
            envelope("alice@x.test", &[]),
            envelope("Alice <alice@x.test>", &[]),
            envelope("alice@x.test", &[]),
            envelope("bob@y.test", &[]),
        ];
        assert_eq!(
            top_senders(&refs(&envelopes)),
            vec![
                SenderCount {
                    address: "alice@x.test".to_string(),
                    messages_received: 3,
                },
                SenderCount {
                    address: "bob@y.test".to_string(),
                    messages_received: 1,
                },
            ]
        );
    }

    #[test]
    fn top_senders_breaks_count_ties_by_address() {
        let envelopes = vec![envelope("zoe@x.test", &[]), envelope("amy@x.test", &[])];
        let senders = top_senders(&refs(&envelopes));
        assert_eq!(senders[0].address, "amy@x.test");
        assert_eq!(senders[1].address, "zoe@x.test");
    }

    #[test]
    fn top_senders_ignores_unparseable_senders() {
        let envelopes = vec![
            envelope("no address here", &[]),
            envelope("real@x.test", &[]),
        ];
        let senders = top_senders(&refs(&envelopes));
        assert_eq!(senders.len(), 1);
        assert_eq!(senders[0].address, "real@x.test");
    }

    #[test]
    fn top_senders_is_capped() {
        let envelopes: Vec<Envelope> = (0..TOP_SENDERS + 10)
            .map(|n| envelope(&format!("s{n}@x.test"), &[]))
            .collect();
        assert_eq!(top_senders(&refs(&envelopes)).len(), TOP_SENDERS);
    }

    #[test]
    fn top_domains_counts_inbound_mail_per_domain() {
        let envelopes = vec![
            envelope("alice@x.test", &[]),
            envelope("bob@x.test", &[]),
            envelope("carol@y.test", &[]),
        ];
        assert_eq!(
            top_domains(&refs(&envelopes)),
            vec![
                DomainCount {
                    domain: "x.test".to_string(),
                    messages_received: 2,
                },
                DomainCount {
                    domain: "y.test".to_string(),
                    messages_received: 1,
                },
            ]
        );
    }

    #[test]
    fn top_domains_is_capped() {
        let envelopes: Vec<Envelope> = (0..TOP_DOMAINS + 10)
            .map(|n| envelope(&format!("s@d{n}.test"), &[]))
            .collect();
        assert_eq!(top_domains(&refs(&envelopes)).len(), TOP_DOMAINS);
    }

    #[test]
    fn bidirectional_finds_contacts_on_both_sides() {
        let inbox = vec![
            envelope("partner@x.test", &[]),
            envelope("newsletter@spam.test", &[]),
        ];
        let sent = vec![
            envelope("me@home.test", &["partner@x.test"]),
            envelope("me@home.test", &["stranger@y.test"]),
        ];
        let contacts = bidirectional(&refs(&inbox), &refs(&sent));
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].address, "partner@x.test");
    }

    #[test]
    fn bidirectional_computes_the_ratio() {
        let inbox = vec![
            envelope("partner@x.test", &[]),
            envelope("partner@x.test", &[]),
            envelope("partner@x.test", &[]),
            envelope("partner@x.test", &[]),
        ];
        let sent = vec![
            envelope("me@home.test", &["partner@x.test"]),
            envelope("me@home.test", &["partner@x.test"]),
        ];
        let contacts = bidirectional(&refs(&inbox), &refs(&sent));
        assert_eq!(contacts[0].messages_received, 4);
        assert_eq!(contacts[0].messages_sent_to, 2);
        assert!((contacts[0].bidirectional_ratio - 0.5).abs() < 1e-9);
    }

    #[test]
    fn bidirectional_is_empty_without_sent_mail() {
        let inbox = vec![envelope("partner@x.test", &[])];
        assert!(bidirectional(&refs(&inbox), &[]).is_empty());
    }

    #[test]
    fn bidirectional_orders_by_total_interaction() {
        let inbox = vec![
            envelope("quiet@x.test", &[]),
            envelope("busy@x.test", &[]),
            envelope("busy@x.test", &[]),
        ];
        let sent = vec![
            envelope("me@home.test", &["quiet@x.test"]),
            envelope("me@home.test", &["busy@x.test", "busy@x.test"]),
        ];
        // busy: 2 received + 2 sent = 4 ; quiet: 1 received + 1 sent = 2.
        let contacts = bidirectional(&refs(&inbox), &refs(&sent));
        assert_eq!(contacts[0].address, "busy@x.test");
        assert_eq!(contacts[1].address, "quiet@x.test");
    }

    #[test]
    fn bidirectional_is_capped() {
        let inbox: Vec<Envelope> = (0..TOP_BIDIRECTIONAL + 10)
            .map(|n| envelope(&format!("c{n}@x.test"), &[]))
            .collect();
        let recipient_strings: Vec<String> = (0..TOP_BIDIRECTIONAL + 10)
            .map(|n| format!("c{n}@x.test"))
            .collect();
        let recipient_refs: Vec<&str> = recipient_strings.iter().map(String::as_str).collect();
        let sent = vec![envelope("me@home.test", &recipient_refs)];
        assert_eq!(
            bidirectional(&refs(&inbox), &refs(&sent)).len(),
            TOP_BIDIRECTIONAL
        );
    }
}
