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

//! The `scan` command (PRD v1.1): a strictly read-only analysis of the
//! inbox. It fetches recent envelopes from Bichon, measures the recurring
//! patterns in them, derives candidate configuration rules, and writes a
//! report. It applies no IMAP action and never calls the LLM.

use anyhow::Context;

use crate::config::BergerConfig;
use crate::ingest::bichon_client::BichonClient;
use crate::scan::formatter::{render_text, render_yaml};
use crate::scan::runner::scan;
use crate::scan::suggester::suggest;

/// Milliseconds in one day, for turning a `--since` day count into a
/// `Date:` lower bound.
const MILLIS_PER_DAY: i64 = 86_400_000;

/// What `berger scan` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ScanFormat {
    /// The human-readable report, on stdout.
    Text,
    /// The suggested-configuration YAML, written to a file.
    Yaml,
    /// Both: the text report on stdout and the suggested YAML file.
    All,
}

/// Loads the configuration, scans the inbox read-only over the `--since`
/// window, derives suggestions, and writes the requested output(s).
///
/// # Errors
/// Returns an error if `--since` is malformed, the configuration cannot be
/// loaded, the Bichon client cannot be built, an account name is unknown,
/// the scan fails, or the YAML file cannot be written.
pub async fn run(
    config_path: &str,
    since: &str,
    account: Option<&str>,
    format: ScanFormat,
    output: Option<&str>,
    min_evidence: usize,
) -> anyhow::Result<()> {
    let days = parse_since(since).map_err(anyhow::Error::msg)?;
    let config = BergerConfig::load(config_path).context("loading the configuration")?;

    let bichon = BichonClient::new(
        config.bichon.base_url.clone(),
        config.bichon.api_token.expose(),
    )
    .context("building the Bichon client")?;

    let account_ids = resolve_account_ids(&config, account)?;
    let since_ms = now_epoch_ms() - i64::from(days) * MILLIS_PER_DAY;

    let report = scan(&bichon, &account_ids, since_ms)
        .await
        .context("scanning the inbox")?;
    if report.sent_messages == 0 {
        tracing::warn!(
            "no Sent mail found in the scan window; the bidirectional dimension is skipped"
        );
    }
    let suggestions = suggest(&report, min_evidence);

    if matches!(format, ScanFormat::Text | ScanFormat::All) {
        print!("{}", render_text(&report, &suggestions, days));
    }
    if matches!(format, ScanFormat::Yaml | ScanFormat::All) {
        let path = output.map_or_else(
            || format!("berger-scan-{}.yaml", now_epoch_ms()),
            str::to_string,
        );
        std::fs::write(&path, render_yaml(&report, &suggestions, days))
            .with_context(|| format!("writing the suggestions to `{path}`"))?;
        println!(
            "Wrote {} suggestion(s) to {path}",
            suggestions.filters.len()
        );
    }
    Ok(())
}

/// Current wall-clock time as epoch milliseconds.
fn now_epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// Parses a `--since` value such as `30d` into a number of days.
fn parse_since(spec: &str) -> Result<u32, String> {
    let spec = spec.trim();
    let digits = spec
        .strip_suffix('d')
        .ok_or_else(|| format!("invalid --since `{spec}`: expected a day count such as `30d`"))?;
    let days: u32 = digits.parse().map_err(|_| {
        format!("invalid --since `{spec}`: `{digits}` is not a whole number of days")
    })?;
    if days == 0 {
        return Err(format!(
            "invalid --since `{spec}`: the window must be at least one day"
        ));
    }
    Ok(days)
}

/// Resolves the Bichon account ids to scan: the one named `account`, or
/// every configured account when `account` is `None`.
fn resolve_account_ids(config: &BergerConfig, account: Option<&str>) -> anyhow::Result<Vec<u64>> {
    let matching = config
        .accounts
        .iter()
        .filter(|candidate| account.is_none_or(|name| candidate.name.as_str() == name))
        .collect::<Vec<_>>();
    if matching.is_empty()
        && let Some(name) = account
    {
        anyhow::bail!("no account named `{name}` in the configuration");
    }
    matching
        .into_iter()
        .map(|candidate| {
            candidate.bichon_account_id.parse::<u64>().with_context(|| {
                format!(
                    "account `{}` has a non-numeric bichon_account_id",
                    candidate.name
                )
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_ACCOUNTS: &str = r#"
bichon:
  base_url: "https://bichon.example"
  api_token: "tok"
database:
  path: "berger.db"
accounts:
  - name: "LINAGORA"
    bichon_account_id: "111"
    imap:
      host: "imap.example"
      user: "berger"
      password: "pw"
  - name: "Gmail"
    bichon_account_id: "222"
    imap:
      host: "imap.gmail.com"
      user: "berger@gmail.com"
      password: "pw2"
"#;

    #[test]
    fn parse_since_reads_a_day_count() {
        assert_eq!(parse_since("30d"), Ok(30));
        assert_eq!(parse_since("7d"), Ok(7));
        assert_eq!(parse_since("180d"), Ok(180));
    }

    #[test]
    fn parse_since_rejects_a_missing_day_suffix() {
        assert!(parse_since("30").is_err());
    }

    #[test]
    fn parse_since_rejects_a_non_number() {
        assert!(parse_since("abcd").is_err());
    }

    #[test]
    fn parse_since_rejects_a_zero_window() {
        assert!(parse_since("0d").is_err());
    }

    #[test]
    fn resolve_account_ids_returns_every_account_by_default() {
        let config = BergerConfig::parse(TWO_ACCOUNTS).unwrap();
        assert_eq!(resolve_account_ids(&config, None).unwrap(), vec![111, 222]);
    }

    #[test]
    fn resolve_account_ids_filters_by_name() {
        let config = BergerConfig::parse(TWO_ACCOUNTS).unwrap();
        assert_eq!(
            resolve_account_ids(&config, Some("Gmail")).unwrap(),
            vec![222]
        );
    }

    #[test]
    fn resolve_account_ids_rejects_an_unknown_name() {
        let config = BergerConfig::parse(TWO_ACCOUNTS).unwrap();
        assert!(resolve_account_ids(&config, Some("Nope")).is_err());
    }

    #[test]
    fn resolve_account_ids_rejects_a_non_numeric_id() {
        let yaml = TWO_ACCOUNTS.replace("\"111\"", "\"not-a-number\"");
        let config = BergerConfig::parse(&yaml).unwrap();
        assert!(resolve_account_ids(&config, None).is_err());
    }
}
