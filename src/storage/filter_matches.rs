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

//! Repository for the `filter_matches` table — why a tag was applied: the
//! native filter or the LLM that produced it (PRD §5.9).

use crate::storage::error::StorageError;

/// A filter (or the LLM) that fired a tag for a message, to record in
/// `filter_matches`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterMatch {
    /// RFC 822 Message-ID of the matched message.
    pub message_id: String,
    /// The filter family — `sender_in`, `subject_regex`, `list_unsubscribe`,
    /// `header_match`, or `llm` for a classifier-produced tag.
    pub filter_type: String,
    /// The filter's identifier — the tag it emits, or the LLM model name.
    pub filter_name: String,
    /// What matched precisely, as a JSON string, when recorded.
    pub details_json: Option<String>,
}

/// Write access to the `filter_matches` table — one row per filter that
/// fired for a message.
pub struct FilterMatchRepository<'a> {
    conn: &'a rusqlite::Connection,
}

impl<'a> FilterMatchRepository<'a> {
    /// Wraps a connection in a filter-match repository.
    pub fn new(conn: &'a rusqlite::Connection) -> Self {
        Self { conn }
    }

    /// Records a filter (or LLM) match for a message.
    ///
    /// # Errors
    /// Returns [`StorageError`] on a SQLite failure — including a foreign-key
    /// violation if `message_id` is not a known processed message.
    pub fn record(&self, filter_match: &FilterMatch) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT INTO filter_matches \
             (message_id, filter_type, filter_name, details_json) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                filter_match.message_id,
                filter_match.filter_type,
                filter_match.filter_name,
                filter_match.details_json,
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
    fn a_recorded_match_is_stored_for_its_message() {
        let db = db_with_message("<m@x>");
        db.filter_matches()
            .record(&FilterMatch {
                message_id: "<m@x>".to_string(),
                filter_type: "sender_in".to_string(),
                filter_name: "notif/github".to_string(),
                details_json: None,
            })
            .unwrap();
        let (filter_type, filter_name): (String, String) = db
            .connection()
            .query_row(
                "SELECT filter_type, filter_name \
                 FROM filter_matches WHERE message_id = ?1",
                ["<m@x>"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(filter_type, "sender_in");
        assert_eq!(filter_name, "notif/github");
    }

    #[test]
    fn recording_against_an_unknown_message_is_rejected() {
        // foreign_keys = ON: message_id must reference a processed message.
        let db = Database::open(":memory:").unwrap();
        assert!(
            db.filter_matches()
                .record(&FilterMatch {
                    message_id: "<ghost@x>".to_string(),
                    filter_type: "llm".to_string(),
                    filter_name: "test-model".to_string(),
                    details_json: None,
                })
                .is_err()
        );
    }
}
