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

//! Error type for the actions layer.

/// Something went wrong applying an IMAP action.
#[derive(Debug, thiserror::Error)]
pub enum ActionError {
    /// An IMAP command failed.
    #[error("IMAP error: {0}")]
    Imap(String),

    /// The IMAP connection could not be established.
    #[error("IMAP connection failed: {0}")]
    Connection(String),
}
