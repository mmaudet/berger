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

//! The `export-thunderbird` command: exports the `actions:` configuration
//! as a Mozilla Thunderbird `msgFilterRules.dat` ruleset (PRD §5.7, §10).
//!
//! Berger keeps its triage tags in the SQLite sidecar, recorded under the
//! `X-Berger-Tags` header (PRD §5.4) — never injected into the mail itself.
//! For users who also run Thunderbird, this command turns each tag that
//! routes to a folder into a Thunderbird "Move to folder" filter rule that
//! keys off that header, so the same foldering can be reproduced client-side.

use std::collections::BTreeMap;

use anyhow::Context;

use crate::config::{AccountConfig, BergerConfig, TagActions};

/// The header Berger records its tags under (PRD §5.4) — the Thunderbird
/// rules match against it. Thunderbird lowercases header names in
/// conditions.
const TAGS_HEADER: &str = "x-berger-tags";

/// Thunderbird filter `type` for "applies when fetching mail" (0x01) plus
/// "applies when run manually" (0x10) — the usual value for an import.
const FILTER_TYPE: &str = "17";

/// Loads the configuration and prints a Thunderbird `msgFilterRules.dat`
/// ruleset to standard output (or writes it to `output` when given).
///
/// The ruleset is generated for one account — the first configured, or the
/// one named by `account` — since Thunderbird keeps a separate
/// `msgFilterRules.dat` per account.
///
/// # Errors
/// Returns an error if the configuration cannot be loaded, the named
/// account is unknown, or the output file cannot be written.
pub fn run(config_path: &str, account: Option<&str>, output: Option<&str>) -> anyhow::Result<()> {
    let raw = std::fs::read_to_string(config_path)
        .with_context(|| format!("reading config `{config_path}`"))?;
    let config = BergerConfig::parse(&raw).context("parsing the configuration")?;

    let target = select_account(&config, account)?;
    let rendered = render_filter_rules(&config.actions, target);

    match output {
        Some(path) => {
            std::fs::write(path, &rendered)
                .with_context(|| format!("writing the Thunderbird ruleset to `{path}`"))?;
            println!(
                "Wrote {} Thunderbird filter rule(s) for account `{}` to {path}",
                rule_count(&config.actions),
                target.name
            );
        }
        None => print!("{rendered}"),
    }
    Ok(())
}

/// Picks the account to export rules for: the one named by `account`, or
/// the first configured account when no name is given.
fn select_account<'a>(
    config: &'a BergerConfig,
    account: Option<&str>,
) -> anyhow::Result<&'a AccountConfig> {
    match account {
        Some(name) => config
            .accounts
            .iter()
            .find(|candidate| candidate.name == name)
            .with_context(|| format!("no account named `{name}` in the configuration")),
        // Config validation guarantees at least one account.
        None => config
            .accounts
            .first()
            .context("the configuration declares no accounts"),
    }
}

/// Counts the tags that route to a folder — the number of rules a render
/// will emit.
fn rule_count(actions: &BTreeMap<String, TagActions>) -> usize {
    actions
        .values()
        .filter(|tag_actions| destination_folder(tag_actions).is_some())
        .count()
}

/// The destination folder for a tag, if it has one: `move_to` takes
/// precedence over `copy_to` (it is the stronger routing intent), mirroring
/// the action engine's conflict rule (PRD §5.5).
fn destination_folder(tag_actions: &TagActions) -> Option<&str> {
    tag_actions
        .move_to
        .as_deref()
        .or(tag_actions.copy_to.as_deref())
}

/// Renders the `actions:` map as a Thunderbird `msgFilterRules.dat`
/// ruleset for `account`: the standard header, then one "Move to folder"
/// rule for every tag that routes to a folder.
///
/// Tags are emitted in `BTreeMap` order, so the output is deterministic.
/// A tag whose actions are flags or webhooks only contributes no rule —
/// Thunderbird cannot reproduce those from a header match.
pub fn render_filter_rules(
    actions: &BTreeMap<String, TagActions>,
    account: &AccountConfig,
) -> String {
    let mut out = String::from("version=\"9\"\nlogging=\"no\"\n");
    for (tag, tag_actions) in actions {
        let Some(folder) = destination_folder(tag_actions) else {
            continue;
        };
        out.push_str(&render_rule(tag, folder, account));
    }
    out
}

