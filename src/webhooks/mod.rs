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

//! Webhooks: emitting the canonical `berger.tag_applied` event (PRD §5.6).
//!
//! When a fired tag carries a `webhook:` action, Berger POSTs a structured
//! event to the named endpoint, leaving the complex downstream work
//! (drafts, forwards, push notifications) to the consumer. Emission is
//! fire-and-forget with a bounded retry budget, and every attempt-set is
//! recorded in the `webhook_emissions` table.

pub mod config;
pub mod error;
pub mod payload;
