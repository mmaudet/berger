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

//! Incremental polling.
//!
//! Bichon exposes no ingest-time cursor, so Berger tracks its own
//! high-water mark over the message `Date:` field. Each poll re-queries a
//! safety margin below the watermark; duplicates are dropped downstream by
//! the Message-ID idempotency check (PRD §5.1).

use crate::ingest::error::IngestError;
use crate::ingest::folder_filter::is_berger_folder;
use crate::ingest::source::MessageSource;
use crate::ingest::types::{EmailSearchFilter, EmailSearchRequest, Envelope, SortBy};

/// Subtracted from the watermark on every poll so a message that arrives
/// slightly out of `Date:` order is still swept up. 48 hours.
const SAFETY_MARGIN_MS: i64 = 48 * 60 * 60 * 1000;

/// Page size requested from `search-messages`. Bichon caps this at 500.
const PAGE_SIZE: u64 = 200;

/// Current wall-clock time as epoch milliseconds.
fn now_epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// Per-account incremental-polling watermark: the most recent message
/// `Date:` (epoch milliseconds) Berger has swept up to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Watermark(i64);

impl Watermark {
    /// A watermark for an account Berger has never polled — anchored at the
    /// current time, so existing history is not back-filled (PRD §6).
    pub fn starting_now() -> Self {
        Self(now_epoch_ms())
    }

    /// A watermark restored from a persisted epoch-millisecond value.
    pub fn at(epoch_ms: i64) -> Self {
        Self(epoch_ms)
    }

    /// The watermark as epoch milliseconds, for persistence.
    pub fn as_epoch_ms(self) -> i64 {
        self.0
    }
}

/// The result of polling one account once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollOutcome {
    /// New envelopes to hand to the pipeline. Messages in Berger's own
    /// `Berger/*` folders are already excluded (Bichon coherence rule #1).
    pub envelopes: Vec<Envelope>,
    /// The watermark to persist for this account's next poll.
    pub watermark: Watermark,
}

