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

//! Read-side filter for the first Bichon coherence rule (CLAUDE.md §3.2):
//! Berger must ignore messages that live in its own writeback folders,
//! otherwise it loops forever on its own `copy_to` / `move_to` output.

/// Returns `true` when `mailbox` names one of Berger's own writeback
/// folders — the `Berger/*` tree, or `INBOX/Berger/*` on servers that
/// disallow top-level folders.
///
/// The IMAP hierarchy separator differs per server (`/` on Gmail, `.` on
/// dovecot / Apache James), so the check is separator-agnostic: it matches
/// the first path segment rather than a literal prefix. The `Berger`
/// segment is matched case-sensitively (Berger always creates it
/// capitalised); `INBOX` is matched case-insensitively per RFC 3501.
pub fn is_berger_folder(mailbox: &str) -> bool {
    let mut segments = mailbox
        .trim()
        .split(['/', '.'])
        .filter(|segment| !segment.is_empty());

    match segments.next() {
        Some("Berger") => true,
        Some(first) if first.eq_ignore_ascii_case("INBOX") => segments.next() == Some("Berger"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::is_berger_folder;

    #[test]
    fn recognizes_top_level_berger_folders() {
        assert!(is_berger_folder("Berger"));
        assert!(is_berger_folder("Berger/cat-work"));
        assert!(is_berger_folder("Berger.cat-work"));
        assert!(is_berger_folder("Berger/notifs/github"));
        assert!(is_berger_folder("Berger.notifs.github"));
    }

    #[test]
    fn recognizes_berger_nested_under_inbox() {
        assert!(is_berger_folder("INBOX/Berger/cat-work"));
        assert!(is_berger_folder("INBOX.Berger.cat-work"));
        assert!(is_berger_folder("INBOX/Berger"));
        assert!(is_berger_folder("inbox/Berger/x"));
        assert!(is_berger_folder("Inbox.Berger.x"));
    }

    #[test]
    fn ignores_ordinary_user_folders() {
        assert!(!is_berger_folder("INBOX"));
        assert!(!is_berger_folder("INBOX/Clients"));
        assert!(!is_berger_folder("900_Archives.2007.Clients.EDF"));
        assert!(!is_berger_folder("[Gmail]/Sent Mail"));
        assert!(!is_berger_folder("Newsletter"));
        assert!(!is_berger_folder(""));
    }

    #[test]
    fn does_not_match_folders_that_merely_start_with_berger() {
        assert!(!is_berger_folder("Bergerac"));
        assert!(!is_berger_folder("Berger Stuff"));
        assert!(!is_berger_folder("BergerBackup/x"));
        assert!(!is_berger_folder("berger/cat-work"));
    }

    #[test]
    fn tolerates_leading_separators_and_whitespace() {
        assert!(is_berger_folder("/Berger/cat-work"));
        assert!(is_berger_folder(".Berger.cat-work"));
        assert!(is_berger_folder("  Berger/cat-work  "));
        assert!(!is_berger_folder("   "));
    }
}
