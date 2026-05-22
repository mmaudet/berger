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

//! Technical-header extraction for the scan.
//!
//! [`parse_headers`] reads only the message headers the scan needs —
//! `List-*`, `X-Spam-*`, `Auto-Submitted`, `Precedence`,
//! `Authentication-Results` (PRD v1.1 §4.4). It never touches the message
//! body: no `body_text` / `body_html` call appears in this module, nor
//! anywhere else in `src/scan/`.

use mail_parser::MessageParser;

/// The technical headers the scan reads from a message, for dimensions
/// 4-7. An absent header leaves its field at the [`Default`] — `false`
/// or `None`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScanHeaders {
    /// Whether a `List-Unsubscribe` header is present (dimension 4).
    pub list_unsubscribe: bool,
    /// The raw `List-Id` header value, if present (dimension 5).
    pub list_id: Option<String>,
    /// The `Auto-Submitted` header value, if present (dimension 6).
    pub auto_submitted: Option<String>,
    /// The `Precedence` header value, if present (dimension 6).
    pub precedence: Option<String>,
    /// The `X-Spam-Flag` header value, if present (dimension 7).
    pub x_spam_flag: Option<String>,
    /// The spam score, parsed from `X-Spam-Score` or the `score=` field of
    /// `X-Spam-Status` (dimension 7).
    pub x_spam_score: Option<f64>,
    /// The `Authentication-Results` header value, if present (dimension 7).
    pub authentication_results: Option<String>,
}

/// Parses the technical headers of a raw RFC 822 message.
///
/// An unparseable message yields [`ScanHeaders::default`]. The message
/// body is never read.
pub fn parse_headers(eml: &[u8]) -> ScanHeaders {
    let Some(message) = MessageParser::default().parse(eml) else {
        return ScanHeaders::default();
    };
    let header = |name: &str| {
        message
            .header_raw(name)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    ScanHeaders {
        list_unsubscribe: header("List-Unsubscribe").is_some(),
        list_id: header("List-Id"),
        auto_submitted: header("Auto-Submitted"),
        precedence: header("Precedence"),
        x_spam_flag: header("X-Spam-Flag"),
        x_spam_score: parse_spam_score(
            message.header_raw("X-Spam-Score"),
            message.header_raw("X-Spam-Status"),
        ),
        authentication_results: header("Authentication-Results"),
    }
}

/// Extracts a numeric spam score: `X-Spam-Score` parsed directly when
/// present, otherwise the `score=` field embedded in `X-Spam-Status`.
fn parse_spam_score(score: Option<&str>, status: Option<&str>) -> Option<f64> {
    if let Some(value) = score
        && let Ok(parsed) = value.trim().parse::<f64>()
    {
        return Some(parsed);
    }
    let after = status?.split("score=").nth(1)?;
    let number: String = after
        .chars()
        .take_while(|c| c.is_ascii_digit() || ['.', '-', '+'].contains(c))
        .collect();
    number.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_list_headers() {
        let eml = b"From: a@x.test\r\nList-Unsubscribe: <mailto:u@x.test>\r\nList-Id: Rust Users <rust-users.rust-lang.org>\r\nSubject: s\r\n\r\nbody\r\n";
        let headers = parse_headers(eml);
        assert!(headers.list_unsubscribe);
        assert_eq!(
            headers.list_id.as_deref(),
            Some("Rust Users <rust-users.rust-lang.org>")
        );
    }

    #[test]
    fn parses_notification_headers() {
        let eml = b"From: noreply@x.test\r\nAuto-Submitted: auto-generated\r\nPrecedence: bulk\r\nSubject: s\r\n\r\nbody\r\n";
        let headers = parse_headers(eml);
        assert_eq!(headers.auto_submitted.as_deref(), Some("auto-generated"));
        assert_eq!(headers.precedence.as_deref(), Some("bulk"));
    }

    #[test]
    fn parses_the_spam_flag_and_score_from_status() {
        let eml = b"From: a@x.test\r\nX-Spam-Flag: YES\r\nX-Spam-Status: Yes, score=7.1 required=5.0 tests=BAYES\r\nSubject: s\r\n\r\nbody\r\n";
        let headers = parse_headers(eml);
        assert_eq!(headers.x_spam_flag.as_deref(), Some("YES"));
        assert_eq!(headers.x_spam_score, Some(7.1));
    }

    #[test]
    fn parses_a_negative_x_spam_score_header() {
        let eml = b"From: a@x.test\r\nX-Spam-Score: -2.6\r\nSubject: s\r\n\r\nbody\r\n";
        let headers = parse_headers(eml);
        assert_eq!(headers.x_spam_score, Some(-2.6));
    }

    #[test]
    fn parses_authentication_results() {
        let eml = b"From: a@x.test\r\nAuthentication-Results: mx.x.test; dmarc=fail\r\nSubject: s\r\n\r\nbody\r\n";
        let headers = parse_headers(eml);
        assert!(
            headers
                .authentication_results
                .as_deref()
                .unwrap_or_default()
                .contains("dmarc=fail")
        );
    }

    #[test]
    fn a_plain_message_has_no_technical_headers() {
        let eml = b"From: a@x.test\r\nSubject: hello\r\n\r\njust a body\r\n";
        assert_eq!(parse_headers(eml), ScanHeaders::default());
    }

    #[test]
    fn an_empty_eml_yields_default_headers() {
        assert_eq!(parse_headers(b""), ScanHeaders::default());
    }
}
