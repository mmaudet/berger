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

//! The `MessageSource` abstraction.

use std::future::Future;

use crate::ingest::error::IngestError;
use crate::ingest::types::{DataPage, EmailSearchRequest, Envelope, MinimalAccount};

/// Berger's upstream message archive.
///
/// Berger reads exclusively through this trait, so the concrete source —
/// Bichon today — stays swappable behind it, and so the pipeline can be
/// driven from an in-memory fake in tests (PRD §11).
///
/// Methods return `impl Future + Send` rather than using `async fn`
/// directly, so callers can spawn the resulting futures onto the Tokio
/// runtime.
pub trait MessageSource {
    /// Lists the accounts the source exposes.
    fn list_accounts(
        &self,
    ) -> impl Future<Output = Result<Vec<MinimalAccount>, IngestError>> + Send;

    /// Runs one page of a message search.
    fn search_messages(
        &self,
        request: EmailSearchRequest,
    ) -> impl Future<Output = Result<DataPage<Envelope>, IngestError>> + Send;
}