/// Renders one "Move to folder" filter rule for `tag`.
fn render_rule(tag: &str, folder: &str, account: &AccountConfig) -> String {
    // The IMAP folder URI Thunderbird files into. Berger writes to
    // `Berger/<folder>` (PRD §5.5), so the rule targets the same path.
    let action_value = format!(
        "imap://{user}@{host}/Berger/{folder}",
        user = account.imap.user,
        host = account.imap.host,
    );
    format!(
        "name=\"Berger: {tag}\"\n\
         enabled=\"yes\"\n\
         type=\"{FILTER_TYPE}\"\n\
         action=\"Move to folder\"\n\
         actionValue=\"{action_value}\"\n\
         condition=\"AND (\\\"{TAGS_HEADER}\\\",contains,{tag})\"\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ImapConfig;

    fn account() -> AccountConfig {
        // SecretString has no public constructor; round-trip it through serde.
        let imap: ImapConfig =
            serde_yaml_ng::from_str("host: imap.example\nuser: berger@example\npassword: secret\n")
                .unwrap();
        AccountConfig {
            name: "Test".to_string(),
            bichon_account_id: "1".to_string(),
            imap,
        }
    }

    fn tag_actions(copy_to: Option<&str>, move_to: Option<&str>, flagged: bool) -> TagActions {
        TagActions {
            copy_to: copy_to.map(str::to_string),
            move_to: move_to.map(str::to_string),
            mark_seen: false,
            mark_flagged: flagged,
            webhook: None,
        }
    }

    #[test]
    fn destination_folder_prefers_move_to_over_copy_to() {
        let actions = tag_actions(Some("copied"), Some("moved"), false);
        assert_eq!(destination_folder(&actions), Some("moved"));
    }

    #[test]
    fn destination_folder_falls_back_to_copy_to() {
        let actions = tag_actions(Some("copied"), None, false);
        assert_eq!(destination_folder(&actions), Some("copied"));
    }

    #[test]
    fn destination_folder_is_none_without_routing() {
        let actions = tag_actions(None, None, true);
        assert_eq!(destination_folder(&actions), None);
    }

    #[test]
    fn rule_count_counts_only_routing_tags() {
        let mut actions = BTreeMap::new();
        actions.insert("a".to_string(), tag_actions(Some("x"), None, false));
        actions.insert("b".to_string(), tag_actions(None, Some("y"), false));
        actions.insert("c".to_string(), tag_actions(None, None, true));
        assert_eq!(rule_count(&actions), 2);
    }

    #[test]
    fn render_rule_quotes_the_header_name_in_the_condition() {
        let rule = render_rule("cat/work", "work", &account());
        assert!(rule.contains(r#"condition="AND (\"x-berger-tags\",contains,cat/work)""#));
        assert!(rule.contains("action=\"Move to folder\""));
        assert!(rule.contains("actionValue=\"imap://berger@example@imap.example/Berger/work\""));
    }

    #[test]
    fn render_filter_rules_is_deterministic_in_btreemap_order() {
        let mut actions = BTreeMap::new();
        actions.insert("zeta".to_string(), tag_actions(Some("z"), None, false));
        actions.insert("alpha".to_string(), tag_actions(Some("a"), None, false));
        let rendered = render_filter_rules(&actions, &account());
        let alpha_at = rendered.find("Berger: alpha").unwrap();
        let zeta_at = rendered.find("Berger: zeta").unwrap();
        assert!(alpha_at < zeta_at, "rules must be sorted by tag");
    }

    #[test]
    fn select_account_returns_the_first_account_by_default() {
        let config = sample_config();
        let chosen = select_account(&config, None).unwrap();
        assert_eq!(chosen.name, "LINAGORA");
    }

    #[test]
    fn select_account_finds_a_named_account() {
        let config = sample_config();
        let chosen = select_account(&config, Some("Gmail")).unwrap();
        assert_eq!(chosen.name, "Gmail");
    }

    #[test]
    fn select_account_rejects_an_unknown_name() {
        let config = sample_config();
        assert!(select_account(&config, Some("nope")).is_err());
    }

    fn sample_config() -> BergerConfig {
        BergerConfig::parse(
            r#"
bichon:
  base_url: "https://bichon.example"
  api_token: "tok"
database:
  path: "berger.db"
accounts:
  - name: "LINAGORA"
    bichon_account_id: "111"
    imap:
      host: "imap.linagora.example"
      user: "berger"
      password: "pw"
  - name: "Gmail"
    bichon_account_id: "222"
    imap:
      host: "imap.gmail.com"
      user: "berger@gmail.com"
      password: "pw2"
"#,
        )
        .unwrap()
    }
}
