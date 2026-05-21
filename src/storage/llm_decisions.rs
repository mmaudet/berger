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

//! Repository for the `llm_decisions` table — the LLM cache and audit log.

use rusqlite::OptionalExtension;

use crate::storage::error::StorageError;

/// One LLM classification decision — a cache entry and an audit record
/// (PRD §5.3, §5.9).
#[derive(Debug, Clone, PartialEq)]
pub struct LlmDecision {
    /// RFC 822 Message-ID of the classified message.
    pub message_id: String,
    /// The model that produced the decision.
    pub model: String,
    /// Hash of the prompt — the second half of the cache key.
    pub prompt_hash: String,
    /// The full prompt sent to the model, kept for audit.
    pub prompt_text: String,
    /// The raw JSON the model returned.
    pub response_json: String,
    /// Prompt tokens billed, when the API reports them.
    pub tokens_input: Option<i64>,
    /// Completion tokens billed, when the API reports them.
    pub tokens_output: Option<i64>,
    /// Round-trip latency in milliseconds.
    pub latency_ms: Option<i64>,
    /// Estimated cost in US dollars, when known.
    pub cost_usd: Option<f64>,
}

/// Read/write access to the `llm_decisions` table: the LLM cache keyed by
/// `(message_id, prompt_hash)` and the audit/cost log (PRD §5.3).
pub struct LlmDecisionRepository<'a> {
    conn: &'a rusqlite::Connection,
}

impl<'a> LlmDecisionRepository<'a> {
    /// Wraps a connection in an LLM-decision repository.
    pub fn new(conn: &'a rusqlite::Connection) -> Self {
        Self { conn }
    }

    /// Returns the cached response JSON for this `(message_id, prompt_hash)`
    /// pair, or `None` if the model has not been asked exactly this — so the
    /// LLM is never invoked twice for the same message and prompt (PRD §5.3).
    ///
    /// # Errors
    /// Returns [`StorageError`] on a SQLite failure.
    pub fn find_cached(
        &self,
        message_id: &str,
        prompt_hash: &str,
    ) -> Result<Option<String>, StorageError> {
        Ok(self
            .conn
            .query_row(
                "SELECT response_json FROM llm_decisions WHERE message_id = ?1 AND prompt_hash = ?2 LIMIT 1",
                rusqlite::params![message_id, prompt_hash],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Records a decision in the cache and the audit log.
    ///
    /// # Errors
    /// Returns [`StorageError`] on a SQLite failure — including a foreign
    /// key violation if `message_id` is not a known processed message.
    pub fn record(&self, decision: &LlmDecision) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT INTO llm_decisions (message_id, model, prompt_hash, prompt_text, response_json, tokens_input, tokens_output, latency_ms, cost_usd, called_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, CURRENT_TIMESTAMP)",
            rusqlite::params![
                decision.message_id,
                decision.model,
                decision.prompt_hash,
                decision.prompt_text,
                decision.response_json,
                decision.tokens_input,
                decision.tokens_output,
                decision.latency_ms,
                decision.cost_usd,
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

    fn sample(message_id: &str, prompt_hash: &str) -> LlmDecision {
        LlmDecision {
            message_id: message_id.to_string(),
            model: "test-model".to_string(),
            prompt_hash: prompt_hash.to_string(),
            prompt_text: "classify this".to_string(),
            response_json: r#"{"category":"work"}"#.to_string(),
            tokens_input: Some(10),
            tokens_output: Some(5),
            latency_ms: Some(200),
            cost_usd: Some(0.0001),
        }
    }

    #[test]
    fn an_unknown_decision_is_not_cached() {
        let db = db_with_message("<m@x>");
        assert!(
            db.llm_decisions()
                .find_cached("<m@x>", "hash-1")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_recorded_decision_is_returned_by_find_cached() {
        let db = db_with_message("<m@x>");
        let repo = db.llm_decisions();
        repo.record(&sample("<m@x>", "hash-1")).unwrap();
        assert_eq!(
            repo.find_cached("<m@x>", "hash-1").unwrap().as_deref(),
            Some(r#"{"category":"work"}"#)
        );
    }

    #[test]
    fn find_cached_is_scoped_to_the_prompt_hash() {
        // A changed prompt (new hash) is a cache miss — the model is re-asked.
        let db = db_with_message("<m@x>");
        let repo = db.llm_decisions();
        repo.record(&sample("<m@x>", "hash-1")).unwrap();
        assert!(repo.find_cached("<m@x>", "hash-2").unwrap().is_none());
    }

    #[test]
    fn recording_against_an_unknown_message_is_rejected() {
        // foreign_keys = ON: message_id must reference a processed message.
        let db = Database::open(":memory:").unwrap();
        assert!(
            db.llm_decisions()
                .record(&sample("<ghost@x>", "h"))
                .is_err()
        );
    }
}
