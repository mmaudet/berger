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

//! The Askama page templates and the config-redaction helper.
//!
//! Each struct binds to a file under `templates/`; Askama type-checks the
//! template against the struct at compile time. `WebTemplate` (from
//! `askama_web`) makes each one an Axum response that renders to HTML.

use askama::Template;
use askama_web::WebTemplate;

use crate::webui::queries::{DashboardStats, MessageExplanation, RecentMessage};

/// The Berger version stamped in every page footer.
const BERGER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The `/` dashboard page.
#[derive(Template, WebTemplate)]
#[template(path = "index.html")]
pub struct IndexPage {
    /// Which nav tab to highlight.
    pub active: &'static str,
    /// Berger version, for the footer.
    pub berger_version: &'static str,
    /// The headline counters.
    pub stats: DashboardStats,
    /// LLM cost, pre-formatted to four decimal places.
    pub cost_usd: String,
    /// Cache hit rate, pre-formatted to one decimal place.
    pub cache_hit_rate: String,
}

impl IndexPage {
    /// Builds the dashboard page from its statistics.
    pub fn new(stats: DashboardStats) -> Self {
        let cost_usd = format!("{:.4}", stats.llm_cost_usd);
        let cache_hit_rate = format!("{:.1}", stats.cache_hit_rate_pct());
        Self {
            active: "home",
            berger_version: BERGER_VERSION,
            stats,
            cost_usd,
            cache_hit_rate,
        }
    }
}

/// The `/recent` page.
#[derive(Template, WebTemplate)]
#[template(path = "recent.html")]
pub struct RecentPage {
    /// Which nav tab to highlight.
    pub active: &'static str,
    /// Berger version, for the footer.
    pub berger_version: &'static str,
    /// The recently triaged messages, newest first.
    pub messages: Vec<RecentMessage>,
}

impl RecentPage {
    /// Builds the recent-messages page from its rows.
    pub fn new(messages: Vec<RecentMessage>) -> Self {
        Self {
            active: "recent",
            berger_version: BERGER_VERSION,
            messages,
        }
    }
}

/// The `/explain/<id>` page.
#[derive(Template, WebTemplate)]
#[template(path = "explain.html")]
pub struct ExplainPage {
    /// Which nav tab to highlight (none — reached from `/recent`).
    pub active: &'static str,
    /// Berger version, for the footer.
    pub berger_version: &'static str,
    /// The full triage of the message.
    pub explanation: MessageExplanation,
}

impl ExplainPage {
    /// Builds the explain page from one message's reconstructed triage.
    pub fn new(explanation: MessageExplanation) -> Self {
        Self {
            active: "",
            berger_version: BERGER_VERSION,
            explanation,
        }
    }
}

/// The `/config` page.
#[derive(Template, WebTemplate)]
#[template(path = "config.html")]
pub struct ConfigPage {
    /// Which nav tab to highlight.
    pub active: &'static str,
    /// Berger version, for the footer.
    pub berger_version: &'static str,
    /// The active YAML, with every secret value redacted.
    pub config_yaml: String,
}

impl ConfigPage {
    /// Builds the config page, redacting secrets from `raw_yaml` first.
    pub fn new(raw_yaml: &str) -> Self {
        Self {
            active: "config",
            berger_version: BERGER_VERSION,
            config_yaml: redact_secrets(raw_yaml),
        }
    }
}

/// The YAML keys whose values are secrets and must never be shown.
const SECRET_KEYS: [&str; 3] = ["password", "api_token", "api_key"];

/// Replaces the value of every secret-bearing YAML key with `<redacted>`,
/// so the `/config` page can show the configuration's shape without ever
/// leaking a literal credential (CLAUDE.md §3.5, §7.2).
///
/// The key name and indentation are preserved; only the scalar value is
/// masked. A `${ENV_VAR}` reference is masked too — harmless, and it keeps
/// the rule blunt and predictable.
fn redact_secrets(yaml: &str) -> String {
    let mut out = String::with_capacity(yaml.len());
    for line in yaml.lines() {
        out.push_str(&redact_line(line));
        out.push('\n');
    }
    out
}

/// Redacts one YAML line if it assigns a secret-bearing key.
fn redact_line(line: &str) -> String {
    let trimmed = line.trim_start();
    let indent_len = line.len() - trimmed.len();

    // The key is the text before the first colon; a value follows it.
    let Some(colon) = trimmed.find(':') else {
        return line.to_string();
    };
    let key = trimmed[..colon].trim();
    let after_colon = &trimmed[colon + 1..];

    let is_secret = SECRET_KEYS
        .iter()
        .any(|secret| key.eq_ignore_ascii_case(secret));
    if !is_secret || after_colon.trim().is_empty() {
        // Not a secret, or a mapping header with no inline value.
        return line.to_string();
    }
    format!("{}{key}: \"<redacted>\"", &line[..indent_len])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_an_imap_password() {
        let yaml = "    password: \"super-secret-pw\"";
        let redacted = redact_secrets(yaml);
        assert!(!redacted.contains("super-secret-pw"));
        assert!(redacted.contains("password: \"<redacted>\""));
        // Indentation is preserved.
        assert!(redacted.starts_with("    password:"));
    }

    #[test]
    fn redacts_an_api_token_and_an_api_key() {
        let yaml = "api_token: tok-123\napi_key: key-abc";
        let redacted = redact_secrets(yaml);
        assert!(!redacted.contains("tok-123"));
        assert!(!redacted.contains("key-abc"));
    }

    #[test]
    fn redacts_an_env_var_placeholder_too() {
        // A ${VAR} reference is not itself a secret, but the rule masks it
        // anyway — blunt and predictable beats clever.
        let yaml = "  api_token: \"${HERMES_TOKEN}\"";
        let redacted = redact_secrets(yaml);
        assert!(!redacted.contains("HERMES_TOKEN"));
        assert!(redacted.contains("<redacted>"));
    }

    #[test]
    fn leaves_non_secret_lines_untouched() {
        let yaml = "bichon:\n  base_url: \"https://bichon.example\"\n  host: imap.example";
        let redacted = redact_secrets(yaml);
        assert!(redacted.contains("base_url: \"https://bichon.example\""));
        assert!(redacted.contains("host: imap.example"));
    }

    #[test]
    fn leaves_a_bare_mapping_header_untouched() {
        // `password:` with no inline value (an unusual but valid shape)
        // must not be turned into a redacted scalar.
        let yaml = "password:\n  nested: value";
        let redacted = redact_secrets(yaml);
        assert!(redacted.contains("password:\n"));
        assert!(redacted.contains("nested: value"));
    }

    #[test]
    fn does_not_redact_a_key_that_merely_contains_a_secret_word() {
        // `password_hint` is not `password`; only exact key names redact.
        let yaml = "password_hint: my-cat-name";
        let redacted = redact_secrets(yaml);
        assert!(redacted.contains("my-cat-name"));
    }

    #[test]
    fn redaction_preserves_the_line_count() {
        let yaml = "a: 1\npassword: x\nb: 2";
        let redacted = redact_secrets(yaml);
        assert_eq!(redacted.lines().count(), 3);
    }
}
