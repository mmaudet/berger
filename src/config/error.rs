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

//! Error type for configuration loading.

/// Something went wrong loading or validating `berger.yaml`.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The config file could not be read.
    #[error("could not read config file `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// An `${ENV_VAR}` reference could not be resolved.
    #[error("config interpolation failed: {0}")]
    Interpolation(String),

    /// The YAML could not be parsed.
    #[error("could not parse config: {0}")]
    Parse(#[from] serde_yaml_ng::Error),

    /// The config parsed but failed validation.
    #[error("invalid config: {0}")]
    Validation(String),
}
