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

//! Suggester: turns a [`ScanReport`]'s raw dimensions into candidate
//! `berger.yaml` filter rules, each scored by the PRD v1.1 §4.4 confidence
//! formula and gated by the min-evidence threshold.
//!
//! Suggestions only ever use filter types the existing v1.0 config loader
//! already understands — `sender_in` and `header_match` — so the generated
//! YAML stays mergeable into a real `berger.yaml` (PRD v1.1 §7).

use std::collections::HashSet;

use crate::scan::analyzer::ScanReport;

/// The kind of v1.0 filter a suggestion proposes.
#[derive(Debug, Clone, PartialEq, Eq)]
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
}

/// One candidate filter rule derived from the scan, ready to be reviewed
/// and merged into `berger.yaml`.
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, Default, PartialEq)]
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

/// Turns a [`ScanReport`] into candidate filter rules, dropping every
/// dimension entry backed by fewer than `min_evidence` messages
/// (PRD v1.1 §4.4).
pub fn suggest(report: &ScanReport, min_evidence: usize) -> Suggestions {
    let mut filters: Vec<SuggestedFilter> = Vec::new();

    // Dimension 3: recurring sender domains.
    for domain in &report.top_domains {
        if domain.messages_received < min_evidence {
            continue;
        }
        filters.push(SuggestedFilter {
            name: format!("scan-domain-{}", slug(&domain.domain)),
            kind: SuggestionKind::SenderIn(vec![format!("*@{}", domain.domain)]),
            tag: format!("scan/{}", slug(&domain.domain)),
            evidence_messages: domain.messages_received,
            confidence: confidence(domain.messages_received, 0.0),
            rationale: format!(
                "{} messages received from this domain",
                domain.messages_received
            ),
        });
    }

    // Dimension 2: bidirectional contacts.
    for contact in &report.bidirectional {
        if contact.messages_received < min_evidence {
            continue;
        }
        filters.push(SuggestedFilter {
            name: format!("scan-contact-{}", slug(&contact.address)),
            kind: SuggestionKind::SenderIn(vec![contact.address.clone()]),
            tag: "vip".to_string(),
            evidence_messages: contact.messages_received,
            confidence: confidence(contact.messages_received, contact.bidirectional_ratio),
            rationale: format!(
                "{} received / {} sent — a two-way contact",
                contact.messages_received, contact.messages_sent_to
            ),
        });
    }

    // Dimension 4: newsletters.
    for newsletter in &report.newsletters {
        if newsletter.messages < min_evidence {
            continue;
        }
        filters.push(SuggestedFilter {
            name: format!("scan-newsletter-{}", slug(&newsletter.domain)),
            kind: SuggestionKind::SenderIn(vec![format!("*@{}", newsletter.domain)]),
            tag: "newsletter".to_string(),
            evidence_messages: newsletter.messages,
            confidence: confidence(newsletter.messages, 0.0),
            rationale: format!(
                "{} newsletter messages from {} distinct senders",
                newsletter.messages, newsletter.distinct_senders
            ),
        });
    }

    // Dimension 5: mailing lists.
    for list in &report.mailing_lists {
        if list.messages < min_evidence {
            continue;
        }
        filters.push(SuggestedFilter {
            name: format!("scan-list-{}", slug(&list.list_id)),
            kind: SuggestionKind::HeaderMatch {
                header: "List-Id".to_string(),
                pattern: regex::escape(&list.list_id),
            },
            tag: "mailing-list".to_string(),
            evidence_messages: list.messages,
            confidence: confidence(list.messages, 0.0),
            rationale: format!("{} messages from this mailing list", list.messages),
        });
    }

    // Dimension 6: notification services.
    for service in &report.notification_services {
        if service.messages < min_evidence {
            continue;
        }
        filters.push(SuggestedFilter {
            name: format!("scan-notification-{}", slug(&service.domain)),
            kind: SuggestionKind::SenderIn(vec![format!("*@{}", service.domain)]),
            tag: "notification".to_string(),
            evidence_messages: service.messages,
            confidence: confidence(service.messages, 0.0),
            rationale: format!(
                "{} automated notifications from this service",
                service.messages
            ),
        });
    }

    // Dimension 7: confirmed spam.
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

    Suggestions {
        filters: consolidate(filters),
    }
}

/// Consolidates candidate suggestions so a typical message is tagged at
/// most once by the `sender_in` rules (PRD v1.1 §4.4, tag-once objective):
/// when several dimensions propose the very same `sender_in` matcher, only
/// the highest-priority one is kept. `header_match` rules always pass
/// through — a message can still match one of those as well, hence "at
/// most twice".
fn consolidate(mut filters: Vec<SuggestedFilter>) -> Vec<SuggestedFilter> {
    filters.sort_by_key(|filter| tag_priority(&filter.tag));
    let mut seen: HashSet<Vec<String>> = HashSet::new();
    filters.retain(|filter| match &filter.kind {
        SuggestionKind::SenderIn(patterns) => seen.insert(patterns.clone()),
        SuggestionKind::HeaderMatch { .. } => true,
    });
    filters
}

