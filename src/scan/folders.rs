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

//! Folder classification: which part of the mailbox an envelope came
//! from, from the scan's point of view (PRD v1.1 §4.2, §5.4).

use crate::ingest::folder_filter::is_berger_folder;

/// Which part of the mailbox an envelope was found in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderClass {
    /// The INBOX — received mail, the scan's main subject.
    Inbox,
    /// A Sent folder — the user's outgoing mail, used for the
    /// bidirectional dimension.
    Sent,
    /// Anything else — archives, drafts, spam, and Berger's own writeback
    /// folders. Never counted.
    Other,
}

/// Classifies the folder named `mailbox` for the scan.
///
/// Berger's own `Berger/*` folders are always [`FolderClass::Other`]: the
/// scan never counts its own triage output (CLAUDE.md §3.2 rule 1). The
/// INBOX is matched case-insensitively (RFC 3501); Sent folders by a small
/// set of common leaf names across mail clients and languages.
pub fn classify_folder(mailbox: &str) -> FolderClass {
    if is_berger_folder(mailbox) {
        return FolderClass::Other;
    }
    if mailbox.trim().eq_ignore_ascii_case("INBOX") {
        return FolderClass::Inbox;
    }
    if is_sent_folder(folder_leaf(mailbox)) {
        return FolderClass::Sent;
    }
    FolderClass::Other
}

/// Leaf names (lowercase) of folders that hold the user's outgoing mail,
/// across common mail clients and languages.
const SENT_FOLDER_NAMES: &[&str] = &[
    "sent",
    "sent mail",
    "sent items",
    "sent messages",
    "envoyés",
    "éléments envoyés",
    "messages envoyés",
    "brouillons envoyés",
];

/// Whether `leaf` (a folder's last path segment) names a Sent folder.
fn is_sent_folder(leaf: &str) -> bool {
    let leaf = leaf.trim().to_lowercase();
    SENT_FOLDER_NAMES.contains(&leaf.as_str())
}

/// The last path segment of an IMAP folder path, separator-agnostic
/// (`/` on Gmail, `.` on dovecot / Apache James).
fn folder_leaf(mailbox: &str) -> &str {
    mailbox
        .trim()
        .rsplit(['/', '.'])
        .find(|segment| !segment.trim().is_empty())
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inbox_is_received() {
        assert_eq!(classify_folder("INBOX"), FolderClass::Inbox);
        assert_eq!(classify_folder("inbox"), FolderClass::Inbox);
    }

    #[test]
    fn common_sent_folders_are_classified_as_sent() {
        assert_eq!(classify_folder("Sent"), FolderClass::Sent);
        assert_eq!(classify_folder("[Gmail]/Sent Mail"), FolderClass::Sent);
        assert_eq!(classify_folder("INBOX.Sent"), FolderClass::Sent);
        assert_eq!(classify_folder("Éléments envoyés"), FolderClass::Sent);
    }

    #[test]
    fn berger_folders_are_always_other() {
        assert_eq!(classify_folder("Berger/cat-work"), FolderClass::Other);
        assert_eq!(classify_folder("INBOX.Berger.junk"), FolderClass::Other);
        // Berger's own folders win even when named like a Sent folder.
        assert_eq!(classify_folder("Berger/Sent"), FolderClass::Other);
    }

    #[test]
    fn archives_drafts_and_unknown_folders_are_other() {
        assert_eq!(classify_folder("Archives"), FolderClass::Other);
        assert_eq!(classify_folder("INBOX/Clients"), FolderClass::Other);
        assert_eq!(classify_folder("Trash"), FolderClass::Other);
        assert_eq!(classify_folder(""), FolderClass::Other);
    }
}
