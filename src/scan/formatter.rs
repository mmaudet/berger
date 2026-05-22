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

//! Scan output formatting (PRD v1.1 §4.3): the human-readable text report,
//! and the suggested-configuration YAML.
//!
//! The YAML emits only v1.0 `filters:` entries (`sender_in`,
//! `header_match`), so it can be merged straight into a real `berger.yaml`
//! — the rich evidence and confidence travel as comments above each rule.

use crate::scan::analyzer::ScanReport;
use crate::scan::suggester::{SuggestionKind, Suggestions};

/// How many rows per section the text report prints.
const TEXT_ROWS: usize = 15;

/// Renders the human-readable scan report (PRD v1.1 §4.3, output 2).
pub fn render_text(report: &ScanReport, suggestions: &Suggestions, period_days: u32) -> String {
    let rule = "=".repeat(78);
    let mut out = String::new();
    out.push_str(&format!("{rule}\nBERGER SCAN REPORT\n{rule}\n\n"));
    out.push_str(&format!("Period analyzed : last {period_days} days\n"));
    out.push_str(&format!(
        "Messages        : {} ({} inbox, {} sent)\n",
        report.messages_analyzed, report.inbox_messages, report.sent_messages
    ));
    out.push_str("Read-only — no IMAP action, no LLM call, no message body read.\n");

    section(&mut out, "TOP SENDERS");
    for sender in report.top_senders.iter().take(TEXT_ROWS) {
        out.push_str(&format!(
            "  {:>6}  {}\n",
            sender.messages_received, sender.address
        ));
    }

    section(&mut out, "TOP DOMAINS");
    for domain in report.top_domains.iter().take(TEXT_ROWS) {
        out.push_str(&format!(
            "  {:>6}  {}\n",
            domain.messages_received, domain.domain
        ));
    }

    section(&mut out, "BIDIRECTIONAL CONTACTS");
    for contact in report.bidirectional.iter().take(TEXT_ROWS) {
        out.push_str(&format!(
            "  {:>4} recv / {:>4} sent  {}\n",
            contact.messages_received, contact.messages_sent_to, contact.address
        ));
    }

    section(&mut out, "NEWSLETTERS");
    for newsletter in report.newsletters.iter().take(TEXT_ROWS) {
        out.push_str(&format!(
            "  {:>6}  {} ({} senders)\n",
            newsletter.messages, newsletter.domain, newsletter.distinct_senders
        ));
    }

    section(&mut out, "MAILING LISTS");
    for list in report.mailing_lists.iter().take(TEXT_ROWS) {
        out.push_str(&format!("  {:>6}  {}\n", list.messages, list.list_id));
    }

    section(&mut out, "NOTIFICATION SERVICES");
    for service in report.notification_services.iter().take(TEXT_ROWS) {
        out.push_str(&format!("  {:>6}  {}\n", service.messages, service.domain));
    }

    section(&mut out, "SPAM SIGNALS");
    out.push_str(&format!(
        "  {} flagged · {} high score · {} DMARC failures\n",
        report.spam.flagged, report.spam.high_score, report.spam.dmarc_failures
    ));

    section(&mut out, "SUBJECT PATTERNS");
    for ngram in report.subject_ngrams.iter().take(TEXT_ROWS) {
        out.push_str(&format!("  {:>6}  {}\n", ngram.occurrences, ngram.phrase));
    }

    section(&mut out, "LANGUAGES");
    for share in &report.languages {
        out.push_str(&format!(
            "  {:>5.1}%  {}\n",
            share.share * 100.0,
            share.language
        ));
    }

    section(&mut out, "HOURLY VOLUME");
    out.push_str(&format!(
        "  busiest hour: {:02}h00 UTC ({} messages)\n",
        report.volume.busiest_hour, report.volume.peak_hour_messages
    ));

    section(&mut out, "SUGGESTIONS");
    out.push_str(&format!(
        "  {} filter rule(s) suggested — review the YAML output before merging.\n",
        suggestions.filters.len()
    ));

    out.push_str(&format!("{rule}\n"));
    out
}

/// Appends a `--- TITLE ---…` section header to `out`.
fn section(out: &mut String, title: &str) {
    out.push_str(&format!("\n--- {title} "));
    let filled = title.len() + 5;
    if filled < 78 {
        out.push_str(&"-".repeat(78 - filled));
    }
    out.push('\n');
}

