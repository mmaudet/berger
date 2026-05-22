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

//! Subject n-gram analysis: dimension 8 of the scan (PRD v1.1 §4.2) — the
//! recurring word patterns in inbox subject lines, after stopwords are
//! removed, as 2- and 3-word phrases.

use std::collections::HashMap;

use crate::ingest::types::Envelope;

/// Dimension 8: how many of the most frequent subject phrases to report.
const TOP_NGRAMS: usize = 30;

/// One recurring subject phrase and how often it appeared (dimension 8).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SubjectNgram {
    /// The phrase — 2 or 3 words joined by single spaces.
    pub phrase: String,
    /// Total occurrences of the phrase across the scanned subjects.
    pub occurrences: usize,
}

/// Dimension 8: extracts 2- and 3-word n-grams from inbox subject lines
/// (after dropping stopwords and very short tokens) and returns the most
/// frequent phrases, busiest first (ties broken by phrase).
pub fn top_subject_ngrams(inbox: &[&Envelope]) -> Vec<SubjectNgram> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for envelope in inbox {
        let tokens = clean_tokens(&envelope.subject);
        for size in 2..=3 {
            for window in tokens.windows(size) {
                *counts.entry(window.join(" ")).or_default() += 1;
            }
        }
    }
    let mut ngrams: Vec<SubjectNgram> = counts
        .into_iter()
        .map(|(phrase, occurrences)| SubjectNgram {
            phrase,
            occurrences,
        })
        .collect();
    ngrams.sort_by(|a, b| {
        b.occurrences
            .cmp(&a.occurrences)
            .then_with(|| a.phrase.cmp(&b.phrase))
    });
    ngrams.truncate(TOP_NGRAMS);
    ngrams
}

/// Tokens shorter than this are dropped before n-gram extraction.
const MIN_TOKEN_LEN: usize = 3;

/// Common French and English stopwords (three letters or more — shorter
/// words are already dropped by [`MIN_TOKEN_LEN`]).
const STOPWORDS: &[&str] = &[
    "les", "des", "une", "aux", "dans", "pour", "par", "sur", "avec", "sans", "cette", "ces",
    "que", "qui", "vous", "votre", "vos", "nous", "notre", "est", "son", "ses", "the", "and",
    "for", "with", "are", "this", "that", "your", "you", "our", "from", "has", "have", "was",
    "will", "fwd",
];

/// Lowercases `subject`, splits it on non-alphanumeric characters, and
/// keeps the tokens that are neither stopwords nor shorter than
/// [`MIN_TOKEN_LEN`].
fn clean_tokens(subject: &str) -> Vec<String> {
    subject
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.len() >= MIN_TOKEN_LEN && !STOPWORDS.contains(token))
        .map(str::to_string)
        .collect()
}

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

    #[test]
    fn counts_a_repeated_two_gram() {
        let envelopes = [
            envelope("Quarterly invoice ready"),
            envelope("Quarterly invoice attached"),
        ];
        let ngrams = top_subject_ngrams(&refs(&envelopes));
        let invoice = ngrams
            .iter()
            .find(|ngram| ngram.phrase == "quarterly invoice")
            .expect("the repeated 2-gram should be found");
        assert_eq!(invoice.occurrences, 2);
    }

    #[test]
    fn produces_three_grams() {
        let envelopes = [envelope("monthly status report summary")];
        let ngrams = top_subject_ngrams(&refs(&envelopes));
        assert!(
            ngrams
                .iter()
                .any(|ngram| ngram.phrase == "monthly status report")
        );
    }

    #[test]
    fn drops_stopwords() {
        let envelopes = [envelope("the new release"), envelope("the new release")];
        let ngrams = top_subject_ngrams(&refs(&envelopes));
        // "the" is a stopword: only "new release" survives as a phrase.
        assert!(ngrams.iter().any(|ngram| ngram.phrase == "new release"));
        assert!(!ngrams.iter().any(|ngram| ngram.phrase.contains("the")));
    }

    #[test]
    fn a_one_word_subject_yields_nothing() {
        let envelopes = [envelope("hello")];
        assert!(top_subject_ngrams(&refs(&envelopes)).is_empty());
    }

    #[test]
    fn orders_by_frequency() {
        let envelopes = [
            envelope("alpha beta"),
            envelope("alpha beta"),
            envelope("gamma delta"),
        ];
        let ngrams = top_subject_ngrams(&refs(&envelopes));
        assert_eq!(ngrams[0].phrase, "alpha beta");
        assert_eq!(ngrams[0].occurrences, 2);
    }

    #[test]
    fn is_capped() {
        let envelopes: Vec<Envelope> = (0..TOP_NGRAMS + 10)
            .map(|n| envelope(&format!("topic{n} subject{n}")))
            .collect();
        assert_eq!(top_subject_ngrams(&refs(&envelopes)).len(), TOP_NGRAMS);
    }

    #[test]
    fn empty_input_yields_no_ngrams() {
        assert!(top_subject_ngrams(&[]).is_empty());
    }
}
