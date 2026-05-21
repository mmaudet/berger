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

//! Error type for the ingestion layer.

/// Something went wrong while reading from the upstream message source.
#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    /// The HTTP request to Bichon could not be completed — DNS, TLS,
    /// connection, timeout, or body-read failure.
    #[error("Bichon request failed: {0}")]
    Transport(#[from] reqwest::Error),

    /// Bichon answered with a non-success status and a recognised error
    /// envelope.
    #[error("Bichon returned HTTP {status}: [{code}] {message}")]
    Api {
        status: reqwest::StatusCode,
        code: u32,
        message: String,
    },

    /// Bichon answered with a non-success status and a body that is not a
    /// recognised error envelope.
    #[error("Bichon returned HTTP {status} with an unrecognised body: {body}")]
    Unexpected {
        status: reqwest::StatusCode,
        body: String,
    },

    /// A Bichon response body could not be decoded into the expected type.
    #[error("could not decode a Bichon response: {0}")]
    Decode(#[from] serde_json::Error),

    /// The Bichon client was given invalid configuration.
    #[error("invalid Bichon client configuration: {0}")]
    Config(String),
}