/// Renders the suggested-configuration YAML (PRD v1.1 §4.3, output 1): a
/// `filters:` block of v1.0 filter rules, each annotated with its evidence
/// and confidence as comments, ready to be reviewed and merged.
pub fn render_yaml(report: &ScanReport, suggestions: &Suggestions, period_days: u32) -> String {
    let bar = format!("# {}", "=".repeat(76));
    let mut out = String::new();
    out.push_str(&format!("{bar}\n"));
    out.push_str("# Berger scan — suggested configuration\n");
    out.push_str(&format!(
        "# Period: last {period_days} days · {} messages analyzed\n",
        report.messages_analyzed
    ));
    out.push_str("# Review each rule, then merge the ones you want into the `filters:`\n");
    out.push_str("# section of your berger.yaml. Nothing here is applied automatically.\n");
    out.push_str(&format!("{bar}\n\n"));

    if suggestions.filters.is_empty() {
        out.push_str("filters: []\n");
        return out;
    }

    out.push_str("filters:\n");
    for filter in &suggestions.filters {
        out.push_str(&format!(
            "  # {}  ·  confidence {:.2}\n",
            filter.name, filter.confidence
        ));
        out.push_str(&format!("  # {}\n", filter.rationale));
        match &filter.kind {
            SuggestionKind::SenderIn(patterns) => {
                out.push_str("  - sender_in:\n");
                for pattern in patterns {
                    out.push_str(&format!("      - {}\n", yaml_quote(pattern)));
                }
                out.push_str(&format!("    tag: {}\n", yaml_quote(&filter.tag)));
            }
            SuggestionKind::HeaderMatch { header, pattern } => {
                out.push_str("  - header_match:\n");
                out.push_str(&format!("      header: {}\n", yaml_quote(header)));
                out.push_str(&format!("      pattern: {}\n", yaml_quote(pattern)));
                out.push_str(&format!("    tag: {}\n", yaml_quote(&filter.tag)));
            }
            SuggestionKind::ListUnsubscribe => {
                out.push_str("  - list_unsubscribe: true\n");
                out.push_str(&format!("    tag: {}\n", yaml_quote(&filter.tag)));
            }
        }
        out.push('\n');
    }
    out
}

/// Wraps `value` as a YAML double-quoted scalar, escaping `\` and `"`.
fn yaml_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        if ch == '"' || ch == '\\' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

