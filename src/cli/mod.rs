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

mod db;
pub mod dry_run;
pub mod explain;
pub mod export_thunderbird;
mod run;
pub mod scan;
pub mod status;

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

    /// Print the full triage decision chain of one message: its tags, the
    /// filters and LLM decision behind them, the actions and webhooks.
    Explain {
        /// RFC 822 Message-ID of the message to explain.
        message_id: String,
        /// Path to the configuration file (used to locate the sidecar).
        #[arg(long, default_value = "berger.yaml")]
        config: String,
    },

    /// Print a health and statistics summary of the sidecar: messages
    /// processed, recent activity, LLM cost, and table counts.
    Status {
        /// Path to the configuration file (used to locate the sidecar).
        #[arg(long, default_value = "berger.yaml")]
        config: String,
    },

    /// Run one poll cycle without applying any IMAP action or recording
    /// anything — print the tags and actions Berger would apply.
    DryRun {
        /// Path to the configuration file.
        #[arg(long, default_value = "berger.yaml")]
        config: String,
    },

    /// Export the `actions:` configuration as a Mozilla Thunderbird
    /// `msgFilterRules.dat` ruleset, printed to stdout.
    ExportThunderbird {
        /// Path to the configuration file.
        #[arg(long, default_value = "berger.yaml")]
        config: String,
        /// Account name to export rules for; defaults to the first account.
        #[arg(long)]
        account: Option<String>,
        /// File to write the ruleset to; prints to stdout when omitted.
        #[arg(long)]
        output: Option<String>,
    },

    /// Scan the inbox read-only and report its recurring patterns —
    /// frequent senders, domains and two-way contacts — as a starting
    /// point for writing the configuration.
    Scan {
        /// Path to the configuration file.
        #[arg(long, default_value = "berger.yaml")]
        config: String,
        /// How far back to scan, as a number of days, e.g. `7d` or `30d`.
        #[arg(long, default_value = "30d")]
        since: String,
        /// Restrict the scan to one configured account by name; defaults
        /// to every configured account.
        #[arg(long)]
        account: Option<String>,
        /// What to write: the text report, the suggested YAML, or both.
        #[arg(long, value_enum, default_value = "all")]
        format: scan::ScanFormat,
        /// Path for the suggested-config YAML; defaults to a timestamped file.
        #[arg(long)]
        output: Option<String>,
        /// Minimum messages backing a suggestion (PRD v1.1 §4.4).
        #[arg(long, default_value_t = 5)]
        min_evidence: usize,
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
            Command::Explain { message_id, config } => explain::run(&config, &message_id),
            Command::Status { config } => status::run(&config),
            Command::DryRun { config } => dry_run::run(&config).await,
            Command::ExportThunderbird {
                config,
                account,
                output,
            } => export_thunderbird::run(&config, account.as_deref(), output.as_deref()),
            Command::Scan {
                config,
                since,
                account,
                format,
                output,
                min_evidence,
            } => {
                scan::run(
                    &config,
                    &since,
                    account.as_deref(),
                    format,
                    output.as_deref(),
                    min_evidence,
                )
                .await
            }
        }
    }
}
