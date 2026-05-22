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

//! Configuration types for the `webhooks:` section of `berger.yaml`
//! (PRD §5.6).

use std::collections::BTreeMap;

use serde::Deserialize;

/// One named webhook endpoint, as declared in the `webhooks:` section
/// (PRD §5.6). The `name` is what a tag's `webhook:` action references.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebhookConfig {
    /// The webhook's name — referenced by a tag's `webhook:` action.
    pub name: String,
    /// The endpoint URL to POST to.
    pub url: String,
    /// The HTTP method; only `POST` is meaningful at the MVP. Defaults to
    /// `POST` when omitted.
    #[serde(default = "default_method")]
    pub method: String,
    /// Extra HTTP headers (e.g. `Authorization`) sent with every request.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// The retry policy. Defaults to three attempts with exponential
    /// backoff when omitted (PRD §5.6).
    #[serde(default)]
    pub retry: RetryConfig,
    /// Tag filter: when non-empty, the webhook fires only if the message
    /// carries at least one of these tags (PRD §5.6, `when:`). An empty
    /// list — the default — places no restriction.
    #[serde(default)]
    pub when: Vec<String>,
}

/// The default HTTP method for a webhook.
fn default_method() -> String {
    "POST".to_string()
}

/// A webhook's retry policy (PRD §5.6).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryConfig {
    /// Total number of attempts, including the first. The PRD mandates a
    /// bounded budget of three (PRD §5.6).
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
    /// The backoff strategy between attempts.
    #[serde(default)]
    pub backoff: Backoff,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: default_max_attempts(),
            backoff: Backoff::default(),
        }
    }
}

/// The default retry budget (PRD §5.6: "3 tentatives").
fn default_max_attempts() -> u32 {
    3
}

/// How long to wait between webhook retries (PRD §5.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backoff {
    /// Exponential backoff — the PRD default of 1 s, 4 s, 16 s (PRD §5.6).
    #[default]
    Exponential,
    /// A fixed one-second delay between attempts.
    Fixed,
}

impl WebhookConfig {
    /// The delay before retry attempt number `attempt` (1-based: `attempt`
    /// is 1 for the wait *after* the first try, before the second).
    ///
    /// Exponential backoff follows the PRD's 1 s, 4 s, 16 s schedule —
    /// `4^(attempt - 1)` seconds (PRD §5.6).
    pub fn backoff_delay(&self, attempt: u32) -> std::time::Duration {
        match self.retry.backoff {
            Backoff::Fixed => std::time::Duration::from_secs(1),
            Backoff::Exponential => {
                let exponent = attempt.saturating_sub(1).min(10);
                std::time::Duration::from_secs(4_u64.saturating_pow(exponent))
            }
        }
    }

    /// Whether this webhook should fire for a message carrying `tags`.
    ///
    /// With no `when:` filter the webhook always fires; otherwise it fires
    /// only when at least one configured tag is present (PRD §5.6).
    pub fn fires_for(&self, tags: &[String]) -> bool {
        self.when.is_empty() || self.when.iter().any(|wanted| tags.contains(wanted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minimal_webhook_defaults_method_and_retry() {
        let yaml = "name: hermes-push\nurl: https://example.test/hook";
        let config: WebhookConfig = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(config.name, "hermes-push");
        assert_eq!(config.method, "POST");
        assert_eq!(config.retry.max_attempts, 3);
        assert_eq!(config.retry.backoff, Backoff::Exponential);
        assert!(config.headers.is_empty());
        assert!(config.when.is_empty());
    }

    #[test]
    fn a_full_webhook_parses_every_field() {
        let yaml = r#"
name: linatwin-draft
url: https://hermes.example/webhook/draft
method: POST
headers:
  Authorization: "Bearer tok"
retry:
  max_attempts: 5
  backoff: fixed
when:
  - "a-repondre/pro"
"#;
        let config: WebhookConfig = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(config.url, "https://hermes.example/webhook/draft");
        assert_eq!(config.headers.get("Authorization").unwrap(), "Bearer tok");
        assert_eq!(config.retry.max_attempts, 5);
        assert_eq!(config.retry.backoff, Backoff::Fixed);
        assert_eq!(config.when, ["a-repondre/pro"]);
    }

    #[test]
    fn an_unknown_webhook_field_is_rejected() {
        let yaml = "name: x\nurl: https://x.test\ntypo: true";
        assert!(serde_yaml_ng::from_str::<WebhookConfig>(yaml).is_err());
    }

    #[test]
    fn exponential_backoff_follows_the_prd_schedule() {
        let config: WebhookConfig =
            serde_yaml_ng::from_str("name: x\nurl: https://x.test").unwrap();
        // PRD §5.6: 1 s before retry #1, 4 s before #2, 16 s before #3.
        assert_eq!(config.backoff_delay(1).as_secs(), 1);
        assert_eq!(config.backoff_delay(2).as_secs(), 4);
        assert_eq!(config.backoff_delay(3).as_secs(), 16);
    }

    #[test]
    fn fixed_backoff_is_always_one_second() {
        let config: WebhookConfig =
            serde_yaml_ng::from_str("name: x\nurl: https://x.test\nretry:\n  backoff: fixed")
                .unwrap();
        assert_eq!(config.backoff_delay(1).as_secs(), 1);
        assert_eq!(config.backoff_delay(3).as_secs(), 1);
    }

    #[test]
    fn a_webhook_with_no_when_filter_fires_for_any_tags() {
        let config: WebhookConfig =
            serde_yaml_ng::from_str("name: x\nurl: https://x.test").unwrap();
        assert!(config.fires_for(&["whatever".to_string()]));
        assert!(config.fires_for(&[]));
    }

    #[test]
    fn a_when_filter_restricts_the_webhook_to_matching_tags() {
        let config: WebhookConfig =
            serde_yaml_ng::from_str("name: x\nurl: https://x.test\nwhen:\n  - \"cat/urgent\"")
                .unwrap();
        assert!(config.fires_for(&["cat/urgent".to_string()]));
        assert!(config.fires_for(&["other".to_string(), "cat/urgent".to_string()]));
        assert!(!config.fires_for(&["cat/work".to_string()]));
        assert!(!config.fires_for(&[]));
    }
}
