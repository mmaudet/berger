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

//! Error type for the LLM client.

/// Something went wrong talking to the LLM.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    /// The HTTP client could not be built (e.g. a malformed API key).
    #[error("LLM client configuration error: {0}")]
    Config(String),

    /// The HTTP request itself failed.
    #[error("LLM transport error: {0}")]
    Transport(#[from] reqwest::Error),

    /// The LLM API answered with a non-success status.
    #[error("LLM API returned {status}")]
    Api {
        status: reqwest::StatusCode,
        body: String,
    },

    /// The response body could not be decoded.
    #[error("could not decode the LLM response: {0}")]
    Decode(String),

    /// The response carried no completion.
    #[error("the LLM response contained no choices")]
    EmptyResponse,
}
