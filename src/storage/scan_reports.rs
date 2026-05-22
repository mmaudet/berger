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

//! Repository for the `scan_reports` table — the optional history of
//! `berger scan` runs (PRD v1.1).

use rusqlite::OptionalExtension;

use crate::storage::error::StorageError;

/// A persisted `berger scan` run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanReportRow {
    /// The `--since` window the scan covered, in days.
    pub period_days: u32,
    /// How many messages the scan analyzed.
    pub messages_analyzed: usize,
    /// The full `ScanReport`, serialized as JSON.
    pub report_json: String,
    /// The derived `Suggestions`, serialized as JSON.
    pub suggestions_json: String,
}

/// Read/write access to the `scan_reports` table.
pub struct ScanReportRepository<'a> {
    conn: &'a rusqlite::Connection,
}

impl<'a> ScanReportRepository<'a> {
    /// Wraps a connection in a scan-report repository.
    pub fn new(conn: &'a rusqlite::Connection) -> Self {
        Self { conn }
    }

    /// Persists a scan run, returning its row id.
    ///
    /// # Errors
    /// Returns [`StorageError`] on a SQLite failure.
    pub fn save(&self, row: &ScanReportRow) -> Result<i64, StorageError> {
        self.conn.execute(
            "INSERT INTO scan_reports (created_at, period_days, messages_analyzed, report_json, suggestions_json) VALUES (CURRENT_TIMESTAMP, ?1, ?2, ?3, ?4)",
            rusqlite::params![
                row.period_days,
                row.messages_analyzed as i64,
                row.report_json,
                row.suggestions_json,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Returns the most recently saved scan run, or `None` when the table
    /// is empty.
    ///
    /// # Errors
    /// Returns [`StorageError`] on a SQLite failure.
    pub fn latest(&self) -> Result<Option<ScanReportRow>, StorageError> {
        Ok(self
            .conn
            .query_row(
                "SELECT period_days, messages_analyzed, report_json, suggestions_json FROM scan_reports ORDER BY id DESC LIMIT 1",
                [],
                |row| {
                    Ok(ScanReportRow {
                        period_days: row.get::<_, i64>(0)? as u32,
                        messages_analyzed: row.get::<_, i64>(1)? as usize,
                        report_json: row.get(2)?,
                        suggestions_json: row.get(3)?,
                    })
                },
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::database::Database;

    fn sample(period_days: u32) -> ScanReportRow {
        ScanReportRow {
            period_days,
            messages_analyzed: 100,
            report_json: r#"{"messages_analyzed":100}"#.to_string(),
            suggestions_json: r#"{"filters":[]}"#.to_string(),
        }
    }

    #[test]
    fn latest_is_none_on_an_empty_table() {
        let db = Database::open(":memory:").unwrap();
        assert_eq!(db.scan_reports().latest().unwrap(), None);
    }

    #[test]
    fn a_saved_report_is_returned_by_latest() {
        let db = Database::open(":memory:").unwrap();
        let repo = db.scan_reports();
        repo.save(&sample(30)).unwrap();
        assert_eq!(repo.latest().unwrap(), Some(sample(30)));
    }

    #[test]
    fn latest_returns_the_most_recent_save() {
        let db = Database::open(":memory:").unwrap();
        let repo = db.scan_reports();
        repo.save(&sample(30)).unwrap();
        repo.save(&sample(90)).unwrap();
        assert_eq!(repo.latest().unwrap().unwrap().period_days, 90);
    }

    #[test]
    fn save_returns_an_increasing_row_id() {
        let db = Database::open(":memory:").unwrap();
        let repo = db.scan_reports();
        let first = repo.save(&sample(7)).unwrap();
        let second = repo.save(&sample(7)).unwrap();
        assert!(second > first);
    }
}
