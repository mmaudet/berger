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

//! The real [`ActionTarget`]: IMAP writeback over an `async-imap` session.

use std::fmt::Debug;

use async_imap::Session;
use futures_util::TryStreamExt;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::actions::error::ActionError;
use crate::actions::{ActionTarget, Flag};

/// An [`ActionTarget`] backed by a live, authenticated IMAP session.
///
/// Generic over the transport `T`, so it works over a TLS stream in
/// production and a plain TCP stream in integration tests. Establishing
/// the connection is the pipeline milestone's concern; [`Self::new`]
/// takes an already-authenticated session.
pub struct ImapActionTarget<T>
where
    T: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    session: Session<T>,
    /// The server's mailbox hierarchy separator (`/`, `.`, …).
    separator: String,
}

impl<T> ImapActionTarget<T>
where
    T: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    /// Wraps an authenticated IMAP session, discovering the server's
    /// mailbox hierarchy separator.
    ///
    /// # Errors
    /// Returns [`ActionError`] if the separator cannot be discovered.
    pub async fn new(mut session: Session<T>) -> Result<Self, ActionError> {
        let separator = discover_separator(&mut session).await?;
        Ok(Self { session, separator })
    }

    /// Translates a logical, `/`-separated path onto the server hierarchy.
    fn server_path(&self, folder: &str) -> String {
        folder.replace('/', &self.separator)
    }
}

/// Discovers the server's hierarchy separator via `LIST "" ""`.
async fn discover_separator<T>(session: &mut Session<T>) -> Result<String, ActionError>
where
    T: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    let names: Vec<_> = session
        .list(None, None)
        .await
        .map_err(imap_err)?
        .try_collect()
        .await
        .map_err(imap_err)?;
    Ok(names
        .first()
        .and_then(|name| name.delimiter())
        .unwrap_or("/")
        .to_string())
}

/// Maps an `async-imap` error onto [`ActionError::Imap`].
fn imap_err(error: async_imap::error::Error) -> ActionError {
    ActionError::Imap(error.to_string())
}

impl<T> ActionTarget for ImapActionTarget<T>
where
    T: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    async fn folder_exists(&mut self, folder: &str) -> Result<bool, ActionError> {
        let pattern = format!("\"{}\"", self.server_path(folder));
        let names: Vec<_> = self
            .session
            .list(None, Some(&pattern))
            .await
            .map_err(imap_err)?
            .try_collect()
            .await
            .map_err(imap_err)?;
        Ok(!names.is_empty())
    }

    async fn create_folder(&mut self, folder: &str) -> Result<(), ActionError> {
        let path = self.server_path(folder);
        self.session.create(&path).await.map_err(imap_err)?;
        self.session.subscribe(&path).await.map_err(imap_err)?;
        Ok(())
    }

    async fn copy_message(&mut self, uid: u32, folder: &str) -> Result<(), ActionError> {
        let path = self.server_path(folder);
        self.session
            .uid_copy(uid.to_string(), &path)
            .await
            .map_err(imap_err)
    }

    async fn move_message(&mut self, uid: u32, folder: &str) -> Result<(), ActionError> {
        // UID MOVE is atomic: it satisfies CLAUDE.md §3.3 (no global EXPUNGE,
        // no collateral purge) without a copy/store/expunge sequence.
        let path = self.server_path(folder);
        self.session
            .uid_mv(uid.to_string(), &path)
            .await
            .map_err(imap_err)
    }

    async fn add_flag(&mut self, uid: u32, flag: Flag) -> Result<(), ActionError> {
        let flags = match flag {
            Flag::Seen => "+FLAGS (\\Seen)",
            Flag::Flagged => "+FLAGS (\\Flagged)",
        };
        // Drain the FETCH responses so the next command starts on a clean stream.
        self.session
            .uid_store(uid.to_string(), flags)
            .await
            .map_err(imap_err)?
            .try_collect::<Vec<_>>()
            .await
            .map_err(imap_err)?;
        Ok(())
    }
}
