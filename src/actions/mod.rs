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

//! Actions: the IMAP writeback engine — `ensure_folder_exists` and the
//! consolidated application of a message's per-tag actions (PRD §5.5).

pub mod error;

use std::future::Future;

use crate::actions::error::ActionError;

/// An IMAP message flag Berger can set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    /// `\Seen`.
    Seen,
    /// `\Flagged`.
    Flagged,
}

/// One IMAP action to apply to a message (PRD §5.5).
///
/// The folder of `CopyTo` / `MoveTo` is the logical path *below* `Berger/`,
/// `/`-separated (e.g. `notifs/github`); the engine prefixes `Berger/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Copy the message into `Berger/<folder>`.
    CopyTo(String),
    /// Move the message into `Berger/<folder>`.
    MoveTo(String),
    /// Mark the message `\Seen`.
    MarkSeen,
    /// Mark the message `\Flagged`.
    MarkFlagged,
}

/// The IMAP side Berger writes actions to.
///
/// Berger applies actions exclusively through this trait, so the engine's
/// logic can be unit-tested against an in-memory fake. Folder paths are
/// logical and `/`-separated; the implementation maps `/` onto the
/// server's hierarchy separator.
pub trait ActionTarget {
    /// Returns whether `folder` exists on the server.
    fn folder_exists(
        &mut self,
        folder: &str,
    ) -> impl Future<Output = Result<bool, ActionError>> + Send;

    /// Creates `folder` and subscribes to it (so it shows in mail clients).
    fn create_folder(
        &mut self,
        folder: &str,
    ) -> impl Future<Output = Result<(), ActionError>> + Send;

    /// Copies the message with this UID into `folder`.
    fn copy_message(
        &mut self,
        uid: u32,
        folder: &str,
    ) -> impl Future<Output = Result<(), ActionError>> + Send;

    /// Moves the message with this UID into `folder` (atomic `UID MOVE`).
    fn move_message(
        &mut self,
        uid: u32,
        folder: &str,
    ) -> impl Future<Output = Result<(), ActionError>> + Send;

    /// Adds `flag` to the message with this UID.
    fn add_flag(
        &mut self,
        uid: u32,
        flag: Flag,
    ) -> impl Future<Output = Result<(), ActionError>> + Send;
}

/// Ensures `folder` exists, creating (and subscribing) it if a user has
/// deleted it — Bichon coherence rule #3 (CLAUDE.md §3.2, PRD §5.11).
pub async fn ensure_folder_exists<T: ActionTarget>(
    target: &mut T,
    folder: &str,
) -> Result<(), ActionError> {
    if !target.folder_exists(folder).await? {
        tracing::warn!(folder = %folder, "folder was missing, recreating (likely user-deleted)");
        target.create_folder(folder).await?;
    }
    Ok(())
}

/// Applies a message's consolidated actions (PRD §5.5): flags and copies
/// run while the message is still in INBOX, the move runs last.
pub async fn apply_actions<T: ActionTarget>(
    target: &mut T,
    uid: u32,
    actions: &[Action],
) -> Result<(), ActionError> {
    let mut copies: Vec<String> = Vec::new();
    let mut moves: Vec<String> = Vec::new();
    let mut mark_seen = false;
    let mut mark_flagged = false;
    for action in actions {
        match action {
            Action::CopyTo(folder) => {
                if !copies.contains(folder) {
                    copies.push(folder.clone());
                }
            }
            Action::MoveTo(folder) => {
                if !moves.contains(folder) {
                    moves.push(folder.clone());
                }
            }
            Action::MarkSeen => mark_seen = true,
            Action::MarkFlagged => mark_flagged = true,
        }
    }

    // move_to wins over copy_to for the same folder (PRD §5.5).
    copies.retain(|folder| {
        if moves.contains(folder) {
            tracing::warn!(
                folder = %folder,
                "copy_to and move_to target the same folder; move_to wins"
            );
            false
        } else {
            true
        }
    });

    // Flags and copies run first — they need the message still in INBOX.
    if mark_seen {
        target.add_flag(uid, Flag::Seen).await?;
    }
    if mark_flagged {
        target.add_flag(uid, Flag::Flagged).await?;
    }
    for folder in &copies {
        let path = berger_folder(folder);
        ensure_folder_exists(target, &path).await?;
        target.copy_message(uid, &path).await?;
    }

    // The move runs last: UID MOVE removes the message from INBOX.
    if moves.len() > 1 {
        tracing::warn!(
            count = moves.len(),
            "several move_to targets; a message can move only once — using the first"
        );
    }
    if let Some(folder) = moves.first() {
        let path = berger_folder(folder);
        ensure_folder_exists(target, &path).await?;
        target.move_message(uid, &path).await?;
    }
    Ok(())
}

