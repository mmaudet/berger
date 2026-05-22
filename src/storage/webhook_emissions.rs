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

//! Repository for the `webhook_emissions` table — the webhook audit log
//! (PRD §5.6, §5.9).

use crate::storage::error::StorageError;

/// One webhook emission to record in the audit log (PRD §5.9).
///
/// A row is written once per `(message, webhook)` pair, after the retry
/// loop has finished — whether it eventually succeeded or was abandoned.
#[derive(Debug, Clone, PartialEq)]
pub struct WebhookEmission {
    /// RFC 822 Message-ID of the message that triggered the webhook.
    pub message_id: String,
    /// The webhook's name in the configuration.
    pub webhook_name: String,
    /// The exact JSON body that was POSTed.
    pub payload_json: String,
    /// HTTP status of the last attempt, when one was received.
    pub http_status: Option<i64>,
    /// Number of attempts made (1 on first-try success).
    pub attempts: i64,
    /// Whether the webhook ultimately succeeded.
    pub succeeded: bool,
}

/// Write access to the `webhook_emissions` table — the audit trail of every
/// webhook Berger has POSTed (PRD §5.6).
pub struct WebhookEmissionRepository<'a> {
    conn: &'a rusqlite::Connection,
}

impl<'a> WebhookEmissionRepository<'a> {
    /// Wraps a connection in a webhook-emission repository.
    pub fn new(conn: &'a rusqlite::Connection) -> Self {
        Self { conn }
    }

    /// Records one webhook emission in the audit log.
    ///
    /// `emitted_at` and `completed_at` are both stamped with the current
    /// time: Berger emits and waits inline, so the two instants coincide
    /// closely enough for an audit log.
    ///
    /// # Errors
    /// Returns [`StorageError`] on a SQLite failure — including a foreign
    /// key violation if `message_id` is not a known processed message.
    pub fn record(&self, emission: &WebhookEmission) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT INTO webhook_emissions (message_id, webhook_name, payload_json, http_status, attempts, succeeded, emitted_at, completed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
            rusqlite::params![
                emission.message_id,
                emission.webhook_name,
                emission.payload_json,
                emission.http_status,
                emission.attempts,
                emission.succeeded,
            ],
        )?;
        Ok(())
    }

    /// Counts the rows in the `webhook_emissions` table — used by tests and,
    /// later, by the WebUI's webhook counters.
    ///
    /// # Errors
    /// Returns [`StorageError`] on a SQLite failure.
    pub fn count(&self) -> Result<i64, StorageError> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM webhook_emissions", [], |row| {
                row.get(0)
            })?)
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

    fn sample(message_id: &str) -> WebhookEmission {
        WebhookEmission {
            message_id: message_id.to_string(),
            webhook_name: "linatwin-draft".to_string(),
            payload_json: r#"{"event":"berger.tag_applied"}"#.to_string(),
            http_status: Some(200),
            attempts: 1,
            succeeded: true,
        }
    }

    #[test]
    fn a_fresh_table_is_empty() {
        let db = db_with_message("<m@x>");
        assert_eq!(db.webhook_emissions().count().unwrap(), 0);
    }

    #[test]
    fn recording_an_emission_increments_the_count() {
        let db = db_with_message("<m@x>");
        let repo = db.webhook_emissions();
        repo.record(&sample("<m@x>")).unwrap();
        assert_eq!(repo.count().unwrap(), 1);
    }

    #[test]
    fn an_emission_round_trips_its_fields() {
        let db = db_with_message("<m@x>");
        db.webhook_emissions().record(&sample("<m@x>")).unwrap();
        let (name, payload, status, attempts, succeeded): (
            String,
            String,
            Option<i64>,
            i64,
            bool,
        ) = db
            .connection()
            .query_row(
                "SELECT webhook_name, payload_json, http_status, attempts, succeeded FROM webhook_emissions",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(name, "linatwin-draft");
        assert_eq!(payload, r#"{"event":"berger.tag_applied"}"#);
        assert_eq!(status, Some(200));
        assert_eq!(attempts, 1);
        assert!(succeeded);
    }

    #[test]
    fn a_failed_emission_is_recorded_too() {
        // A webhook that exhausted its retries is still audited (PRD §5.6).
        let db = db_with_message("<m@x>");
        let repo = db.webhook_emissions();
        repo.record(&WebhookEmission {
            http_status: Some(500),
            attempts: 3,
            succeeded: false,
            ..sample("<m@x>")
        })
        .unwrap();
        let succeeded: bool = db
            .connection()
            .query_row("SELECT succeeded FROM webhook_emissions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(!succeeded);
    }

    #[test]
    fn a_transport_failure_records_a_null_status() {
        // No HTTP response ever arrived: http_status stays NULL.
        let db = db_with_message("<m@x>");
        let repo = db.webhook_emissions();
        repo.record(&WebhookEmission {
            http_status: None,
            attempts: 3,
            succeeded: false,
            ..sample("<m@x>")
        })
        .unwrap();
        let status: Option<i64> = db
            .connection()
            .query_row("SELECT http_status FROM webhook_emissions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(status, None);
    }

    #[test]
    fn recording_against_an_unknown_message_is_rejected() {
        // foreign_keys = ON: message_id must reference a processed message.
        let db = Database::open(":memory:").unwrap();
        assert!(db.webhook_emissions().record(&sample("<ghost@x>")).is_err());
    }
}
