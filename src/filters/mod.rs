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

//! Native (non-LLM) filters — the four declarative filter types of
//! PRD §5.2: `list_unsubscribe`, `sender_in`, `subject_regex`,
//! `header_match`.

pub mod error;

use regex::Regex;

use crate::filters::error::FilterError;

/// The slice of a message the native filters examine.
#[derive(Debug, Clone)]
pub struct MessageView {
    /// The raw `From` header value (display name and/or address).
    pub from: String,
    /// The `Subject` header value.
    pub subject: String,
    /// Raw RFC 822 headers as `(name, value)` pairs. Names are matched
    /// case-insensitively; a header name may appear more than once.
    pub headers: Vec<(String, String)>,
}

/// One native, declaratively-configured filter (PRD §5.2).
#[derive(Debug)]
pub enum NativeFilter {
    /// Matches when a `List-Unsubscribe` header is present.
    ListUnsubscribe,
    /// Matches when the sender's address — or its domain — is in this set.
    SenderIn { senders: Vec<String> },
    /// Matches when the subject matches this regex.
    SubjectRegex { pattern: Regex },
    /// Matches when a value of the named header matches this regex.
    HeaderMatch { header: String, pattern: Regex },
}

impl NativeFilter {
    /// Builds a `list_unsubscribe` filter.
    pub fn list_unsubscribe() -> Self {
        NativeFilter::ListUnsubscribe
    }

    /// Builds a `sender_in` filter over a set of addresses and/or domains.
    pub fn sender_in(senders: Vec<String>) -> Self {
        NativeFilter::SenderIn { senders }
    }

    /// Builds a `subject_regex` filter.
    ///
    /// # Errors
    /// Returns [`FilterError`] if `pattern` is not a valid regex.
    pub fn subject_regex(pattern: &str) -> Result<Self, FilterError> {
        Ok(NativeFilter::SubjectRegex {
            pattern: compile(pattern)?,
        })
    }

    /// Builds a `header_match` filter against the named header.
    ///
    /// # Errors
    /// Returns [`FilterError`] if `pattern` is not a valid regex.
    pub fn header_match(header: &str, pattern: &str) -> Result<Self, FilterError> {
        Ok(NativeFilter::HeaderMatch {
            header: header.to_string(),
            pattern: compile(pattern)?,
        })
    }

    /// Returns whether `message` matches this filter.
    pub fn matches(&self, message: &MessageView) -> bool {
        match self {
            NativeFilter::ListUnsubscribe => message
                .headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("List-Unsubscribe")),
            NativeFilter::SenderIn { senders } => sender_in_matches(&message.from, senders),
            NativeFilter::SubjectRegex { pattern } => pattern.is_match(&message.subject),
            NativeFilter::HeaderMatch { header, pattern } => message
                .headers
                .iter()
                .any(|(name, value)| name.eq_ignore_ascii_case(header) && pattern.is_match(value)),
        }
    }
}

/// Compiles a regex, wrapping a failure in [`FilterError::InvalidRegex`].
fn compile(pattern: &str) -> Result<Regex, FilterError> {
    Regex::new(pattern).map_err(|source| FilterError::InvalidRegex {
        pattern: pattern.to_string(),
        source,
    })
}

/// Extracts the bare, lower-cased e-mail address from a `From` value such
/// as `"Display Name <addr@host>"` or a plain `addr@host`.
fn extract_address(from: &str) -> String {
    let address = match (from.rfind('<'), from.rfind('>')) {
        (Some(open), Some(close)) if open < close => &from[open + 1..close],
        _ => from,
    };
    address.trim().to_ascii_lowercase()
}

/// Returns whether `from`'s address — or its domain — is listed in `senders`.
fn sender_in_matches(from: &str, senders: &[String]) -> bool {
    let address = extract_address(from);
    let domain = address.rsplit('@').next().unwrap_or_default();
    senders.iter().any(|sender| {
        let sender = sender.trim().to_ascii_lowercase();
        !sender.is_empty() && (sender == address || sender == domain)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(from: &str, subject: &str, headers: &[(&str, &str)]) -> MessageView {
        MessageView {
            from: from.to_string(),
            subject: subject.to_string(),
            headers: headers
                .iter()
                .map(|&(name, value)| (name.to_string(), value.to_string()))
                .collect(),
        }
    }

    #[test]
    fn list_unsubscribe_matches_when_the_header_is_present() {
        let filter = NativeFilter::list_unsubscribe();
        assert!(filter.matches(&msg("a@x", "s", &[("List-Unsubscribe", "<mailto:u@x>")])));
        // header-name matching is case-insensitive
        assert!(filter.matches(&msg("a@x", "s", &[("list-unsubscribe", "<mailto:u@x>")])));
    }

    #[test]
    fn list_unsubscribe_does_not_match_when_the_header_is_absent() {
        let filter = NativeFilter::list_unsubscribe();
        assert!(!filter.matches(&msg("a@x", "s", &[("Subject", "s")])));
    }

    #[test]
    fn sender_in_matches_a_full_address() {
        let filter = NativeFilter::sender_in(vec!["notifications@github.com".to_string()]);
        assert!(filter.matches(&msg("notifications@github.com", "s", &[])));
        assert!(filter.matches(&msg("GitHub <Notifications@GitHub.com>", "s", &[])));
    }

    #[test]
    fn sender_in_matches_a_bare_domain() {
        let filter = NativeFilter::sender_in(vec!["github.com".to_string()]);
        assert!(filter.matches(&msg("noreply@github.com", "s", &[])));
        assert!(!filter.matches(&msg("someone@gitlab.com", "s", &[])));
    }

    #[test]
    fn sender_in_does_not_match_an_unlisted_sender() {
        let filter = NativeFilter::sender_in(vec!["github.com".to_string()]);
        assert!(!filter.matches(&msg("Arnaud <arnaud@interieur.gouv.fr>", "s", &[])));
    }

    #[test]
    fn subject_regex_matches_the_subject() {
        let filter = NativeFilter::subject_regex(r"(?i)facture").unwrap();
        assert!(filter.matches(&msg("a@x", "Votre Facture du mois", &[])));
        assert!(!filter.matches(&msg("a@x", "Bonjour", &[])));
    }

    #[test]
    fn subject_regex_rejects_an_invalid_pattern() {
        assert!(matches!(
            NativeFilter::subject_regex("[unclosed"),
            Err(FilterError::InvalidRegex { .. })
        ));
    }

    #[test]
    fn header_match_matches_a_header_value() {
        let filter = NativeFilter::header_match("X-Spam-Flag", "(?i)yes").unwrap();
        assert!(filter.matches(&msg("a@x", "s", &[("X-Spam-Flag", "YES")])));
        // header-name lookup is case-insensitive
        assert!(filter.matches(&msg("a@x", "s", &[("x-spam-flag", "yes")])));
    }

    #[test]
    fn header_match_does_not_match_when_absent_or_different() {
        let filter = NativeFilter::header_match("Auto-Submitted", "auto-replied").unwrap();
        assert!(!filter.matches(&msg("a@x", "s", &[])));
        assert!(!filter.matches(&msg("a@x", "s", &[("Auto-Submitted", "no")])));
    }

    #[test]
    fn header_match_rejects_an_invalid_pattern() {
        assert!(matches!(
            NativeFilter::header_match("X-Test", "(unclosed"),
            Err(FilterError::InvalidRegex { .. })
        ));
    }
}