/// Renders the scan as a JSON document (PRD v1.1 §4.3, output 3) — the
/// machine-readable form, for third-party integration.
pub fn render_json(report: &ScanReport, suggestions: &Suggestions, period_days: u32) -> String {
    let document = serde_json::json!({
        "period_days": period_days,
        "report": report,
        "suggestions": suggestions.filters,
    });
    serde_json::to_string_pretty(&document).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BergerConfig;
    use crate::scan::analyzer::ScanReport;
    use crate::scan::analyzers::language::LanguageShare;
    use crate::scan::analyzers::senders::SenderCount;
    use crate::scan::analyzers::spam::SpamSummary;
    use crate::scan::analyzers::subjects::SubjectNgram;
    use crate::scan::analyzers::volume::VolumeProfile;
    use crate::scan::suggester::SuggestedFilter;

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

    fn sample_suggestions() -> Suggestions {
        Suggestions {
            filters: vec![
                SuggestedFilter {
                    name: "scan-domain-github-com".to_string(),
                    kind: SuggestionKind::SenderIn(vec!["*@github.com".to_string()]),
                    tag: "scan/github-com".to_string(),
                    evidence_messages: 284,
                    confidence: 0.92,
                    rationale: "284 messages received from this domain".to_string(),
                },
                SuggestedFilter {
                    name: "scan-spam-confirmed".to_string(),
                    kind: SuggestionKind::HeaderMatch {
                        header: "X-Spam-Flag".to_string(),
                        pattern: "(?i)yes".to_string(),
                    },
                    tag: "spam".to_string(),
                    evidence_messages: 31,
                    confidence: 0.78,
                    rationale: "31 messages flagged by the upstream filter".to_string(),
                },
            ],
        }
    }

    #[test]
    fn text_report_states_the_period_and_volume() {
        let mut report = empty_report();
        report.messages_analyzed = 7847;
        report.inbox_messages = 7204;
        report.sent_messages = 643;
        let text = render_text(&report, &Suggestions::default(), 30);
        assert!(text.contains("30"));
        assert!(text.contains("7847"));
        assert!(text.contains("7204 inbox"));
    }

    #[test]
    fn text_report_lists_top_senders() {
        let mut report = empty_report();
        report.top_senders = vec![SenderCount {
            address: "noreply@github.com".to_string(),
            messages_received: 284,
        }];
        let text = render_text(&report, &Suggestions::default(), 30);
        assert!(text.contains("noreply@github.com"));
    }

    #[test]
    fn text_report_shows_the_spam_summary() {
        let mut report = empty_report();
        report.spam = SpamSummary {
            flagged: 12,
            high_score: 31,
            dmarc_failures: 8,
        };
        let text = render_text(&report, &Suggestions::default(), 30);
        assert!(text.contains("12 flagged"));
    }

    #[test]
    fn text_report_states_it_is_read_only() {
        let text = render_text(&empty_report(), &Suggestions::default(), 30);
        assert!(text.contains("no IMAP action"));
        assert!(text.contains("no LLM"));
    }

    #[test]
    fn yaml_emits_a_filters_block() {
        let yaml = render_yaml(&empty_report(), &sample_suggestions(), 30);
        assert!(yaml.contains("filters:"));
        assert!(yaml.contains("sender_in"));
        assert!(yaml.contains("header_match"));
    }

    #[test]
    fn yaml_carries_confidence_and_rationale_as_comments() {
        let yaml = render_yaml(&empty_report(), &sample_suggestions(), 30);
        assert!(yaml.contains("confidence 0.92"));
        assert!(yaml.contains("# 284 messages received from this domain"));
    }

    #[test]
    fn yaml_quotes_a_star_sender_pattern() {
        let yaml = render_yaml(&empty_report(), &sample_suggestions(), 30);
        assert!(yaml.contains("\"*@github.com\""));
    }

    #[test]
    fn yaml_output_parses_with_the_existing_config_loader() {
        // PRD v1.1 §7: the suggested YAML's filter rules must be valid for
        // the v1.0 config loader.
        let yaml = render_yaml(&empty_report(), &sample_suggestions(), 30);
        let config_text = format!(
            "bichon:\n  base_url: \"https://b.test\"\n  api_token: \"t\"\n\
             database:\n  path: \"berger.db\"\n\
             accounts:\n  - name: \"A\"\n    bichon_account_id: \"1\"\n    imap:\n      host: \"h\"\n      user: \"u\"\n      password: \"p\"\n\
             {yaml}"
        );
        let config = BergerConfig::parse(&config_text).expect("suggested YAML must parse");
        assert_eq!(config.filters.len(), 2);
    }

    #[test]
    fn yaml_with_no_suggestions_is_an_empty_filters_list() {
        let yaml = render_yaml(&empty_report(), &Suggestions::default(), 30);
        assert!(yaml.contains("filters: []"));
    }

    #[test]
    fn text_report_shows_the_envelope_dimensions() {
        let mut report = empty_report();
        report.subject_ngrams = vec![SubjectNgram {
            phrase: "weekly report".to_string(),
            occurrences: 12,
        }];
        report.languages = vec![LanguageShare {
            language: "fra".to_string(),
            share: 0.8,
        }];
        report.volume = VolumeProfile {
            hourly: vec![0; 24],
            busiest_hour: 9,
            peak_hour_messages: 30,
        };
        let text = render_text(&report, &Suggestions::default(), 30);
        assert!(text.contains("weekly report"));
        assert!(text.contains("fra"));
        assert!(text.contains("busiest hour"));
    }

    #[test]
    fn json_output_is_valid_and_carries_the_scan() {
        let json = render_json(&empty_report(), &sample_suggestions(), 30);
        let value: serde_json::Value =
            serde_json::from_str(&json).expect("render_json must produce valid JSON");
        assert_eq!(value["period_days"], 30);
        assert!(value["report"].is_object());
        assert_eq!(value["suggestions"].as_array().map(Vec::len), Some(2));
    }
}