/// Polls one account once: pages through every message whose `Date:` is at
/// or after `watermark - SAFETY_MARGIN_MS`, drops Berger's own folders, and
/// returns the new envelopes together with the advanced watermark.
pub async fn poll_account<S: MessageSource>(
    source: &S,
    account_id: u64,
    watermark: Watermark,
) -> Result<PollOutcome, IngestError> {
    let since = watermark.as_epoch_ms().saturating_sub(SAFETY_MARGIN_MS);

    let mut envelopes: Vec<Envelope> = Vec::new();
    let mut page = 1;
    loop {
        let request = EmailSearchRequest {
            filter: EmailSearchFilter {
                since: Some(since),
                account_ids: Some(vec![account_id]),
            },
            page,
            page_size: PAGE_SIZE,
            sort_by: Some(SortBy::Date),
            desc: Some(false),
        };
        let result = source.search_messages(request).await?;
        let is_last_page = (result.items.len() as u64) < PAGE_SIZE;
        envelopes.extend(result.items);
        if is_last_page {
            break;
        }
        page += 1;
    }

    // The watermark advances past every message Bichon returned, including
    // Berger's own folders, so they are not re-fetched on the next poll.
    let new_watermark = match envelopes.iter().map(|envelope| envelope.date).max() {
        Some(latest) => Watermark::at(watermark.as_epoch_ms().max(latest)),
        None => watermark,
    };

    // Bichon coherence rule #1: never hand Berger's own folders downstream.
    envelopes
        .retain(|envelope| !is_berger_folder(envelope.mailbox_name.as_deref().unwrap_or_default()));

    tracing::debug!(
        account_id,
        pages = page,
        new_messages = envelopes.len(),
        watermark = new_watermark.as_epoch_ms(),
        "polled account"
    );

    Ok(PollOutcome {
        envelopes,
        watermark: new_watermark,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::types::{DataPage, MinimalAccount};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// In-memory `MessageSource`: hands out pre-canned pages in order and
    /// records every search request it received.
    struct FakeSource {
        pages: Mutex<VecDeque<DataPage<Envelope>>>,
        requests: Mutex<Vec<EmailSearchRequest>>,
    }

    impl FakeSource {
        fn new(pages: Vec<DataPage<Envelope>>) -> Self {
            Self {
                pages: Mutex::new(pages.into()),
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    impl MessageSource for FakeSource {
        async fn list_accounts(&self) -> Result<Vec<MinimalAccount>, IngestError> {
            Ok(Vec::new())
        }

        async fn search_messages(
            &self,
            request: EmailSearchRequest,
        ) -> Result<DataPage<Envelope>, IngestError> {
            self.requests.lock().unwrap().push(request);
            let page = self
                .pages
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(empty_page);
            Ok(page)
        }
    }

    fn empty_page() -> DataPage<Envelope> {
        DataPage {
            current_page: None,
            page_size: Some(PAGE_SIZE),
            total_items: 0,
            items: Vec::new(),
            total_pages: Some(0),
        }
    }

    fn page(items: Vec<Envelope>) -> DataPage<Envelope> {
        DataPage {
            current_page: None,
            page_size: Some(PAGE_SIZE),
            total_items: items.len() as u64,
            items,
            total_pages: None,
        }
    }

    fn envelope(message_id: &str, date: i64, mailbox_name: &str) -> Envelope {
        Envelope {
            id: format!("id-{message_id}"),
            message_id: message_id.to_string(),
            account_id: 1,
            account_email: None,
            mailbox_id: 1,
            mailbox_name: Some(mailbox_name.to_string()),
            uid: 1,
            subject: String::new(),
            preview: String::new(),
            from: String::new(),
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            date,
            internal_date: date,
            ingest_at: date,
            size: 0,
            thread_id: String::new(),
            attachment_count: 0,
            regular_attachment_count: 0,
            tags: None,
            content_hash: String::new(),
        }
    }

    /// A page that is exactly `PAGE_SIZE` long, so the poller keeps paging.
    fn full_page() -> DataPage<Envelope> {
        let mut items = Vec::new();
        while (items.len() as u64) < PAGE_SIZE {
            let n = items.len();
            items.push(envelope(&format!("<filler-{n}@x>"), 1, "INBOX"));
        }
        page(items)
    }

    #[tokio::test]
    async fn collects_envelopes_across_pages_until_a_short_page() {
        let source = FakeSource::new(vec![
            full_page(),
            page(vec![envelope("<real@x>", 5, "INBOX")]),
        ]);
        let outcome = poll_account(&source, 1, Watermark::at(1_000))
            .await
            .unwrap();
        assert_eq!(outcome.envelopes.len() as u64, PAGE_SIZE + 1);
        assert!(outcome.envelopes.iter().any(|e| e.message_id == "<real@x>"));
        assert_eq!(source.requests.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn advances_the_watermark_to_the_latest_date() {
        let source = FakeSource::new(vec![page(vec![
            envelope("<a@x>", 100, "INBOX"),
            envelope("<b@x>", 900, "INBOX"),
            envelope("<c@x>", 400, "INBOX"),
        ])]);
        let outcome = poll_account(&source, 1, Watermark::at(50)).await.unwrap();
        assert_eq!(outcome.watermark, Watermark::at(900));
    }

    #[tokio::test]
    async fn keeps_the_watermark_when_no_messages_are_returned() {
        let source = FakeSource::new(vec![empty_page()]);
        let outcome = poll_account(&source, 1, Watermark::at(777)).await.unwrap();
        assert_eq!(outcome.watermark, Watermark::at(777));
        assert!(outcome.envelopes.is_empty());
    }

    #[tokio::test]
    async fn excludes_messages_in_bergers_own_folders() {
        let source = FakeSource::new(vec![page(vec![
            envelope("<inbox@x>", 10, "INBOX"),
            envelope("<copied@x>", 20, "Berger/cat-work"),
            envelope("<moved@x>", 30, "INBOX.Berger.junk"),
        ])]);
        let outcome = poll_account(&source, 1, Watermark::at(0)).await.unwrap();
        assert_eq!(outcome.envelopes.len(), 1);
        assert_eq!(outcome.envelopes[0].message_id, "<inbox@x>");
    }

    #[tokio::test]
    async fn advances_the_watermark_past_berger_folders_too() {
        // The newest message is one of Berger's own; the watermark must
        // still move past it so it is not re-fetched on every poll.
        let source = FakeSource::new(vec![page(vec![
            envelope("<inbox@x>", 10, "INBOX"),
            envelope("<copied@x>", 5_000, "Berger/cat-work"),
        ])]);
        let outcome = poll_account(&source, 1, Watermark::at(0)).await.unwrap();
        assert_eq!(outcome.watermark, Watermark::at(5_000));
        assert_eq!(outcome.envelopes.len(), 1);
    }

    #[tokio::test]
    async fn queries_with_since_a_safety_margin_below_the_watermark() {
        let source = FakeSource::new(vec![empty_page()]);
        poll_account(&source, 4242, Watermark::at(1_000_000_000_000))
            .await
            .unwrap();

        let requests = source.requests.lock().unwrap();
        assert_eq!(
            requests[0].filter.since,
            Some(1_000_000_000_000 - SAFETY_MARGIN_MS)
        );
        assert_eq!(requests[0].filter.account_ids, Some(vec![4242]));
        assert_eq!(requests[0].sort_by, Some(SortBy::Date));
    }
}
