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

//! The scan runner: fetch the window, download each inbox message's
//! headers, and analyze — the read-only I/O orchestration behind
//! `berger scan`.

use crate::ingest::error::IngestError;
use crate::scan::analyzer::{ScanReport, ScannedMessage, analyze, partition};
use crate::scan::headers::{ScanHeaders, parse_headers};
use crate::scan::source::{ReadOnlyMessageSource, fetch_window};

/// Runs a full inbox scan: fetches every envelope in the `since` window,
/// downloads each INBOX message's raw eml to read its technical headers,
/// and analyzes the result into a [`ScanReport`].
///
/// A message whose download fails is kept with empty headers and logged;
/// it never aborts the scan.
///
/// # Errors
/// Returns [`IngestError`] if the initial windowed fetch fails.
pub async fn scan<S: ReadOnlyMessageSource>(
    source: &S,
    account_ids: &[u64],
    since: i64,
) -> Result<ScanReport, IngestError> {
    let envelopes = fetch_window(source, account_ids, since).await?;
    let (inbox_envelopes, sent) = partition(&envelopes);

    let mut inbox: Vec<ScannedMessage> = Vec::with_capacity(inbox_envelopes.len());
    for envelope in inbox_envelopes {
        let headers = match source
            .download_message(&envelope.account_id.to_string(), &envelope.id)
            .await
        {
            Ok(eml) => parse_headers(&eml),
            Err(error) => {
                tracing::warn!(
                    message_id = %envelope.message_id,
                    error = %error,
                    "could not download a message; its technical headers are skipped"
                );
                ScanHeaders::default()
            }
        };
        inbox.push(ScannedMessage { envelope, headers });
    }

    Ok(analyze(&inbox, &sent))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::source::MessageSource;
    use crate::ingest::types::{DataPage, EmailSearchRequest, Envelope, MinimalAccount};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct FakeSource {
        pages: Mutex<VecDeque<DataPage<Envelope>>>,
        downloads: Mutex<Vec<String>>,
        fail_download: bool,
    }

    impl FakeSource {
        fn new(envelopes: Vec<Envelope>) -> Self {
            Self {
                pages: Mutex::new(VecDeque::from([page(envelopes)])),
                downloads: Mutex::new(Vec::new()),
                fail_download: false,
            }
        }

        fn failing_downloads(envelopes: Vec<Envelope>) -> Self {
            Self {
                fail_download: true,
                ..Self::new(envelopes)
            }
        }
    }

    impl MessageSource for FakeSource {
        async fn list_accounts(&self) -> Result<Vec<MinimalAccount>, IngestError> {
            Ok(Vec::new())
        }

        async fn search_messages(
            &self,
            _request: EmailSearchRequest,
        ) -> Result<DataPage<Envelope>, IngestError> {
            Ok(self
                .pages
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| page(Vec::new())))
        }

        async fn download_message(
            &self,
            _account_id: &str,
            envelope_id: &str,
        ) -> Result<Vec<u8>, IngestError> {
            self.downloads.lock().unwrap().push(envelope_id.to_string());
            if self.fail_download {
                Err(IngestError::Config(
                    "simulated download failure".to_string(),
                ))
            } else {
                Ok(Vec::new())
            }
        }
    }

    fn page(items: Vec<Envelope>) -> DataPage<Envelope> {
        DataPage {
            current_page: None,
            page_size: Some(200),
            total_items: items.len() as u64,
            items,
            total_pages: None,
        }
    }

    fn envelope(id: &str, mailbox: &str, from: &str) -> Envelope {
        Envelope {
            id: id.to_string(),
            message_id: format!("<{id}@x.test>"),
            account_id: 1,
            account_email: None,
            mailbox_id: 1,
            mailbox_name: Some(mailbox.to_string()),
            uid: 1,
            subject: String::new(),
            preview: String::new(),
            from: from.to_string(),
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

    #[tokio::test]
    async fn scan_produces_a_report_with_partition_counts() {
        let source = FakeSource::new(vec![
            envelope("a", "INBOX", "alice@x.test"),
            envelope("b", "INBOX", "bob@x.test"),
            envelope("c", "Sent", "me@x.test"),
        ]);
        let report = scan(&source, &[1], 0).await.unwrap();
        assert_eq!(report.inbox_messages, 2);
        assert_eq!(report.sent_messages, 1);
        assert_eq!(report.top_senders.len(), 2);
    }

    #[tokio::test]
    async fn scan_downloads_only_inbox_messages() {
        let source = FakeSource::new(vec![
            envelope("inbox-1", "INBOX", "alice@x.test"),
            envelope("sent-1", "Sent", "me@x.test"),
        ]);
        scan(&source, &[1], 0).await.unwrap();
        let downloads = source.downloads.lock().unwrap();
        assert_eq!(downloads.len(), 1);
        assert_eq!(downloads[0], "inbox-1");
    }

    #[tokio::test]
    async fn scan_tolerates_a_download_failure() {
        let source = FakeSource::failing_downloads(vec![envelope("a", "INBOX", "alice@x.test")]);
        let report = scan(&source, &[1], 0).await.unwrap();
        assert_eq!(report.inbox_messages, 1);
    }

    #[tokio::test]
    async fn scan_on_an_empty_window_is_an_empty_report() {
        let source = FakeSource::new(Vec::new());
        let report = scan(&source, &[1], 0).await.unwrap();
        assert_eq!(report.messages_analyzed, 0);
    }
}
