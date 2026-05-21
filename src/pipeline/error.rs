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

//! Error type for the pipeline.

use crate::actions::error::ActionError;
use crate::ingest::error::IngestError;
use crate::storage::error::StorageError;

/// Something went wrong processing a message through the pipeline.
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    /// A storage (SQLite) operation failed.
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    /// Fetching the message from the source failed.
    #[error("ingest error: {0}")]
    Ingest(#[from] IngestError),

    /// Applying an IMAP action failed.
    #[error("action error: {0}")]
    Action(#[from] ActionError),
}
