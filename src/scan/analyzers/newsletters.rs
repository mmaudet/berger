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

//! Newsletter detection: dimension 4 of the scan (PRD v1.1 §4.2).
//!
//! A message is a newsletter when it carried a `List-Unsubscribe` header.
//! [`detect_newsletters`] groups those messages by sender domain and, for
//! each domain, reports how much newsletter mail it sends and how many
//! distinct addresses it sends it from.

use std::collections::HashMap;
use std::collections::HashSet;

use crate::scan::address::{domain_of, extract_address};
use crate::scan::analyzer::ScannedMessage;

/// Dimension 4: how many of the busiest newsletter domains to report. The
/// PRD sets no cap; 50 keeps the report bounded on a large inbox.
const TOP_NEWSLETTER_DOMAINS: usize = 50;

/// One sender domain and the newsletter mail it sends (dimension 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsletterDomain {
    /// The lowercased sender domain.
    pub domain: String,
    /// Newsletter messages received from this domain.
    pub messages: usize,
    /// Distinct sender addresses on this domain that sent newsletters.
    pub distinct_senders: usize,
}

/// Dimension 4: groups every newsletter message — one carrying a
/// `List-Unsubscribe` header — by sender domain and returns the busiest
/// domains, most newsletter mail first (ties broken by domain).
///
/// A newsletter message whose `From` yields no parseable address (and so
/// no domain) is skipped. Non-newsletter messages are ignored entirely.
pub fn detect_newsletters(inbox: &[ScannedMessage]) -> Vec<NewsletterDomain> {
    let mut messages: HashMap<String, usize> = HashMap::new();
    let mut senders: HashMap<String, HashSet<String>> = HashMap::new();
    for message in inbox {
        if !message.headers.list_unsubscribe {
            continue;
        }
        if let Some(address) = extract_address(&message.envelope.from)
            && let Some(domain) = domain_of(&address)
        {
            *messages.entry(domain.to_string()).or_default() += 1;
            senders
                .entry(domain.to_string())
                .or_default()
                .insert(address);
        }
    }

    let mut domains: Vec<NewsletterDomain> = messages
        .into_iter()
        .map(|(domain, message_count)| {
            let distinct_senders = senders.get(&domain).map_or(0, HashSet::len);
            NewsletterDomain {
                domain,
                messages: message_count,
                distinct_senders,
            }
        })
        .collect();
    domains.sort_by(|a, b| {
        b.messages
            .cmp(&a.messages)
            .then_with(|| a.domain.cmp(&b.domain))
    });
    domains.truncate(TOP_NEWSLETTER_DOMAINS);
    domains
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::types::Envelope;
    use crate::scan::headers::ScanHeaders;

    fn envelope(from: &str) -> Envelope {
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

    /// A scanned message whose `List-Unsubscribe` presence is set explicitly.
    fn scanned(envelope: &Envelope, list_unsubscribe: bool) -> ScannedMessage<'_> {
        ScannedMessage {
            envelope,
            headers: ScanHeaders {
                list_unsubscribe,
                ..ScanHeaders::default()
            },
        }
    }

    #[test]
    fn counts_newsletter_messages_per_domain() {
        let envelopes = [
            envelope("news@a.test"),
            envelope("news@a.test"),
            envelope("news@b.test"),
        ];
        let inbox = [
            scanned(&envelopes[0], true),
            scanned(&envelopes[1], true),
            scanned(&envelopes[2], true),
        ];
        assert_eq!(
            detect_newsletters(&inbox),
            vec![
                NewsletterDomain {
                    domain: "a.test".to_string(),
                    messages: 2,
                    distinct_senders: 1,
                },
                NewsletterDomain {
                    domain: "b.test".to_string(),
                    messages: 1,
                    distinct_senders: 1,
                },
            ]
        );
    }

    #[test]
    fn counts_distinct_senders_within_one_domain() {
        let envelopes = [
            envelope("alerts@shop.test"),
            envelope("Promotions <promo@shop.test>"),
            envelope("alerts@shop.test"),
        ];
        let inbox = [
            scanned(&envelopes[0], true),
            scanned(&envelopes[1], true),
            scanned(&envelopes[2], true),
        ];
        let domains = detect_newsletters(&inbox);
        assert_eq!(domains.len(), 1);
        assert_eq!(domains[0].domain, "shop.test");
        assert_eq!(domains[0].messages, 3);
        assert_eq!(domains[0].distinct_senders, 2);
    }

    #[test]
    fn ignores_messages_without_list_unsubscribe() {
        let envelopes = [
            envelope("news@a.test"),
            envelope("colleague@a.test"),
            envelope("friend@b.test"),
        ];
        let inbox = [
            scanned(&envelopes[0], true),
            scanned(&envelopes[1], false),
            scanned(&envelopes[2], false),
        ];
        assert_eq!(
            detect_newsletters(&inbox),
            vec![NewsletterDomain {
                domain: "a.test".to_string(),
                messages: 1,
                distinct_senders: 1,
            }]
        );
    }

    #[test]
    fn skips_newsletters_with_an_unparseable_sender() {
        let envelopes = [envelope("no address here"), envelope("real@x.test")];
        let inbox = [scanned(&envelopes[0], true), scanned(&envelopes[1], true)];
        let domains = detect_newsletters(&inbox);
        assert_eq!(domains.len(), 1);
        assert_eq!(domains[0].domain, "x.test");
    }

    #[test]
    fn orders_domains_by_message_count_then_by_name() {
        let envelopes = [
            envelope("a@quiet.test"),
            envelope("a@busy.test"),
            envelope("a@busy.test"),
            envelope("a@zeta.test"),
        ];
        let inbox = [
            scanned(&envelopes[0], true),
            scanned(&envelopes[1], true),
            scanned(&envelopes[2], true),
            scanned(&envelopes[3], true),
        ];
        let domains = detect_newsletters(&inbox);
        // busy: 2 messages first; quiet and zeta tie at 1, broken by name.
        assert_eq!(domains[0].domain, "busy.test");
        assert_eq!(domains[1].domain, "quiet.test");
        assert_eq!(domains[2].domain, "zeta.test");
    }

    #[test]
    fn is_capped() {
        let envelopes: Vec<Envelope> = (0..TOP_NEWSLETTER_DOMAINS + 10)
            .map(|n| envelope(&format!("news@d{n}.test")))
            .collect();
        let inbox: Vec<ScannedMessage> = envelopes
            .iter()
            .map(|envelope| scanned(envelope, true))
            .collect();
        assert_eq!(detect_newsletters(&inbox).len(), TOP_NEWSLETTER_DOMAINS);
    }

    #[test]
    fn empty_input_yields_no_domains() {
        assert!(detect_newsletters(&[]).is_empty());
    }
}
