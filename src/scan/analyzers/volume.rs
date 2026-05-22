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

//! Hourly-volume analysis: dimension 10 of the scan (PRD v1.1 §4.2) — a
//! histogram of inbox messages by UTC hour-of-day, derived from the
//! `Date:` header, used to recommend a polling interval.

use std::cmp::Reverse;

use crate::ingest::types::Envelope;

/// Dimension 10: how the inbox's mail is spread across the hours of the day.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VolumeProfile {
    /// Messages per UTC hour-of-day — exactly 24 entries, index `0..=23`.
    pub hourly: Vec<usize>,
    /// The hour with the most messages (the lowest, on a tie).
    pub busiest_hour: u32,
    /// How many messages fell in [`busiest_hour`](Self::busiest_hour).
    pub peak_hour_messages: usize,
}

/// Dimension 10: buckets the inbox by UTC hour-of-day from each message's
/// `Date:` and reports the histogram, the busiest hour, and its volume.
pub fn analyze_volume(inbox: &[&Envelope]) -> VolumeProfile {
    let mut hourly = vec![0_usize; 24];
    for envelope in inbox {
        let hour = (envelope.date / 1000 / 3600).rem_euclid(24) as usize;
        hourly[hour] += 1;
    }
    let busiest_hour = hourly
        .iter()
        .enumerate()
        .max_by_key(|&(hour, &count)| (count, Reverse(hour)))
        .map_or(0, |(hour, _)| hour as u32);
    let peak_hour_messages = hourly[busiest_hour as usize];
    VolumeProfile {
        hourly,
        busiest_hour,
        peak_hour_messages,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One hour as epoch milliseconds, for placing a message in an hour.
    const HOUR_MS: i64 = 3_600_000;

    fn envelope(date_ms: i64) -> Envelope {
        Envelope {
            id: String::new(),
            message_id: String::new(),
            account_id: 1,
            account_email: None,
            mailbox_id: 1,
            mailbox_name: None,
            uid: 1,
            subject: String::new(),
            preview: String::new(),
            from: String::new(),
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            date: date_ms,
            internal_date: date_ms,
            ingest_at: date_ms,
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
    fn buckets_messages_by_hour() {
        let envelopes = [envelope(10 * HOUR_MS), envelope(14 * HOUR_MS)];
        let profile = analyze_volume(&refs(&envelopes));
        assert_eq!(profile.hourly[10], 1);
        assert_eq!(profile.hourly[14], 1);
    }

    #[test]
    fn accumulates_multiple_messages_in_one_hour() {
        let envelopes = [
            envelope(9 * HOUR_MS),
            envelope(9 * HOUR_MS),
            envelope(9 * HOUR_MS),
        ];
        assert_eq!(analyze_volume(&refs(&envelopes)).hourly[9], 3);
    }

    #[test]
    fn identifies_the_busiest_hour() {
        let envelopes = [
            envelope(9 * HOUR_MS),
            envelope(15 * HOUR_MS),
            envelope(15 * HOUR_MS),
            envelope(15 * HOUR_MS),
        ];
        let profile = analyze_volume(&refs(&envelopes));
        assert_eq!(profile.busiest_hour, 15);
        assert_eq!(profile.peak_hour_messages, 3);
    }

    #[test]
    fn a_tie_picks_the_lowest_hour() {
        let envelopes = [
            envelope(5 * HOUR_MS),
            envelope(5 * HOUR_MS),
            envelope(20 * HOUR_MS),
            envelope(20 * HOUR_MS),
        ];
        assert_eq!(analyze_volume(&refs(&envelopes)).busiest_hour, 5);
    }

    #[test]
    fn hourly_always_has_24_entries() {
        assert_eq!(analyze_volume(&[]).hourly.len(), 24);
        let envelopes = [envelope(0)];
        assert_eq!(analyze_volume(&refs(&envelopes)).hourly.len(), 24);
    }

    #[test]
    fn empty_input_is_the_zero_profile() {
        let profile = analyze_volume(&[]);
        assert!(profile.hourly.iter().all(|&count| count == 0));
        assert_eq!(profile.busiest_hour, 0);
        assert_eq!(profile.peak_hour_messages, 0);
    }
}
