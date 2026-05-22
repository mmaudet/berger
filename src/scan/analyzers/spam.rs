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

//! Spam-pattern analysis: dimension 7 of the scan (PRD v1.1 §4.2).
//!
//! Summarises the spam signals an upstream filter already left in the
//! inbox's headers — the `X-Spam-Flag` verdict, the `X-Spam-Score`, and
//! `DMARC` authentication failures — into a single [`SpamSummary`].

use crate::scan::analyzer::ScannedMessage;

/// SpamAssassin's default `required` threshold: a message scoring at or
/// above this is treated as spam.
const HIGH_SCORE_THRESHOLD: f64 = 5.0;

/// Dimension 7: a single inbox-wide tally of the spam signals already
/// present in the messages' headers.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct SpamSummary {
    /// Messages carrying `X-Spam-Flag: YES` (case-insensitive).
    pub flagged: usize,
    /// Messages whose `X-Spam-Score` reaches the spam threshold (5.0).
    pub high_score: usize,
    /// Messages whose `Authentication-Results` reports `dmarc=fail`.
    pub dmarc_failures: usize,
}

/// Dimension 7: tallies the spam signals across the whole `inbox` —
/// `X-Spam-Flag` verdicts, high `X-Spam-Score` values, and `DMARC`
/// failures — into one [`SpamSummary`]. There is no ranking or
/// truncation: the result is a single summary value.
pub fn analyze_spam(inbox: &[ScannedMessage]) -> SpamSummary {
    let mut summary = SpamSummary::default();
    for message in inbox {
        let headers = &message.headers;
        if headers
            .x_spam_flag
            .as_deref()
            .is_some_and(|flag| flag.trim().eq_ignore_ascii_case("yes"))
        {
            summary.flagged += 1;
        }
        if headers
            .x_spam_score
            .is_some_and(|score| score >= HIGH_SCORE_THRESHOLD)
        {
            summary.high_score += 1;
        }
        if headers
            .authentication_results
            .as_deref()
            .is_some_and(|results| results.to_ascii_lowercase().contains("dmarc=fail"))
        {
            summary.dmarc_failures += 1;
        }
    }
    summary
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
            mailbox_name: Some("INBOX".to_string()),
            uid: 1,
            subject: String::new(),
            preview: String::new(),
            from: "sender@x.test".to_string(),
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

    fn scanned(envelope: &Envelope, headers: ScanHeaders) -> ScannedMessage<'_> {
        ScannedMessage { envelope, headers }
    }

    #[test]
    fn counts_messages_flagged_as_spam() {
        let env = envelope();
        let inbox = [
            scanned(
                &env,
                ScanHeaders {
                    x_spam_flag: Some("YES".to_string()),
                    ..ScanHeaders::default()
                },
            ),
            scanned(
                &env,
                ScanHeaders {
                    // Trimmed and lowercased still matches.
                    x_spam_flag: Some("  yes  ".to_string()),
                    ..ScanHeaders::default()
                },
            ),
            scanned(
                &env,
                ScanHeaders {
                    x_spam_flag: Some("NO".to_string()),
                    ..ScanHeaders::default()
                },
            ),
            scanned(&env, ScanHeaders::default()),
        ];
        assert_eq!(analyze_spam(&inbox).flagged, 2);
    }

    #[test]
    fn high_score_uses_the_spamassassin_threshold() {
        let env = envelope();
        let inbox = [
            scanned(
                &env,
                ScanHeaders {
                    // Exactly at the threshold counts.
                    x_spam_score: Some(5.0),
                    ..ScanHeaders::default()
                },
            ),
            scanned(
                &env,
                ScanHeaders {
                    x_spam_score: Some(9.3),
                    ..ScanHeaders::default()
                },
            ),
            scanned(
                &env,
                ScanHeaders {
                    // Just below the threshold does not count.
                    x_spam_score: Some(4.9),
                    ..ScanHeaders::default()
                },
            ),
            scanned(
                &env,
                ScanHeaders {
                    x_spam_score: Some(-2.6),
                    ..ScanHeaders::default()
                },
            ),
            scanned(&env, ScanHeaders::default()),
        ];
        assert_eq!(analyze_spam(&inbox).high_score, 2);
    }

    #[test]
    fn counts_dmarc_authentication_failures() {
        let env = envelope();
        let inbox = [
            scanned(
                &env,
                ScanHeaders {
                    authentication_results: Some(
                        "mx.x.test; spf=pass; dmarc=fail (p=none)".to_string(),
                    ),
                    ..ScanHeaders::default()
                },
            ),
            scanned(
                &env,
                ScanHeaders {
                    // Case-insensitive match on the substring.
                    authentication_results: Some("mx.x.test; DMARC=FAIL".to_string()),
                    ..ScanHeaders::default()
                },
            ),
            scanned(
                &env,
                ScanHeaders {
                    authentication_results: Some("mx.x.test; dmarc=pass".to_string()),
                    ..ScanHeaders::default()
                },
            ),
            scanned(&env, ScanHeaders::default()),
        ];
        assert_eq!(analyze_spam(&inbox).dmarc_failures, 2);
    }

    #[test]
    fn a_clean_inbox_yields_the_zero_summary() {
        let env = envelope();
        let inbox = [
            scanned(
                &env,
                ScanHeaders {
                    x_spam_flag: Some("No".to_string()),
                    x_spam_score: Some(0.1),
                    authentication_results: Some("mx.x.test; dmarc=pass".to_string()),
                    ..ScanHeaders::default()
                },
            ),
            scanned(&env, ScanHeaders::default()),
        ];
        assert_eq!(analyze_spam(&inbox), SpamSummary::default());
    }

    #[test]
    fn an_empty_inbox_yields_the_zero_summary() {
        assert_eq!(analyze_spam(&[]), SpamSummary::default());
    }

    #[test]
    fn tallies_every_dimension_at_once() {
        let env = envelope();
        let inbox = [scanned(
            &env,
            ScanHeaders {
                x_spam_flag: Some("YES".to_string()),
                x_spam_score: Some(8.0),
                authentication_results: Some("mx.x.test; dmarc=fail".to_string()),
                ..ScanHeaders::default()
            },
        )];
        assert_eq!(
            analyze_spam(&inbox),
            SpamSummary {
                flagged: 1,
                high_score: 1,
                dmarc_failures: 1,
            }
        );
    }
}
