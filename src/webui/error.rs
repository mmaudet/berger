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

//! Error type for the WebUI.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Something went wrong serving a WebUI request.
#[derive(Debug, thiserror::Error)]
pub enum WebError {
    /// A SQLite query failed.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// The shared sidecar connection's mutex was poisoned — a previous
    /// request handler panicked while holding it.
    #[error("the sidecar connection lock was poisoned")]
    LockPoisoned,

    /// A page was requested for a message that is not in the sidecar.
    #[error("message not found: {0}")]
    MessageNotFound(String),

    /// An Askama template failed to render.
    #[error("template rendering failed: {0}")]
    Render(#[from] askama::Error),
}

impl IntoResponse for WebError {
    /// Maps a [`WebError`] onto an HTTP response: a 404 for a missing
    /// message, a 500 otherwise. The detailed cause is logged, never
    /// echoed to the client.
    fn into_response(self) -> Response {
        let status = match &self {
            WebError::MessageNotFound(_) => StatusCode::NOT_FOUND,
            WebError::Database(_) | WebError::LockPoisoned | WebError::Render(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!(error = %self, "webui request failed");
        } else {
            tracing::warn!(error = %self, "webui request could not be served");
        }
        let body = match status {
            StatusCode::NOT_FOUND => "404 — not found",
            _ => "500 — internal error",
        };
        (status, body).into_response()
    }
}
