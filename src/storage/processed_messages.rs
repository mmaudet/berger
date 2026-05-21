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

//! Repository for the `processed_messages` table — the idempotency ledger.

use crate::storage::error::StorageError;

/// A row to record in the idempotency ledger once a message is processed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessedMessage {
    /// RFC 822 Message-ID — the idempotency key.
    pub message_id: String,
    pub account_id: i64,
    pub bichon_uri: Option<String>,
    pub subject: Option<String>,
    pub from_email: Option<String>,
    pub from_name: Option<String>,
    /// The message `Date:` header, epoch milliseconds.
    pub date: Option<i64>,
    pub berger_version: String,
    /// Hash of the YAML config in force when the message was processed.
    pub config_hash: String,
}

/// Read/write access to the `processed_messages` table — the source of
/// truth for Bichon coherence rule #2 (Message-ID idempotence).
pub struct ProcessedMessageRepository<'a> {
    conn: &'a rusqlite::Connection,
}

impl<'a> ProcessedMessageRepository<'a> {
    /// Wraps a connection in a processed-message repository.
    pub fn new(conn: &'a rusqlite::Connection) -> Self {
        Self { conn }
    }

    /// Returns whether a message with this Message-ID has already been
    /// processed — the idempotency check run before any processing.
    ///
    /// # Errors
    /// Returns [`StorageError`] on a SQLite failure.
    pub fn is_already_processed(&self, message_id: &str) -> Result<bool, StorageError> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM processed_messages WHERE message_id = ?1)",
            rusqlite::params![message_id],
            |row| row.get(0),
        )?)
    }

    /// Records a message in the idempotency ledger.
    ///
    /// # Errors
    /// Returns [`StorageError`] on a SQLite failure — including a primary
    /// key violation if the Message-ID is already recorded, or a foreign
    /// key violation if `account_id` does not name a known account.
    pub fn record(&self, message: &ProcessedMessage) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT INTO processed_messages (message_id, account_id, bichon_uri, subject, from_email, from_name, date, processed_at, berger_version, config_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, CURRENT_TIMESTAMP, ?8, ?9)",
            rusqlite::params![
                message.message_id,
                message.account_id,
                message.bichon_uri,
                message.subject,
                message.from_email,
                message.from_name,
                message.date,
                message.berger_version,
                message.config_hash,
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::database::Database;

    fn sample(message_id: &str, account_id: i64) -> ProcessedMessage {
        ProcessedMessage {
            message_id: message_id.to_string(),
            account_id,
            bichon_uri: Some("https://bichon.example/m/1".to_string()),
            subject: Some("Hello".to_string()),
            from_email: Some("a@example.test".to_string()),
            from_name: Some("Sender".to_string()),
            date: Some(1_779_109_081_851),
            berger_version: "0.0.1".to_string(),
            config_hash: "cfg-hash".to_string(),
        }
    }

    #[test]
    fn an_unrecorded_message_is_not_already_processed() {
        let db = Database::open(":memory:").unwrap();
        assert!(
            !db.processed_messages()
                .is_already_processed("<never@seen>")
                .unwrap()
        );
    }

    #[test]
    fn a_recorded_message_is_already_processed() {
        let db = Database::open(":memory:").unwrap();
        let account_id = db.accounts().insert("LINAGORA", "bichon-1").unwrap();
        let processed = db.processed_messages();
        processed
            .record(&sample("<m1@example.test>", account_id))
            .unwrap();
        assert!(processed.is_already_processed("<m1@example.test>").unwrap());
    }

    #[test]
    fn recording_is_scoped_to_the_exact_message_id() {
        let db = Database::open(":memory:").unwrap();
        let account_id = db.accounts().insert("LINAGORA", "bichon-1").unwrap();
        let processed = db.processed_messages();
        processed
            .record(&sample("<m1@example.test>", account_id))
            .unwrap();
        assert!(!processed.is_already_processed("<m2@example.test>").unwrap());
    }

    #[test]
    fn recording_the_same_message_twice_is_rejected() {
        // The primary key on message_id makes a duplicate insert fail loudly.
        let db = Database::open(":memory:").unwrap();
        let account_id = db.accounts().insert("LINAGORA", "bichon-1").unwrap();
        let processed = db.processed_messages();
        processed
            .record(&sample("<dup@example.test>", account_id))
            .unwrap();
        assert!(
            processed
                .record(&sample("<dup@example.test>", account_id))
                .is_err()
        );
    }

    #[test]
    fn recording_against_an_unknown_account_is_rejected() {
        // foreign_keys = ON: account_id must reference a real account.
        let db = Database::open(":memory:").unwrap();
        assert!(
            db.processed_messages()
                .record(&sample("<m@example.test>", 999_999))
                .is_err()
        );
    }
}
