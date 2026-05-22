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

//! Error type for the webhooks layer.

/// Something went wrong configuring or emitting a webhook.
///
/// Note that a webhook *delivery* that fails after exhausting its retries
/// is not a [`WebhookError`]: emission is fire-and-forget (PRD §5.6), so an
/// exhausted retry budget is logged and audited, not propagated. This error
/// covers only the cases that prevent emission from being attempted at all,
/// or that corrupt Berger's own state.
#[derive(Debug, thiserror::Error)]
pub enum WebhookError {
    /// A webhook is misconfigured — an unbuildable HTTP client, a header
    /// name or value that is not valid HTTP, or an unusable URL.
    #[error("webhook configuration error: {0}")]
    Config(String),

    /// The Handlebars payload template could not be parsed or rendered.
    #[error("webhook template error: {0}")]
    Template(String),

    /// The canonical payload could not be serialized to JSON.
    #[error("could not build the webhook payload: {0}")]
    Payload(#[from] serde_json::Error),

    /// Recording the emission in the sidecar failed.
    #[error("storage error: {0}")]
    Storage(#[from] crate::storage::error::StorageError),
}
