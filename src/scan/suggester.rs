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

//! Suggester: turns a [`ScanReport`]'s category dimensions into a small set
//! of `berger.yaml` filter rules — one factored rule per category, scored
//! by the PRD v1.1 §4.4 confidence formula and gated by min-evidence.
//!
//! Only the dimensions that *are* triage categories produce rules:
//! newsletters, mailing lists, notification services, two-way contacts and
//! spam. The pure-frequency dimensions — top senders, top domains — are
//! reported but never turned into rules: a frequent sender is not a
//! category, and grouping domains into themes is a human or LLM judgement.
//! Rules use only filter types the v1.0 config loader understands, so the
//! YAML merges straight into a real `berger.yaml` (PRD v1.1 §7).

use crate::scan::analyzer::ScanReport;

/// The kind of v1.0 filter a suggestion proposes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum SuggestionKind {
    /// A `sender_in:` rule with one or more address/domain patterns.
    SenderIn(Vec<String>),
    /// A `header_match:` rule on a named header.
    HeaderMatch {
        /// The header to match.
        header: String,
        /// The regex matched against it.
        pattern: String,
    },
    /// A `list_unsubscribe: true` native filter rule.
    ListUnsubscribe,
}

/// One candidate filter rule derived from the scan, ready to be reviewed
/// and merged into `berger.yaml`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SuggestedFilter {
    /// A unique, descriptive suggestion name.
    pub name: String,
    /// The filter rule itself.
    pub kind: SuggestionKind,
    /// The tag the rule would apply.
    pub tag: String,
    /// How many messages back this suggestion.
    pub evidence_messages: usize,
    /// Confidence in `[0, 1]` from the PRD §4.4 formula.
    pub confidence: f64,
    /// A one-line human rationale.
    pub rationale: String,
}

/// Every suggestion produced from one scan.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct Suggestions {
    /// Candidate filter rules.
    pub filters: Vec<SuggestedFilter>,
}

/// The PRD v1.1 §4.4 confidence score for a suggestion:
/// `min(1, ln(messages) / 4 + bidirectional_ratio * 0.3)`.
///
/// Zero messages score zero.
pub fn confidence(messages: usize, bidirectional_ratio: f64) -> f64 {
    if messages == 0 {
        return 0.0;
    }
    let score = (messages as f64).ln() / 4.0 + bidirectional_ratio * 0.3;
    score.clamp(0.0, 1.0)
}

