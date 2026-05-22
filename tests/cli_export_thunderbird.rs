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

//! Integration test for `berger export-thunderbird` (PRD §5.7, §10): the
//! `actions:` configuration is exported as a Thunderbird `msgFilterRules.dat`
//! ruleset, one "Move to folder" rule per tag with a destination folder.

use berger::cli::export_thunderbird::render_filter_rules;
use berger::config::BergerConfig;

const CONFIG_WITH_ACTIONS: &str = r#"
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
      user: "berger@linagora.example"
      password: "pw"
actions:
  newsletter:
    move_to: "newsletters"
    mark_seen: true
  cat/urgent:
    copy_to: "urgent"
    mark_flagged: true
  needs-reply:
    mark_flagged: true
"#;

#[test]
fn renders_a_thunderbird_ruleset_header() {
    let config = BergerConfig::parse(CONFIG_WITH_ACTIONS).unwrap();
    let rendered = render_filter_rules(&config.actions, &config.accounts[0]);
    // Thunderbird msgFilterRules.dat starts with a version and logging line.
    assert!(
        rendered.starts_with("version=\"9\"\nlogging=\"no\"\n"),
        "ruleset must begin with the Thunderbird header, got: {rendered}"
    );
}

#[test]
fn emits_one_move_to_folder_rule_per_tag_with_a_destination() {
    let config = BergerConfig::parse(CONFIG_WITH_ACTIONS).unwrap();
    let rendered = render_filter_rules(&config.actions, &config.accounts[0]);

    // newsletter (move_to) and cat/urgent (copy_to) both have a destination
    // folder; needs-reply has only mark_flagged, so it yields no rule.
    let rule_count = rendered.matches("action=\"Move to folder\"").count();
    assert_eq!(rule_count, 2, "two tags route to a folder, got: {rendered}");
    assert!(rendered.contains("name=\"Berger: newsletter\""));
    assert!(rendered.contains("name=\"Berger: cat/urgent\""));
    assert!(
        !rendered.contains("needs-reply"),
        "a tag with no destination folder must not produce a rule"
    );
}

#[test]
fn a_rule_matches_the_x_berger_tags_header_and_targets_the_berger_folder() {
    let config = BergerConfig::parse(CONFIG_WITH_ACTIONS).unwrap();
    let rendered = render_filter_rules(&config.actions, &config.accounts[0]);

    // The condition keys off the X-Berger-Tags header Berger records per
    // message (PRD §5.4); the actionValue is an IMAP URI for the account.
    assert!(
        rendered.contains(r#"condition="AND (\"x-berger-tags\",contains,newsletter)""#),
        "got: {rendered}"
    );
    assert!(
        rendered.contains(
            r#"actionValue="imap://berger@linagora.example@imap.linagora.example/Berger/newsletters""#
        ),
        "got: {rendered}"
    );
    assert!(rendered.contains("enabled=\"yes\""));
    assert!(rendered.contains("type=\"17\""));
}

#[test]
fn a_config_with_no_routing_actions_still_renders_a_valid_header() {
    let config = BergerConfig::parse(
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
      host: "imap.example"
      user: "berger"
      password: "pw"
"#,
    )
    .unwrap();
    let rendered = render_filter_rules(&config.actions, &config.accounts[0]);
    assert_eq!(rendered, "version=\"9\"\nlogging=\"no\"\n");
}
