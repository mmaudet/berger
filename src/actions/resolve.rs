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

//! Resolves a message's tags into the IMAP actions to apply (PRD §5.5).

use std::collections::BTreeMap;

use crate::actions::Action;
use crate::config::TagActions;

/// Resolves the IMAP actions for a message's `tags` from the `actions:`
/// configuration (PRD §5.5).
///
/// A tag with no entry — or an entry with no primitive — contributes
/// nothing. The result is left unconsolidated; [`apply_actions`] dedups it
/// and resolves `move_to` / `copy_to` conflicts. The `webhook` primitive
/// is not yet an [`Action`] (webhooks arrive at their own milestone) and
/// is skipped here.
///
/// [`apply_actions`]: crate::actions::apply_actions
pub fn resolve_actions(tags: &[String], actions: &BTreeMap<String, TagActions>) -> Vec<Action> {
    let mut resolved = Vec::new();
    for tag in tags {
        let Some(tag_actions) = actions.get(tag) else {
            continue;
        };
        if let Some(folder) = &tag_actions.copy_to {
            resolved.push(Action::CopyTo(folder.clone()));
        }
        if let Some(folder) = &tag_actions.move_to {
            resolved.push(Action::MoveTo(folder.clone()));
        }
        if tag_actions.mark_seen {
            resolved.push(Action::MarkSeen);
        }
        if tag_actions.mark_flagged {
            resolved.push(Action::MarkFlagged);
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(entries: &[(&str, TagActions)]) -> BTreeMap<String, TagActions> {
        entries
            .iter()
            .map(|(tag, actions)| (tag.to_string(), actions.clone()))
            .collect()
    }

    #[test]
    fn resolves_a_tag_to_its_actions() {
        let actions = config(&[(
            "cat/urgent",
            TagActions {
                copy_to: Some("urgent".to_string()),
                mark_flagged: true,
                ..TagActions::default()
            },
        )]);
        let resolved = resolve_actions(&["cat/urgent".to_string()], &actions);
        assert_eq!(
            resolved,
            [Action::CopyTo("urgent".to_string()), Action::MarkFlagged]
        );
    }

    #[test]
    fn an_unconfigured_tag_resolves_to_nothing() {
        let actions = config(&[]);
        assert!(resolve_actions(&["cat/unknown".to_string()], &actions).is_empty());
    }

    #[test]
    fn collects_actions_across_several_tags() {
        let actions = config(&[
            (
                "newsletter",
                TagActions {
                    move_to: Some("news".to_string()),
                    ..TagActions::default()
                },
            ),
            (
                "cat/work",
                TagActions {
                    copy_to: Some("work".to_string()),
                    ..TagActions::default()
                },
            ),
        ]);
        let resolved = resolve_actions(
            &["newsletter".to_string(), "cat/work".to_string()],
            &actions,
        );
        assert_eq!(resolved.len(), 2);
    }

    #[test]
    fn the_webhook_primitive_is_skipped() {
        let actions = config(&[(
            "cat/urgent",
            TagActions {
                webhook: Some("hermes".to_string()),
                ..TagActions::default()
            },
        )]);
        assert!(resolve_actions(&["cat/urgent".to_string()], &actions).is_empty());
    }
}
