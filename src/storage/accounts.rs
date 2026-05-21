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

//! Repository for the `accounts` table.

use rusqlite::OptionalExtension;

use crate::storage::error::StorageError;

/// A mail account Berger polls through Bichon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub id: i64,
    pub name: String,
    pub bichon_account_id: String,
    /// The persisted polling watermark (epoch milliseconds, stored as
    /// text in the `last_cursor` column), or `None` if never polled.
    pub last_cursor: Option<String>,
}

/// Read/write access to the `accounts` table.
pub struct AccountRepository<'a> {
    conn: &'a rusqlite::Connection,
}

impl<'a> AccountRepository<'a> {
    /// Wraps a connection in an account repository.
    pub fn new(conn: &'a rusqlite::Connection) -> Self {
        Self { conn }
    }

    /// Inserts a new account, returning its assigned id.
    ///
    /// # Errors
    /// Returns [`StorageError`] on a SQLite failure — including a unique
    /// constraint violation when `name` is already taken.
    pub fn insert(&self, name: &str, bichon_account_id: &str) -> Result<i64, StorageError> {
        self.conn.execute(
            "INSERT INTO accounts (name, bichon_account_id) VALUES (?1, ?2)",
            rusqlite::params![name, bichon_account_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Looks an account up by its unique name.
    ///
    /// # Errors
    /// Returns [`StorageError`] on a SQLite failure.
    pub fn find_by_name(&self, name: &str) -> Result<Option<Account>, StorageError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, name, bichon_account_id, last_cursor FROM accounts WHERE name = ?1",
                rusqlite::params![name],
                |row| {
                    Ok(Account {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        bichon_account_id: row.get(2)?,
                        last_cursor: row.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    /// Reads the polling watermark (epoch milliseconds) for an account, or
    /// `None` if none has been stored yet.
    ///
    /// # Errors
    /// Returns [`StorageError`] on a SQLite failure.
    pub fn get_watermark(&self, account_id: i64) -> Result<Option<i64>, StorageError> {
        let stored: Option<String> = self
            .conn
            .query_row(
                "SELECT last_cursor FROM accounts WHERE id = ?1",
                rusqlite::params![account_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        Ok(stored.and_then(|text| text.parse::<i64>().ok()))
    }

    /// Stores the polling watermark for an account and stamps the poll time.
    ///
    /// # Errors
    /// Returns [`StorageError`] on a SQLite failure.
    pub fn save_watermark(&self, account_id: i64, watermark_ms: i64) -> Result<(), StorageError> {
        self.conn.execute(
            "UPDATE accounts SET last_cursor = ?1, last_polled_at = CURRENT_TIMESTAMP WHERE id = ?2",
            rusqlite::params![watermark_ms.to_string(), account_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::database::Database;

    fn test_db() -> Database {
        Database::open(":memory:").unwrap()
    }

    #[test]
    fn insert_then_find_by_name_round_trips() {
        let db = test_db();
        let accounts = db.accounts();
        let id = accounts.insert("LINAGORA", "bichon-acc-1").unwrap();
        let found = accounts.find_by_name("LINAGORA").unwrap().unwrap();
        assert_eq!(found.id, id);
        assert_eq!(found.name, "LINAGORA");
        assert_eq!(found.bichon_account_id, "bichon-acc-1");
        assert_eq!(found.last_cursor, None);
    }

    #[test]
    fn find_by_name_returns_none_for_an_unknown_account() {
        let db = test_db();
        assert_eq!(db.accounts().find_by_name("nope").unwrap(), None);
    }

    #[test]
    fn a_fresh_account_has_no_watermark() {
        let db = test_db();
        let accounts = db.accounts();
        let id = accounts.insert("Gmail", "bichon-acc-2").unwrap();
        assert_eq!(accounts.get_watermark(id).unwrap(), None);
    }

    #[test]
    fn save_watermark_then_get_watermark_round_trips() {
        let db = test_db();
        let accounts = db.accounts();
        let id = accounts.insert("Gmail", "bichon-acc-3").unwrap();
        accounts.save_watermark(id, 1_779_109_081_851).unwrap();
        assert_eq!(accounts.get_watermark(id).unwrap(), Some(1_779_109_081_851));
    }

    #[test]
    fn account_names_are_unique() {
        let db = test_db();
        let accounts = db.accounts();
        accounts.insert("LINAGORA", "acc-a").unwrap();
        assert!(accounts.insert("LINAGORA", "acc-b").is_err());
    }
}
