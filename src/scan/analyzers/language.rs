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

//! Dominant-language detection: dimension 9 of the scan (PRD v1.1 §4.2).
//!
//! The language of the inbox is estimated from its subject lines with the
//! `whatlang` crate. To bound the cost the subjects are stride-sampled
//! (PRD §5.5) — deterministic, so the result is reproducible.

use std::collections::HashMap;

use crate::ingest::types::Envelope;

/// One detected language and its share of the sampled subjects (dimension 9).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct LanguageShare {
    /// The ISO 639-3 language code, e.g. `eng` or `fra`.
    pub language: String,
    /// The language's share of the detected subjects, in `[0, 1]`.
    pub share: f64,
}

/// Dimension 9: detects the language of a stride sample of the inbox's
/// subject lines and returns each detected language's share, the most
/// common first (ties broken by language code).
pub fn detect_languages(inbox: &[&Envelope]) -> Vec<LanguageShare> {
    if inbox.is_empty() {
        return Vec::new();
    }
    let target = if inbox.len() <= SAMPLE_FLOOR {
        inbox.len()
    } else {
        (inbox.len() / 10).max(SAMPLE_FLOOR)
    };
    let stride = (inbox.len() / target).max(1);

    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut detected = 0_usize;
    for envelope in inbox.iter().step_by(stride) {
        if let Some(info) = whatlang::detect(&envelope.subject) {
            *counts.entry(info.lang().code().to_string()).or_default() += 1;
            detected += 1;
        }
    }
    if detected == 0 {
        return Vec::new();
    }

    let mut shares: Vec<LanguageShare> = counts
        .into_iter()
        .map(|(language, count)| LanguageShare {
            language,
            share: count as f64 / detected as f64,
        })
        .collect();
    shares.sort_by(|a, b| {
        b.share
            .total_cmp(&a.share)
            .then_with(|| a.language.cmp(&b.language))
    });
    shares
}

/// The minimum number of subjects to language-detect, when the inbox has
/// at least that many (PRD v1.1 §8).
const SAMPLE_FLOOR: usize = 50;

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(subject: &str) -> Envelope {
        Envelope {
            id: String::new(),
            message_id: String::new(),
            account_id: 1,
            account_email: None,
            mailbox_id: 1,
            mailbox_name: None,
            uid: 1,
            subject: subject.to_string(),
            preview: String::new(),
            from: String::new(),
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            date: 0,
            internal_date: 0,
            ingest_at: 0,
            size: 0,
            thread_id: String::new(),
            attachment_count: 0,
            regular_attachment_count: 0,
            tags: None,
            content_hash: String::new(),
        }
    }

    fn refs(envelopes: &[Envelope]) -> Vec<&Envelope> {
        envelopes.iter().collect()
    }

    const ENGLISH: &[&str] = &[
        "The quarterly financial report is ready for your review today.",
        "Please confirm your attendance at the meeting tomorrow morning.",
        "The new software update will be released to everyone next week.",
        "Thank you very much for your help with this important project.",
        "We are pleased to announce the launch of our latest product.",
    ];

    const FRENCH: &[&str] = &[
        "Le rapport financier trimestriel est prêt pour votre examen aujourd'hui.",
        "Veuillez confirmer votre présence à la réunion de demain matin.",
        "La nouvelle mise à jour du logiciel sera publiée la semaine prochaine.",
        "Merci beaucoup pour votre aide sur ce projet vraiment important.",
        "Nous avons le plaisir de vous annoncer le lancement de notre produit.",
    ];

    #[test]
    fn an_english_inbox_is_dominated_by_english() {
        let envelopes: Vec<Envelope> = ENGLISH.iter().map(|s| envelope(s)).collect();
        let shares = detect_languages(&refs(&envelopes));
        assert_eq!(shares[0].language, "eng");
    }

    #[test]
    fn a_mixed_inbox_reports_french_and_english() {
        let mut envelopes: Vec<Envelope> = ENGLISH.iter().map(|s| envelope(s)).collect();
        envelopes.extend(FRENCH.iter().map(|s| envelope(s)));
        let shares = detect_languages(&refs(&envelopes));
        assert!(shares.iter().any(|s| s.language == "eng"));
        assert!(shares.iter().any(|s| s.language == "fra"));
    }

    #[test]
    fn shares_sum_to_one() {
        let envelopes: Vec<Envelope> = ENGLISH.iter().map(|s| envelope(s)).collect();
        let shares = detect_languages(&refs(&envelopes));
        let sum: f64 = shares.iter().map(|s| s.share).sum();
        assert!((sum - 1.0).abs() < 1e-9);
    }

    #[test]
    fn undetectable_subjects_yield_no_languages() {
        let envelopes = [envelope(""), envelope(""), envelope("")];
        assert!(detect_languages(&refs(&envelopes)).is_empty());
    }

    #[test]
    fn empty_input_yields_no_languages() {
        assert!(detect_languages(&[]).is_empty());
    }
}