/// The full IMAP path of a Berger writeback folder.
fn berger_folder(folder: &str) -> String {
    format!("Berger/{folder}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// In-memory `ActionTarget`: records every operation and tracks which
    /// folders exist.
    struct FakeTarget {
        existing: HashSet<String>,
        calls: Vec<String>,
    }

    impl FakeTarget {
        fn new() -> Self {
            Self {
                existing: HashSet::new(),
                calls: Vec::new(),
            }
        }

        fn with_folders(folders: &[&str]) -> Self {
            Self {
                existing: folders.iter().map(|f| (*f).to_string()).collect(),
                calls: Vec::new(),
            }
        }
    }

    impl ActionTarget for FakeTarget {
        async fn folder_exists(&mut self, folder: &str) -> Result<bool, ActionError> {
            Ok(self.existing.contains(folder))
        }

        async fn create_folder(&mut self, folder: &str) -> Result<(), ActionError> {
            self.calls.push(format!("create:{folder}"));
            self.existing.insert(folder.to_string());
            Ok(())
        }

        async fn copy_message(&mut self, uid: u32, folder: &str) -> Result<(), ActionError> {
            self.calls.push(format!("copy:{uid}:{folder}"));
            Ok(())
        }

        async fn move_message(&mut self, uid: u32, folder: &str) -> Result<(), ActionError> {
            self.calls.push(format!("move:{uid}:{folder}"));
            Ok(())
        }

        async fn add_flag(&mut self, uid: u32, flag: Flag) -> Result<(), ActionError> {
            self.calls.push(format!("flag:{uid}:{flag:?}"));
            Ok(())
        }
    }

    #[tokio::test]
    async fn ensure_folder_creates_a_missing_folder() {
        let mut target = FakeTarget::new();
        ensure_folder_exists(&mut target, "Berger/cat-work")
            .await
            .unwrap();
        assert_eq!(target.calls, ["create:Berger/cat-work"]);
    }

    #[tokio::test]
    async fn ensure_folder_leaves_an_existing_folder_alone() {
        let mut target = FakeTarget::with_folders(&["Berger/cat-work"]);
        ensure_folder_exists(&mut target, "Berger/cat-work")
            .await
            .unwrap();
        assert!(target.calls.is_empty());
    }

    #[tokio::test]
    async fn copy_to_ensures_the_folder_then_copies() {
        let mut target = FakeTarget::new();
        apply_actions(&mut target, 7, &[Action::CopyTo("cat-work".to_string())])
            .await
            .unwrap();
        assert_eq!(
            target.calls,
            ["create:Berger/cat-work", "copy:7:Berger/cat-work"]
        );
    }

    #[tokio::test]
    async fn move_to_ensures_the_folder_then_moves() {
        let mut target = FakeTarget::new();
        apply_actions(&mut target, 9, &[Action::MoveTo("junk".to_string())])
            .await
            .unwrap();
        assert_eq!(target.calls, ["create:Berger/junk", "move:9:Berger/junk"]);
    }

    #[tokio::test]
    async fn marks_set_the_imap_flags() {
        let mut target = FakeTarget::new();
        apply_actions(&mut target, 4, &[Action::MarkSeen, Action::MarkFlagged])
            .await
            .unwrap();
        assert_eq!(target.calls, ["flag:4:Seen", "flag:4:Flagged"]);
    }

    #[tokio::test]
    async fn move_wins_over_copy_for_the_same_folder() {
        let mut target = FakeTarget::new();
        apply_actions(
            &mut target,
            1,
            &[
                Action::CopyTo("urgent".to_string()),
                Action::MoveTo("urgent".to_string()),
            ],
        )
        .await
        .unwrap();
        assert!(!target.calls.iter().any(|call| call.starts_with("copy:")));
        assert!(
            target
                .calls
                .iter()
                .any(|call| call == "move:1:Berger/urgent")
        );
    }

    #[tokio::test]
    async fn copies_and_flags_run_before_the_move() {
        let mut target = FakeTarget::new();
        apply_actions(
            &mut target,
            2,
            &[
                Action::MoveTo("junk".to_string()),
                Action::CopyTo("archive".to_string()),
                Action::MarkSeen,
            ],
        )
        .await
        .unwrap();
        let move_at = target
            .calls
            .iter()
            .position(|c| c.starts_with("move:"))
            .unwrap();
        let copy_at = target
            .calls
            .iter()
            .position(|c| c.starts_with("copy:"))
            .unwrap();
        let flag_at = target
            .calls
            .iter()
            .position(|c| c.starts_with("flag:"))
            .unwrap();
        assert!(copy_at < move_at, "copy must run before move");
        assert!(flag_at < move_at, "flag must run before move");
    }

    #[tokio::test]
    async fn duplicate_copy_actions_are_consolidated() {
        let mut target = FakeTarget::new();
        apply_actions(
            &mut target,
            3,
            &[
                Action::CopyTo("cat-work".to_string()),
                Action::CopyTo("cat-work".to_string()),
            ],
        )
        .await
        .unwrap();
        let copies = target
            .calls
            .iter()
            .filter(|c| c.starts_with("copy:"))
            .count();
        assert_eq!(copies, 1);
    }
}
