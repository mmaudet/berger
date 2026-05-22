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
//! patterns in them, and prints a summary — a starting point for writing
//! `berger.yaml`. It applies no IMAP action and never calls the LLM.

use anyhow::Context;

use crate::config::BergerConfig;
use crate::ingest::bichon_client::BichonClient;
use crate::scan::analyzer::{ScanReport, ScannedMessage, analyze, partition};
use crate::scan::headers::ScanHeaders;
use crate::scan::source::fetch_window;

/// Milliseconds in one day, for turning a `--since` day count into a
/// `Date:` lower bound.
const MILLIS_PER_DAY: i64 = 86_400_000;

/// How many rows per dimension the summary prints.
const SUMMARY_ROWS: usize = 10;

/// Loads the configuration, scans the inbox read-only over the `--since`
/// window, and prints a summary.
///
/// # Errors
/// Returns an error if `--since` is malformed, the configuration cannot be
/// loaded, the Bichon client cannot be built, an account name is unknown,
/// or the scan fails.
pub async fn run(config_path: &str, since: &str, account: Option<&str>) -> anyhow::Result<()> {
    let days = parse_since(since).map_err(anyhow::Error::msg)?;
    let config = BergerConfig::load(config_path).context("loading the configuration")?;

    let bichon = BichonClient::new(
        config.bichon.base_url.clone(),
        config.bichon.api_token.expose(),
    )
    .context("building the Bichon client")?;

    let account_ids = resolve_account_ids(&config, account)?;
    let since_ms = now_epoch_ms() - i64::from(days) * MILLIS_PER_DAY;

    let envelopes = fetch_window(&bichon, &account_ids, since_ms)
        .await
        .context("scanning the inbox")?;
    let (inbox_envelopes, sent) = partition(&envelopes);
    let inbox: Vec<ScannedMessage> = inbox_envelopes
        .iter()
        .map(|&envelope| ScannedMessage {
            envelope,
            headers: ScanHeaders::default(),
        })
        .collect();
    let report = analyze(&inbox, &sent);

    if report.sent_messages == 0 {
        tracing::warn!(
            "no Sent mail found in the scan window; the bidirectional dimension is skipped"
        );
    }

    print!("{}", render_summary(&report, days));
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

/// Renders a short, human-readable summary of a [`ScanReport`].
fn render_summary(report: &ScanReport, days: u32) -> String {
    let mut out = String::new();
    out.push_str("Berger scan — read-only inbox analysis.\n");
    out.push_str("No IMAP action applied, no LLM call, no message body read.\n\n");
    out.push_str(&format!("Window:            last {days} days\n"));
    out.push_str(&format!(
        "Messages analyzed: {} ({} inbox, {} sent)\n",
        report.messages_analyzed, report.inbox_messages, report.sent_messages
    ));

    out.push_str("\nTop senders\n");
    if report.top_senders.is_empty() {
        out.push_str("  (none)\n");
    }
    for sender in report.top_senders.iter().take(SUMMARY_ROWS) {
        out.push_str(&format!(
            "  {:>6}  {}\n",
            sender.messages_received, sender.address
        ));
    }

    out.push_str("\nTop domains\n");
    if report.top_domains.is_empty() {
        out.push_str("  (none)\n");
    }
    for domain in report.top_domains.iter().take(SUMMARY_ROWS) {
        out.push_str(&format!(
            "  {:>6}  {}\n",
            domain.messages_received, domain.domain
        ));
    }

    out.push_str("\nBidirectional contacts\n");
    if report.sent_messages == 0 {
        out.push_str("  (no Sent mail in the window — dimension skipped)\n");
    } else if report.bidirectional.is_empty() {
        out.push_str("  (none)\n");
    }
    for contact in report.bidirectional.iter().take(SUMMARY_ROWS) {
        out.push_str(&format!(
            "  {:>4} recv / {:>4} sent  {}\n",
            contact.messages_received, contact.messages_sent_to, contact.address
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::analyzers::senders::{BidirectionalContact, DomainCount, SenderCount};

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

    fn empty_report() -> ScanReport {
        ScanReport {
            messages_analyzed: 0,
            inbox_messages: 0,
            sent_messages: 0,
            top_senders: Vec::new(),
            top_domains: Vec::new(),
            bidirectional: Vec::new(),
        }
    }

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

    #[test]
    fn summary_states_the_read_only_guarantee() {
        let text = render_summary(&empty_report(), 30);
        assert!(text.contains("read-only"));
        assert!(text.contains("No IMAP action"));
        assert!(text.contains("no LLM call"));
    }

    #[test]
    fn summary_reports_the_analyzed_volume() {
        let mut report = empty_report();
        report.messages_analyzed = 50;
        report.inbox_messages = 42;
        report.sent_messages = 8;
        let text = render_summary(&report, 30);
        assert!(text.contains("50"));
        assert!(text.contains("42 inbox"));
        assert!(text.contains("8 sent"));
    }

    #[test]
    fn summary_lists_top_senders_and_domains() {
        let mut report = empty_report();
        report.top_senders = vec![SenderCount {
            address: "noreply@github.com".to_string(),
            messages_received: 284,
        }];
        report.top_domains = vec![DomainCount {
            domain: "github.com".to_string(),
            messages_received: 284,
        }];
        let text = render_summary(&report, 30);
        assert!(text.contains("noreply@github.com"));
        assert!(text.contains("github.com"));
        assert!(text.contains("284"));
    }

    #[test]
    fn summary_notes_a_missing_sent_folder() {
        let text = render_summary(&empty_report(), 30);
        assert!(text.contains("skipped"));
    }

    #[test]
    fn summary_shows_bidirectional_contacts() {
        let mut report = empty_report();
        report.sent_messages = 5;
        report.bidirectional = vec![BidirectionalContact {
            address: "partner@x.test".to_string(),
            messages_received: 47,
            messages_sent_to: 23,
            bidirectional_ratio: 0.49,
        }];
        let text = render_summary(&report, 30);
        assert!(text.contains("partner@x.test"));
        assert!(text.contains("47"));
        assert!(text.contains("23"));
    }
}
