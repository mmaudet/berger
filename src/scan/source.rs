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

//! The scan's read layer: a read-only view of the upstream message
//! source, and a paginated fetch over a recent time window.

use std::future::Future;

use crate::ingest::error::IngestError;
use crate::ingest::source::MessageSource;
use crate::ingest::types::{DataPage, EmailSearchFilter, EmailSearchRequest, Envelope, SortBy};

/// Page size requested from Bichon's `search-messages`. Bichon caps it at 500.
const PAGE_SIZE: u64 = 200;

/// A message source the scan is allowed to READ, and only read.
///
/// The scan is generic over this trait and never over [`MessageSource`],
/// so the type system forbids it from ever reaching an IMAP write — this
/// trait simply exposes no mutating method (PRD v1.1 §4.4). The blanket
/// impl below makes every [`MessageSource`] (the Bichon client, the test
/// fakes) usable as a read-only source at no cost.
pub trait ReadOnlyMessageSource {
    /// Runs one page of a message search.
    fn search_messages(
        &self,
        request: EmailSearchRequest,
    ) -> impl Future<Output = Result<DataPage<Envelope>, IngestError>> + Send;

    /// Downloads the raw RFC 822 bytes of one message.
    fn download_message(
        &self,
        account_id: &str,
        envelope_id: &str,
    ) -> impl Future<Output = Result<Vec<u8>, IngestError>> + Send;
}

impl<T: MessageSource> ReadOnlyMessageSource for T {
    fn search_messages(
        &self,
        request: EmailSearchRequest,
    ) -> impl Future<Output = Result<DataPage<Envelope>, IngestError>> + Send {
        MessageSource::search_messages(self, request)
    }

    fn download_message(
        &self,
        account_id: &str,
        envelope_id: &str,
    ) -> impl Future<Output = Result<Vec<u8>, IngestError>> + Send {
        MessageSource::download_message(self, account_id, envelope_id)
    }
}

/// Fetches every envelope whose `Date:` is at or after `since` (epoch
/// milliseconds), for the given Bichon account ids, paging through the
/// search endpoint until a short page closes the window.
///
/// The result spans every folder Bichon indexes; the caller classifies
/// them (INBOX, Sent, ...) afterwards. An empty `account_ids` yields an
/// empty result and issues no request.
///
/// # Errors
/// Returns [`IngestError`] if a search page fails.
pub async fn fetch_window<S: ReadOnlyMessageSource>(
    source: &S,
    account_ids: &[u64],
    since: i64,
) -> Result<Vec<Envelope>, IngestError> {
    if account_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut envelopes: Vec<Envelope> = Vec::new();
    let mut page = 1;
    loop {
        let request = EmailSearchRequest {
            filter: EmailSearchFilter {
                since: Some(since),
                account_ids: Some(account_ids.to_vec()),
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
    Ok(envelopes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::types::MinimalAccount;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// In-memory source: hands out pre-canned pages in order and records
    /// every search request it received.
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
            Ok(self
                .pages
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(empty_page))
        }

        async fn download_message(
            &self,
            _account_id: &str,
            _envelope_id: &str,
        ) -> Result<Vec<u8>, IngestError> {
            Ok(Vec::new())
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

    fn empty_page() -> DataPage<Envelope> {
        page(Vec::new())
    }

    fn envelope(message_id: &str) -> Envelope {
        Envelope {
            id: format!("id-{message_id}"),
            message_id: message_id.to_string(),
            account_id: 1,
            account_email: None,
            mailbox_id: 1,
            mailbox_name: Some("INBOX".to_string()),
            uid: 1,
            subject: String::new(),
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

    /// A page exactly `PAGE_SIZE` long, so the fetch keeps paging.
    fn full_page() -> DataPage<Envelope> {
        let items = (0..PAGE_SIZE)
            .map(|n| envelope(&format!("<filler-{n}@x>")))
            .collect();
        page(items)
    }

    #[tokio::test]
    async fn fetches_envelopes_across_pages_until_a_short_page() {
        let source = FakeSource::new(vec![full_page(), page(vec![envelope("<last@x>")])]);
        let envelopes = fetch_window(&source, &[1], 0).await.unwrap();
        assert_eq!(envelopes.len() as u64, PAGE_SIZE + 1);
        assert!(envelopes.iter().any(|e| e.message_id == "<last@x>"));
        assert_eq!(source.requests.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn passes_the_window_and_accounts_to_the_search() {
        let source = FakeSource::new(vec![empty_page()]);
        fetch_window(&source, &[42, 99], 1_700_000_000_000)
            .await
            .unwrap();
        let requests = source.requests.lock().unwrap();
        assert_eq!(requests[0].filter.since, Some(1_700_000_000_000));
        assert_eq!(requests[0].filter.account_ids, Some(vec![42, 99]));
        assert_eq!(requests[0].sort_by, Some(SortBy::Date));
    }

    #[tokio::test]
    async fn an_empty_window_returns_no_envelopes() {
        let source = FakeSource::new(vec![empty_page()]);
        let envelopes = fetch_window(&source, &[1], 0).await.unwrap();
        assert!(envelopes.is_empty());
    }

    #[tokio::test]
    async fn empty_account_ids_make_no_request() {
        let source = FakeSource::new(Vec::new());
        let envelopes = fetch_window(&source, &[], 0).await.unwrap();
        assert!(envelopes.is_empty());
        assert_eq!(source.requests.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn download_message_is_reachable_through_the_read_only_trait() {
        let source = FakeSource::new(Vec::new());
        let eml = ReadOnlyMessageSource::download_message(&source, "1", "e1")
            .await
            .unwrap();
        assert!(eml.is_empty());
    }
}
