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

//! The SQLite sidecar: connection, pragmas, and schema migrations.

use crate::storage::accounts::AccountRepository;
use crate::storage::error::StorageError;
use crate::storage::processed_messages::ProcessedMessageRepository;

mod embedded {
    refinery::embed_migrations!("migrations");
}

/// Handle to Berger's SQLite sidecar database.
pub struct Database {
    conn: rusqlite::Connection,
}

impl Database {
    /// Opens (creating it if needed) the sidecar at `path`, enables WAL mode
    /// and foreign-key enforcement, and applies every pending migration.
    /// Pass `":memory:"` for an ephemeral in-memory database.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the database cannot be opened or a
    /// migration fails.
    pub fn open(path: &str) -> Result<Self, StorageError> {
        let mut conn = rusqlite::Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode = WAL;\nPRAGMA foreign_keys = ON;")?;
        embedded::migrations::runner().run(&mut conn)?;
        Ok(Self { conn })
    }

    /// Borrows the underlying connection, for use by the repositories.
    pub fn connection(&self) -> &rusqlite::Connection {
        &self.conn
    }

    /// Returns a repository over the `accounts` table.
    pub fn accounts(&self) -> AccountRepository<'_> {
        AccountRepository::new(&self.conn)
    }

    /// Returns a repository over the `processed_messages` table.
    pub fn processed_messages(&self) -> ProcessedMessageRepository<'_> {
        ProcessedMessageRepository::new(&self.conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_db_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("berger-test-{tag}-{}.db", std::process::id()))
    }

    fn cleanup_db(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        if let Some(text) = path.to_str() {
            let _ = std::fs::remove_file(format!("{text}-wal"));
            let _ = std::fs::remove_file(format!("{text}-shm"));
        }
    }

    #[test]
    fn open_in_memory_creates_the_seven_tables() {
        let db = Database::open(":memory:").unwrap();
        let mut stmt = db
            .connection()
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap();
        let mut tables: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        tables.retain(|name| !name.starts_with("sqlite_") && !name.starts_with("refinery_"));
        assert_eq!(
            tables,
            [
                "accounts",
                "applied_tags",
                "executed_actions",
                "filter_matches",
                "llm_decisions",
                "processed_messages",
                "webhook_emissions",
            ]
        );
    }

    #[test]
    fn reopening_an_existing_database_reruns_migrations_cleanly() {
        let path = unique_temp_db_path("reopen");
        let path_str = path.to_str().unwrap();
        {
            let _db = Database::open(path_str).unwrap();
        }
        // Second open of the same file: refinery sees V1 already applied.
        {
            let _db = Database::open(path_str).unwrap();
        }
        cleanup_db(&path);
    }

    #[test]
    fn wal_mode_is_enabled_on_a_file_database() {
        let path = unique_temp_db_path("wal");
        let path_str = path.to_str().unwrap();
        {
            let db = Database::open(path_str).unwrap();
            let mode: String = db
                .connection()
                .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                .unwrap();
            assert_eq!(mode, "wal");
        }
        cleanup_db(&path);
    }
}
