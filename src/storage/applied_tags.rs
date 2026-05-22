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

//! Repository for the `applied_tags` table — the tags Berger applied to a
//! message (PRD §5.9).

use crate::storage::error::StorageError;

/// A tag applied to a processed message, to record in `applied_tags`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedTag {
    /// RFC 822 Message-ID of the tagged message.
    pub message_id: String,
    /// The tag applied.
    pub tag: String,
}

/// Write access to the `applied_tags` table — one row per `(message, tag)`.
pub struct AppliedTagRepository<'a> {
    conn: &'a rusqlite::Connection,
}

impl<'a> AppliedTagRepository<'a> {
    /// Wraps a connection in an applied-tag repository.
    pub fn new(conn: &'a rusqlite::Connection) -> Self {
        Self { conn }
    }

    /// Records a tag applied to a message.
    ///
    /// # Errors
    /// Returns [`StorageError`] on a SQLite failure — including a foreign-key
    /// violation if `message_id` is not a known processed message, or a
    /// primary-key violation if the same `(message_id, tag)` is recorded
    /// twice.
    pub fn record(&self, tag: &AppliedTag) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT INTO applied_tags (message_id, tag, applied_at) \
             VALUES (?1, ?2, CURRENT_TIMESTAMP)",
            rusqlite::params![tag.message_id, tag.tag],
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

    fn tags_in_db(db: &Database, message_id: &str) -> Vec<String> {
        db.connection()
            .prepare("SELECT tag FROM applied_tags WHERE message_id = ?1 ORDER BY tag")
            .unwrap()
            .query_map([message_id], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn a_recorded_tag_is_stored_for_its_message() {
        let db = db_with_message("<m@x>");
        db.applied_tags()
            .record(&AppliedTag {
                message_id: "<m@x>".to_string(),
                tag: "cat/work".to_string(),
            })
            .unwrap();
        assert_eq!(tags_in_db(&db, "<m@x>"), ["cat/work"]);
    }

    #[test]
    fn recording_against_an_unknown_message_is_rejected() {
        // foreign_keys = ON: message_id must reference a processed message.
        let db = Database::open(":memory:").unwrap();
        assert!(
            db.applied_tags()
                .record(&AppliedTag {
                    message_id: "<ghost@x>".to_string(),
                    tag: "cat/work".to_string(),
                })
                .is_err()
        );
    }
}
