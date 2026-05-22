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

//! WebUI: the Axum HTTP server and its four Askama-rendered pages (PRD §5.7).
//!
//! `berger run` starts [`serve`] as a background task on port [`DEFAULT_PORT`].
//! It exposes `/` (stats), `/recent` (recently triaged messages),
//! `/explain/<id>` (the full triage of one message) and `/config` (the
//! active configuration, secrets redacted), reading the sidecar through
//! [`Database::connection`](crate::storage::database::Database::connection).

pub mod error;
pub mod queries;
pub mod server;
pub mod static_assets;
pub mod templates;

pub use server::{AppState, serve};

/// The port the WebUI listens on — fixed by PRD §5.7.
pub const DEFAULT_PORT: u16 = 7000;