/// Turns a [`ScanReport`] into a small set of candidate filter rules — one
/// factored rule per category dimension, each category needing at least
/// `min_evidence` messages of support (PRD v1.1 §4.4).
pub fn suggest(report: &ScanReport, min_evidence: usize) -> Suggestions {
    let mut filters: Vec<SuggestedFilter> = Vec::new();

    // Newsletters → one native `list_unsubscribe` rule covering them all.
    let newsletter_messages: usize = report
        .newsletters
        .iter()
        .map(|domain| domain.messages)
        .sum();
    if newsletter_messages >= min_evidence {
        filters.push(SuggestedFilter {
            name: "scan-newsletter".to_string(),
            kind: SuggestionKind::ListUnsubscribe,
            tag: "newsletter".to_string(),
            evidence_messages: newsletter_messages,
            confidence: confidence(newsletter_messages, 0.0),
            rationale: format!(
                "{} bulk messages across {} domains carry a List-Unsubscribe header",
                newsletter_messages,
                report.newsletters.len()
            ),
        });
    }

    // Mailing lists → one generic rule on the List-Id header.
    let list_messages: usize = report.mailing_lists.iter().map(|list| list.messages).sum();
    if list_messages >= min_evidence {
        filters.push(SuggestedFilter {
            name: "scan-mailing-list".to_string(),
            kind: SuggestionKind::HeaderMatch {
                header: "List-Id".to_string(),
                pattern: ".".to_string(),
            },
            tag: "mailing-list".to_string(),
            evidence_messages: list_messages,
            confidence: confidence(list_messages, 0.0),
            rationale: format!(
                "{} messages across {} lists carry a List-Id header",
                list_messages,
                report.mailing_lists.len()
            ),
        });
    }

    // Notification services → one `sender_in` rule listing the domains
    // that each cleared the evidence threshold.
    let services: Vec<_> = report
        .notification_services
        .iter()
        .filter(|service| service.messages >= min_evidence)
        .collect();
    if !services.is_empty() {
        let messages: usize = services.iter().map(|service| service.messages).sum();
        filters.push(SuggestedFilter {
            name: "scan-notification".to_string(),
            kind: SuggestionKind::SenderIn(
                services
                    .iter()
                    .map(|service| service.domain.clone())
                    .collect(),
            ),
            tag: "notification".to_string(),
            evidence_messages: messages,
            confidence: confidence(messages, 0.0),
            rationale: format!(
                "{} automated messages from {} notification domains",
                messages,
                services.len()
            ),
        });
    }

    // Two-way contacts → one `sender_in` rule listing the VIP addresses.
    let contacts: Vec<_> = report
        .bidirectional
        .iter()
        .filter(|contact| contact.messages_received >= min_evidence)
        .collect();
    if !contacts.is_empty() {
        let messages: usize = contacts
            .iter()
            .map(|contact| contact.messages_received)
            .sum();
        filters.push(SuggestedFilter {
            name: "scan-vip".to_string(),
            kind: SuggestionKind::SenderIn(
                contacts
                    .iter()
                    .map(|contact| contact.address.clone())
                    .collect(),
            ),
            tag: "vip".to_string(),
            evidence_messages: messages,
            confidence: confidence(messages, 0.0),
            rationale: format!(
                "{} messages from {} two-way contacts",
                messages,
                contacts.len()
            ),
        });
    }

    // Confirmed spam → one rule on the X-Spam-Flag header.
    if report.spam.flagged >= min_evidence {
        filters.push(SuggestedFilter {
            name: "scan-spam-confirmed".to_string(),
            kind: SuggestionKind::HeaderMatch {
                header: "X-Spam-Flag".to_string(),
                pattern: "(?i)yes".to_string(),
            },
            tag: "spam".to_string(),
            evidence_messages: report.spam.flagged,
            confidence: confidence(report.spam.flagged, 0.0),
            rationale: format!(
                "{} messages already flagged by the upstream spam filter",
                report.spam.flagged
            ),
        });
    }

    Suggestions { filters }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::analyzer::ScanReport;
    use crate::scan::analyzers::lists::MailingList;
    use crate::scan::analyzers::newsletters::NewsletterDomain;
    use crate::scan::analyzers::notifications::NotificationService;
    use crate::scan::analyzers::senders::{BidirectionalContact, DomainCount, SenderCount};
    use crate::scan::analyzers::spam::SpamSummary;
    use crate::scan::analyzers::volume::VolumeProfile;

    fn empty_report() -> ScanReport {
        ScanReport {
            messages_analyzed: 0,
            inbox_messages: 0,
            sent_messages: 0,
            top_senders: Vec::new(),
            top_domains: Vec::new(),
            bidirectional: Vec::new(),
            newsletters: Vec::new(),
            mailing_lists: Vec::new(),
            notification_services: Vec::new(),
            spam: SpamSummary::default(),
            subject_ngrams: Vec::new(),
            languages: Vec::new(),
            volume: VolumeProfile::default(),
        }
    }

    fn contact(address: &str, received: usize) -> BidirectionalContact {
        BidirectionalContact {
            address: address.to_string(),
            messages_received: received,
            messages_sent_to: 5,
            bidirectional_ratio: 0.4,
        }
    }

    #[test]
    fn confidence_grows_with_message_volume() {
        assert!(confidence(50, 0.0) > confidence(5, 0.0));
    }

    #[test]
    fn confidence_never_exceeds_one() {
        assert!(confidence(1_000_000, 1.0) <= 1.0);
    }

    #[test]
    fn confidence_of_no_messages_is_zero() {
        assert_eq!(confidence(0, 0.0), 0.0);
    }

    #[test]
    fn confidence_rewards_a_two_way_exchange() {
        assert!(confidence(10, 0.8) > confidence(10, 0.0));
    }

    #[test]
    fn newsletters_yield_one_native_list_unsubscribe_rule() {
        let mut report = empty_report();
        report.newsletters = vec![
            NewsletterDomain {
                domain: "a.test".to_string(),
                messages: 20,
                distinct_senders: 3,
            },
            NewsletterDomain {
                domain: "b.test".to_string(),
                messages: 15,
                distinct_senders: 2,
            },
        ];
        let suggestions = suggest(&report, 5);
        assert_eq!(suggestions.filters.len(), 1);
        assert_eq!(suggestions.filters[0].kind, SuggestionKind::ListUnsubscribe);
        assert_eq!(suggestions.filters[0].tag, "newsletter");
    }

    #[test]
    fn mailing_lists_yield_one_generic_list_id_rule() {
        let mut report = empty_report();
        report.mailing_lists = vec![
            MailingList {
                list_id: "x@a.test".to_string(),
                messages: 10,
            },
            MailingList {
                list_id: "y@b.test".to_string(),
                messages: 8,
            },
        ];
        let suggestions = suggest(&report, 5);
        assert_eq!(suggestions.filters.len(), 1);
        assert!(matches!(
            &suggestions.filters[0].kind,
            SuggestionKind::HeaderMatch { header, pattern }
                if header.as_str() == "List-Id" && pattern.as_str() == "."
        ));
    }

    #[test]
    fn notification_services_become_one_sender_in_list() {
        let mut report = empty_report();
        report.notification_services = vec![
            NotificationService {
                domain: "github.com".to_string(),
                messages: 40,
            },
            NotificationService {
                domain: "gitlab.com".to_string(),
                messages: 12,
            },
        ];
        let suggestions = suggest(&report, 5);
        assert_eq!(suggestions.filters.len(), 1);
        assert!(matches!(
            &suggestions.filters[0].kind,
            SuggestionKind::SenderIn(domains) if domains.len() == 2
        ));
        assert_eq!(suggestions.filters[0].tag, "notification");
    }

    #[test]
    fn notification_drops_domains_below_min_evidence() {
        let mut report = empty_report();
        report.notification_services = vec![
            NotificationService {
                domain: "busy.test".to_string(),
                messages: 40,
            },
            NotificationService {
                domain: "rare.test".to_string(),
                messages: 2,
            },
        ];
        let suggestions = suggest(&report, 5);
        assert!(matches!(
            &suggestions.filters[0].kind,
            SuggestionKind::SenderIn(domains)
                if domains == &["busy.test".to_string()]
        ));
    }

    #[test]
    fn bidirectional_contacts_become_one_vip_rule() {
        let mut report = empty_report();
        report.bidirectional = vec![contact("a@x.test", 30), contact("b@x.test", 20)];
        let suggestions = suggest(&report, 5);
        assert_eq!(suggestions.filters.len(), 1);
        assert!(matches!(
            &suggestions.filters[0].kind,
            SuggestionKind::SenderIn(addresses) if addresses.len() == 2
        ));
        assert_eq!(suggestions.filters[0].tag, "vip");
    }

    #[test]
    fn spam_yields_one_header_match_rule() {
        let mut report = empty_report();
        report.spam = SpamSummary {
            flagged: 12,
            high_score: 0,
            dmarc_failures: 0,
        };
        let suggestions = suggest(&report, 5);
        assert_eq!(suggestions.filters.len(), 1);
        assert_eq!(suggestions.filters[0].tag, "spam");
    }

    #[test]
    fn the_frequency_dimensions_produce_no_rules() {
        // Top senders and top domains are reported, not turned into rules:
        // a frequent sender is not by itself a triage category.
        let mut report = empty_report();
        report.top_senders = vec![SenderCount {
            address: "a@x.test".to_string(),
            messages_received: 200,
        }];
        report.top_domains = vec![DomainCount {
            domain: "x.test".to_string(),
            messages_received: 500,
        }];
        assert!(suggest(&report, 5).filters.is_empty());
    }

    #[test]
    fn a_category_below_min_evidence_is_dropped() {
        let mut report = empty_report();
        report.newsletters = vec![NewsletterDomain {
            domain: "a.test".to_string(),
            messages: 3,
            distinct_senders: 1,
        }];
        assert!(suggest(&report, 5).filters.is_empty());
    }

    #[test]
    fn a_full_report_yields_one_rule_per_category() {
        let mut report = empty_report();
        report.newsletters = vec![NewsletterDomain {
            domain: "a.test".to_string(),
            messages: 20,
            distinct_senders: 2,
        }];
        report.mailing_lists = vec![MailingList {
            list_id: "l@a.test".to_string(),
            messages: 10,
        }];
        report.notification_services = vec![NotificationService {
            domain: "n.test".to_string(),
            messages: 30,
        }];
        report.bidirectional = vec![contact("v@x.test", 15)];
        report.spam = SpamSummary {
            flagged: 8,
            high_score: 0,
            dmarc_failures: 0,
        };
        assert_eq!(suggest(&report, 5).filters.len(), 5);
    }

    #[test]
    fn suggest_on_an_empty_report_proposes_nothing() {
        assert!(suggest(&empty_report(), 5).filters.is_empty());
    }

    #[test]
    fn a_suggestion_carries_a_positive_confidence_score() {
        let mut report = empty_report();
        report.notification_services = vec![NotificationService {
            domain: "n.test".to_string(),
            messages: 40,
        }];
        assert!(suggest(&report, 5).filters[0].confidence > 0.0);
    }
}
