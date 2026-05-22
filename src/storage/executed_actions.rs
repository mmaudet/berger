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

//! Repository for the `executed_actions` table — the IMAP actions Berger
//! applied to a message (PRD §5.9).

use crate::storage::error::StorageError;

/// An IMAP action Berger executed for a message, to record in
/// `executed_actions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutedAction {
    /// RFC 822 Message-ID of the message acted on.
    pub message_id: String,
    /// The action — `copy_to`, `move_to`, `mark_seen` or `mark_flagged`.
    pub action_type: String,
    /// The destination folder, for `copy_to` / `move_to`; `None` for flags.
    pub target: Option<String>,
    /// Whether the action succeeded.
    pub succeeded: bool,
    /// The failure message, when the action failed.
    pub error: Option<String>,
}

/// Write access to the `executed_actions` table — one row per IMAP action.
pub struct ExecutedActionRepository<'a> {
    conn: &'a rusqlite::Connection,
}

impl<'a> ExecutedActionRepository<'a> {
    /// Wraps a connection in an executed-action repository.
    pub fn new(conn: &'a rusqlite::Connection) -> Self {
        Self { conn }
    }

    /// Records an IMAP action Berger executed for a message.
    ///
    /// # Errors
    /// Returns [`StorageError`] on a SQLite failure — including a foreign-key
    /// violation if `message_id` is not a known processed message.
    pub fn record(&self, action: &ExecutedAction) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT INTO executed_actions \
             (message_id, action_type, target, succeeded, error, executed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, CURRENT_TIMESTAMP)",
            rusqlite::params![
                action.message_id,
                action.action_type,
                action.target,
                action.succeeded,
                action.error,
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::database::Database;
    use crate::storage::processed_messages::ProcessedMessage;

    fn db_with_message(message_id: &str) -> Database {
        let db = Database::open(":memory:").unwrap();
        let account_id = db.accounts().insert("LINAGORA", "bichon-1").unwrap();
        db.processed_messages()
            .record(&ProcessedMessage {
                message_id: message_id.to_string(),
                account_id,
                bichon_uri: None,
                subject: None,
                from_email: None,
                from_name: None,
                date: None,
                berger_version: "0.0.1".to_string(),
                config_hash: "cfg".to_string(),
            })
            .unwrap();
        db
    }

    #[test]
    fn a_recorded_action_is_stored_for_its_message() {
        let db = db_with_message("<m@x>");
        db.executed_actions()
            .record(&ExecutedAction {
                message_id: "<m@x>".to_string(),
                action_type: "copy_to".to_string(),
                target: Some("notifs/github".to_string()),
                succeeded: true,
                error: None,
            })
            .unwrap();
        let (action_type, target, succeeded): (String, Option<String>, bool) = db
            .connection()
            .query_row(
                "SELECT action_type, target, succeeded \
                 FROM executed_actions WHERE message_id = ?1",
                ["<m@x>"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(action_type, "copy_to");
        assert_eq!(target.as_deref(), Some("notifs/github"));
        assert!(succeeded);
    }

    #[test]
    fn a_flag_action_records_a_null_target() {
        let db = db_with_message("<m@x>");
        db.executed_actions()
            .record(&ExecutedAction {
                message_id: "<m@x>".to_string(),
                action_type: "mark_seen".to_string(),
                target: None,
                succeeded: true,
                error: None,
            })
            .unwrap();
        let target: Option<String> = db
            .connection()
            .query_row(
                "SELECT target FROM executed_actions WHERE message_id = ?1",
                ["<m@x>"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(target, None);
    }

    #[test]
    fn recording_against_an_unknown_message_is_rejected() {
        // foreign_keys = ON: message_id must reference a processed message.
        let db = Database::open(":memory:").unwrap();
        assert!(
            db.executed_actions()
                .record(&ExecutedAction {
                    message_id: "<ghost@x>".to_string(),
                    action_type: "copy_to".to_string(),
                    target: Some("x".to_string()),
                    succeeded: true,
                    error: None,
                })
                .is_err()
        );
    }
}
