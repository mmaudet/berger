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

//! Maps an LLM [`Classification`] to triage tags (PRD §5.4).

use crate::llm::classifier::Classification;

/// The tag applied when the LLM classifier fails — the message is still
/// triaged on its native filters and flagged for review (PRD §5.3).
pub const LLM_ERROR_TAG: &str = "llm_error";

/// Derives the triage tags from an LLM classification (PRD §5.4):
/// always the hierarchical `cat/<category>`, plus `needs-reply` and
/// `priority-high` when the classification calls for them.
pub fn classification_tags(classification: &Classification) -> Vec<String> {
    let mut tags = vec![format!("cat/{}", classification.category)];
    if classification.needs_reply {
        tags.push("needs-reply".to_string());
    }
    if classification.priority >= 4 {
        tags.push("priority-high".to_string());
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classification(category: &str, needs_reply: bool, priority: u8) -> Classification {
        Classification {
            category: category.to_string(),
            needs_reply,
            priority,
        }
    }

    #[test]
    fn maps_the_category_to_a_hierarchical_cat_tag() {
        let tags = classification_tags(&classification("work", false, 2));
        assert!(tags.contains(&"cat/work".to_string()));
    }

    #[test]
    fn a_routine_message_gets_only_its_category() {
        let tags = classification_tags(&classification("newsletter", false, 1));
        assert_eq!(tags, ["cat/newsletter"]);
    }

    #[test]
    fn flags_a_message_that_needs_a_reply() {
        let tags = classification_tags(&classification("work", true, 2));
        assert!(tags.contains(&"needs-reply".to_string()));
    }

    #[test]
    fn flags_high_priority_messages() {
        assert!(
            classification_tags(&classification("work", false, 4))
                .contains(&"priority-high".to_string())
        );
        assert!(
            classification_tags(&classification("work", false, 5))
                .contains(&"priority-high".to_string())
        );
        assert!(
            !classification_tags(&classification("work", false, 3))
                .contains(&"priority-high".to_string())
        );
    }
}
