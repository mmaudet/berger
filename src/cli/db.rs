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

//! Shared helper for the read-only CLI commands.
//!
//! `explain` and `status` only inspect the sidecar — they must never
//! create it or mutate it. They open the database file read-only, which
//! also yields a clear error when the file does not exist yet, instead of
//! silently producing an empty database.

use anyhow::{Context, bail};
use rusqlite::{Connection, OpenFlags};

use crate::config::BergerConfig;

/// Resolves the sidecar's path from the configuration at `config_path`.
///
/// The read-only commands locate the database the same way `run` does —
/// through `database.path` in `berger.yaml` — so a single `--config` flag
/// is enough.
///
/// # Errors
/// Returns an error if the configuration cannot be read or parsed.
pub fn database_path(config_path: &str) -> anyhow::Result<String> {
    let raw = std::fs::read_to_string(config_path)
        .with_context(|| format!("reading config `{config_path}`"))?;
    let config = BergerConfig::parse(&raw).context("parsing the configuration")?;
    Ok(config.database.path)
}

/// Opens the sidecar at `path` read-only.
///
/// Unlike [`Database::open`], this neither creates the file nor runs
/// migrations: the read-only commands inspect an existing, already-migrated
/// database. A missing file is reported as a clear error.
///
/// [`Database::open`]: crate::storage::database::Database::open
///
/// # Errors
/// Returns an error if the file does not exist or cannot be opened.
pub fn open_readonly(path: &str) -> anyhow::Result<Connection> {
    if !std::path::Path::new(path).exists() {
        bail!(
            "the sidecar database `{path}` does not exist yet — run `berger run` first \
             so it gets created"
        );
    }
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("opening the sidecar database `{path}` read-only"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_readonly_reports_a_missing_file() {
        let error = open_readonly("/nonexistent/berger-cli-test.db").unwrap_err();
        assert!(error.to_string().contains("does not exist"));
    }

    #[test]
    fn open_readonly_opens_an_existing_database() {
        // Create a real, migrated sidecar, then re-open it read-only.
        let path = std::env::temp_dir().join(format!(
            "berger-cli-db-{}-{:?}.db",
            std::process::id(),
            std::thread::current().id()
        ));
        let path_str = path.to_str().unwrap();
        {
            let _db = crate::storage::database::Database::open(path_str).unwrap();
        }
        let conn = open_readonly(path_str).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(count > 0, "the migrated database has tables");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{path_str}-wal"));
        let _ = std::fs::remove_file(format!("{path_str}-shm"));
    }

    #[test]
    fn a_read_only_connection_rejects_writes() {
        let path = std::env::temp_dir().join(format!(
            "berger-cli-ro-{}-{:?}.db",
            std::process::id(),
            std::thread::current().id()
        ));
        let path_str = path.to_str().unwrap();
        {
            let _db = crate::storage::database::Database::open(path_str).unwrap();
        }
        let conn = open_readonly(path_str).unwrap();
        // A read-only handle must refuse an INSERT.
        assert!(
            conn.execute(
                "INSERT INTO accounts (name, bichon_account_id) VALUES ('x', 'y')",
                [],
            )
            .is_err()
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{path_str}-wal"));
        let _ = std::fs::remove_file(format!("{path_str}-shm"));
    }
}