/// The keep-priority of a suggestion, by its tag — the lower value is kept
/// when two rules collide on the same `sender_in` matcher.
fn tag_priority(tag: &str) -> u8 {
    match tag {
        "newsletter" => 0,
        "notification" => 1,
        "vip" => 2,
        _ => 3,
    }
}

/// An identifier-safe slug of `value`: lowercase ASCII alphanumerics, with
/// every run of other characters collapsed to a single `-` and no leading
/// or trailing `-`.
fn slug(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut pending_dash = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
            pending_dash = false;
        } else {
            pending_dash = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::analyzer::ScanReport;
    use crate::scan::analyzers::lists::MailingList;
    use crate::scan::analyzers::newsletters::NewsletterDomain;
    use crate::scan::analyzers::notifications::NotificationService;
    use crate::scan::analyzers::senders::DomainCount;
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
    fn suggest_proposes_a_sender_in_rule_for_a_busy_domain() {
        let mut report = empty_report();
        report.top_domains = vec![DomainCount {
            domain: "github.com".to_string(),
            messages_received: 40,
        }];
        let suggestions = suggest(&report, 5);
        assert_eq!(suggestions.filters.len(), 1);
        assert!(matches!(
            suggestions.filters[0].kind,
            SuggestionKind::SenderIn(_)
        ));
    }

    #[test]
    fn suggest_drops_entries_below_min_evidence() {
        let mut report = empty_report();
        report.top_domains = vec![DomainCount {
            domain: "rare.test".to_string(),
            messages_received: 3,
        }];
        assert!(suggest(&report, 5).filters.is_empty());
    }

    #[test]
    fn suggest_proposes_a_header_match_for_a_mailing_list() {
        let mut report = empty_report();
        report.mailing_lists = vec![MailingList {
            list_id: "rust-users.rust-lang.org".to_string(),
            messages: 20,
        }];
        let suggestions = suggest(&report, 5);
        assert_eq!(suggestions.filters.len(), 1);
        assert!(matches!(
            &suggestions.filters[0].kind,
            SuggestionKind::HeaderMatch { header, .. } if header == "List-Id"
        ));
    }

    #[test]
    fn suggest_proposes_a_spam_rule_when_enough_spam_is_flagged() {
        let mut report = empty_report();
        report.spam = SpamSummary {
            flagged: 12,
            high_score: 0,
            dmarc_failures: 0,
        };
        let suggestions = suggest(&report, 5);
        assert!(
            suggestions
                .filters
                .iter()
                .any(|filter| filter.tag == "spam")
        );
    }

    #[test]
    fn suggest_keeps_a_newsletter_domain() {
        let mut report = empty_report();
        report.newsletters = vec![NewsletterDomain {
            domain: "substack.com".to_string(),
            messages: 30,
            distinct_senders: 6,
        }];
        let suggestions = suggest(&report, 5);
        assert_eq!(suggestions.filters.len(), 1);
        assert_eq!(suggestions.filters[0].tag, "newsletter");
    }

    #[test]
    fn suggest_on_an_empty_report_proposes_nothing() {
        assert!(suggest(&empty_report(), 5).filters.is_empty());
    }

    #[test]
    fn a_suggestion_carries_a_positive_confidence_score() {
        let mut report = empty_report();
        report.top_domains = vec![DomainCount {
            domain: "x.test".to_string(),
            messages_received: 40,
        }];
        assert!(suggest(&report, 5).filters[0].confidence > 0.0);
    }

    #[test]
    fn suggest_consolidates_a_domain_seen_by_two_dimensions() {
        // The same domain surfaces as a top domain AND a notification
        // service; both would propose `sender_in: ["*@github.com"]`.
        let mut report = empty_report();
        report.top_domains = vec![DomainCount {
            domain: "github.com".to_string(),
            messages_received: 40,
        }];
        report.notification_services = vec![NotificationService {
            domain: "github.com".to_string(),
            messages: 40,
        }];
        let github_rules = suggest(&report, 5)
            .filters
            .iter()
            .filter(|filter| {
                matches!(
                    &filter.kind,
                    SuggestionKind::SenderIn(patterns)
                        if patterns.iter().any(|p| p.as_str() == "*@github.com")
                )
            })
            .count();
        assert_eq!(github_rules, 1);
    }
}
