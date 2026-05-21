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

//! Command-line interface.

use clap::{Parser, Subcommand};

mod run;

/// The Berger command-line interface.
#[derive(Debug, Parser)]
#[command(name = "berger", version, about = "Open-source email triage daemon")]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// A Berger subcommand.
#[derive(Debug, Subcommand)]
enum Command {
    /// Run the triage daemon: poll, filter, act, then repeat.
    Run {
        /// Path to the configuration file.
        #[arg(long, default_value = "berger.yaml")]
        config: String,
    },
}

impl Cli {
    /// Runs the parsed command.
    ///
    /// # Errors
    /// Returns an error if the selected command fails.
    pub async fn dispatch(self) -> anyhow::Result<()> {
        match self.command {
            Command::Run { config } => run::run(&config).await,
        }
    }
}
