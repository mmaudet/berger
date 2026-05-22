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

//! Email-address extraction: turn a raw `From`/`To` header value into the
//! address — and domain — the scan groups senders by.

/// Extracts a lowercased email address from a raw `From`/`To`/`Cc` header
/// value such as `"Arnaud Clair <arnaud.clair@x.fr>"` or `"s@x.test"`.
///
/// Returns `None` when the value carries no recognisable address. The
/// value is treated as a single address: comma-separated lists are not
/// split here.
pub fn extract_address(raw: &str) -> Option<String> {
    let candidate = match (raw.find('<'), raw.find('>')) {
        (Some(open), Some(close)) if open < close => &raw[open + 1..close],
        _ => raw,
    };
    let address = candidate.trim().to_ascii_lowercase();
    is_address(&address).then_some(address)
}

/// Whether `value` has the minimal shape of an address: a non-empty local
/// part and a non-empty domain on either side of an `@`.
fn is_address(value: &str) -> bool {
    match value.rsplit_once('@') {
        Some((local, domain)) => !local.is_empty() && !domain.is_empty(),
        None => false,
    }
}

/// The domain of an email address — everything after the last `@` — or
/// `None` when `address` has no `@`, or an empty domain.
pub fn domain_of(address: &str) -> Option<&str> {
    match address.rsplit_once('@') {
        Some((_, domain)) if !domain.is_empty() => Some(domain),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_an_address_in_angle_brackets() {
        assert_eq!(
            extract_address("Arnaud Clair <arnaud.clair@interieur.gouv.fr>").as_deref(),
            Some("arnaud.clair@interieur.gouv.fr")
        );
    }

    #[test]
    fn extracts_a_bare_address() {
        assert_eq!(
            extract_address("s@example.test").as_deref(),
            Some("s@example.test")
        );
    }

    #[test]
    fn lowercases_the_address() {
        assert_eq!(
            extract_address("<NoReply@GitHub.COM>").as_deref(),
            Some("noreply@github.com")
        );
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(
            extract_address("   spaced@x.test  ").as_deref(),
            Some("spaced@x.test")
        );
    }

    #[test]
    fn handles_a_quoted_display_name_with_a_comma() {
        assert_eq!(
            extract_address("\"Doe, John\" <john@x.test>").as_deref(),
            Some("john@x.test")
        );
    }

    #[test]
    fn rejects_a_value_with_no_address() {
        assert_eq!(extract_address("No Email Here"), None);
        assert_eq!(extract_address(""), None);
    }

    #[test]
    fn rejects_an_address_missing_a_part() {
        assert_eq!(extract_address("@example.test"), None);
        assert_eq!(extract_address("local@"), None);
    }

    #[test]
    fn domain_of_returns_the_part_after_the_at_sign() {
        assert_eq!(
            domain_of("arnaud.clair@interieur.gouv.fr"),
            Some("interieur.gouv.fr")
        );
    }

    #[test]
    fn domain_of_returns_none_without_an_at_sign() {
        assert_eq!(domain_of("not-an-address"), None);
    }

    #[test]
    fn domain_of_returns_none_for_an_empty_domain() {
        assert_eq!(domain_of("local@"), None);
    }
}
