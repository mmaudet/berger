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

//! The `run` command: the triage daemon loop.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;

use crate::actions::imap_target::ImapActionTarget;
use crate::config::{AccountConfig, BergerConfig};
use crate::ingest::bichon_client::BichonClient;
use crate::ingest::poller::{Watermark, poll_account};
use crate::llm::LlmClient;
use crate::llm::classifier::Classifier;
use crate::pipeline::{Pipeline, ProcessOutcome, compile_filters};
use crate::storage::database::Database;
use crate::webui::{self, AppState};

/// How long to wait between poll cycles.
const POLL_INTERVAL: Duration = Duration::from_secs(300);

/// Loads the configuration and runs the triage daemon: poll, filter, act,
/// then repeat, forever.
///
/// # Errors
/// Returns an error if the configuration cannot be loaded or the Bichon
/// client, the database or the filters cannot be built. Per-account and
/// per-message failures are logged and skipped, never fatal.
pub async fn run(config_path: &str) -> anyhow::Result<()> {
    let raw = std::fs::read_to_string(config_path)
        .with_context(|| format!("reading config `{config_path}`"))?;
    let config = BergerConfig::parse(&raw).context("parsing the configuration")?;
    let config_hash = hash_config(&raw);

    let bichon = BichonClient::new(
        config.bichon.base_url.clone(),
        config.bichon.api_token.expose(),
    )
    .context("building the Bichon client")?;
    let database = Database::open(&config.database.path).context("opening the database")?;
    let filters = compile_filters(&config.filters).context("compiling the filters")?;
    let classifier = match &config.llm {
        Some(llm) => {
            let client = LlmClient::new(
                &llm.endpoint,
                &llm.model,
                llm.api_key.as_ref().map(|key| key.expose()),
            )
            .context("building the LLM client")?;
            Some(Classifier::new(
                client,
                llm.model.clone(),
                llm.categories.clone(),
            ))
        }
        None => None,
    };
    let pipeline = Pipeline::new(filters, config.actions.clone(), config_hash, classifier);

    spawn_webui(&config.database.path, &raw);

    tracing::info!(
        accounts = config.accounts.len(),
        filters = config.filters.len(),
        interval_secs = POLL_INTERVAL.as_secs(),
        "berger started"
    );

    loop {
        for account in &config.accounts {
            if let Err(error) = poll_one_account(account, &bichon, &database, &pipeline).await {
                tracing::error!(account = %account.name, error = %error, "poll cycle failed");
            }
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Polls and triages one account for one cycle.
async fn poll_one_account(
    account: &AccountConfig,
    bichon: &BichonClient,
    database: &Database,
    pipeline: &Pipeline,
) -> anyhow::Result<()> {
    // Ensure the account has a row in the sidecar.
    let db_account_id = match database.accounts().find_by_name(&account.name)? {
        Some(existing) => existing.id,
        None => database
            .accounts()
            .insert(&account.name, &account.bichon_account_id)?,
    };

    let bichon_account_id: u64 = account.bichon_account_id.parse().with_context(|| {
        format!(
            "account `{}` has a non-numeric bichon_account_id",
            account.name
        )
    })?;

    let watermark = match database.accounts().get_watermark(db_account_id)? {
        Some(epoch_ms) => Watermark::at(epoch_ms),
        None => Watermark::starting_now(),
    };
    let outcome = poll_account(bichon, bichon_account_id, watermark).await?;

    let mut target = ImapActionTarget::connect(
        &account.imap.host,
        account.imap.port,
        &account.imap.user,
        account.imap.password.expose(),
    )
    .await
    .with_context(|| format!("connecting to IMAP for account `{}`", account.name))?;

    let mut triaged = 0_usize;
    let mut skipped = 0_usize;
    for envelope in &outcome.envelopes {
        match pipeline
            .process(envelope, db_account_id, bichon, database, &mut target)
            .await
        {
            Ok(ProcessOutcome::Processed { .. }) => triaged += 1,
            Ok(ProcessOutcome::AlreadyProcessed) => skipped += 1,
            Err(error) => tracing::error!(
                account = %account.name,
                message_id = %envelope.message_id,
                error = %error,
                "failed to process a message"
            ),
        }
    }

    database
        .accounts()
        .save_watermark(db_account_id, outcome.watermark.as_epoch_ms())?;

    tracing::info!(
        account = %account.name,
        polled = outcome.envelopes.len(),
        triaged,
        skipped,
        "poll cycle complete"
    );
    Ok(())
}

/// Starts the WebUI (PRD §5.7) as a background task on the fixed port.
///
/// The WebUI opens its own SQLite connection to the same sidecar file —
/// safe because the database runs in WAL mode (PRD §5.8) — so the triage
/// loop and the server never contend for one connection. A failure to open
/// that connection, or to bind the port, is logged and leaves the daemon
/// running: the triage loop, not the WebUI, is the daemon's core duty.
fn spawn_webui(database_path: &str, raw_config: &str) {
    let database = match Database::open(database_path) {
        Ok(database) => database,
        Err(error) => {
            tracing::error!(error = %error, "webui disabled: cannot open the sidecar");
            return;
        }
    };
    let state = AppState::new(Arc::new(Mutex::new(database)), raw_config);
    tokio::spawn(async move {
        if let Err(error) = webui::serve(state, webui::DEFAULT_PORT).await {
            tracing::error!(error = %error, "webui server stopped");
        }
    });
}

/// A short, stable fingerprint of the raw configuration text, recorded
/// with every processed message.
fn hash_config(raw: &str) -> String {
    let mut hasher = DefaultHasher::new();
    raw.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
