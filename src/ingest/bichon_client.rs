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

//! Async REST client for the Bichon archiver.

use crate::ingest::error::IngestError;
use crate::ingest::source::MessageSource;
use crate::ingest::types::{ApiError, DataPage, EmailSearchRequest, Envelope, MinimalAccount};

/// Async client for one Bichon instance.
///
/// Every request carries the API token as an HTTP Bearer credential. The
/// token lives only inside the inner `reqwest::Client` (as a header marked
/// sensitive) and never appears in this type's `Debug` output.
pub struct BichonClient {
    http: reqwest::Client,
    /// Bichon base URL, without a trailing slash.
    base_url: String,
}

impl std::fmt::Debug for BichonClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BichonClient")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl BichonClient {
    /// Builds a client for the Bichon instance at `base_url`, authenticating
    /// every request with `token` as an HTTP Bearer credential.
    ///
    /// # Errors
    /// Returns [`IngestError::Config`] when `token` cannot be encoded as an
    /// HTTP header value, or [`IngestError::Transport`] when the underlying
    /// HTTP client cannot be built.
    pub fn new(base_url: impl Into<String>, token: &str) -> Result<Self, IngestError> {
        let base_url = base_url.into().trim_end_matches('/').to_string();

        let mut authorization = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| {
                IngestError::Config(
                    "the API token contains characters not valid in an HTTP header".to_string(),
                )
            })?;
        authorization.set_sensitive(true);

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::AUTHORIZATION, authorization);

        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .default_headers(headers)
            .build()?;

        Ok(Self { http, base_url })
    }

    /// Reads a Bichon response: decodes `T` on success, otherwise maps the
    /// non-2xx response onto the matching [`IngestError`] variant.
    async fn read_json<T>(&self, response: reqwest::Response) -> Result<T, IngestError>
    where
        T: serde::de::DeserializeOwned,
    {
        let status = response.status();
        let body = response.text().await?;
        if status.is_success() {
            Ok(serde_json::from_str(&body)?)
        } else {
            Err(map_error_body(status, body))
        }
    }
}

/// Maps a non-success Bichon response onto the matching [`IngestError`]:
/// a structured `ApiError` body becomes [`IngestError::Api`], anything
/// else [`IngestError::Unexpected`].
fn map_error_body(status: reqwest::StatusCode, body: String) -> IngestError {
    if let Ok(api_error) = serde_json::from_str::<ApiError>(&body) {
        IngestError::Api {
            status,
            code: api_error.code,
            message: api_error.message,
        }
    } else {
        IngestError::Unexpected { status, body }
    }
}

impl MessageSource for BichonClient {
    async fn list_accounts(&self) -> Result<Vec<MinimalAccount>, IngestError> {
        let url = format!("{}/api/v1/minimal-account-list", self.base_url);
        let response = self.http.get(url).send().await?;
        self.read_json(response).await
    }

    async fn search_messages(
        &self,
        request: EmailSearchRequest,
    ) -> Result<DataPage<Envelope>, IngestError> {
        let url = format!("{}/api/v1/search-messages", self.base_url);
        let response = self.http.post(url).json(&request).send().await?;
        self.read_json(response).await
    }

    async fn download_message(
        &self,
        account_id: &str,
        envelope_id: &str,
    ) -> Result<Vec<u8>, IngestError> {
        let url = format!(
            "{}/api/v1/download-message/{account_id}/{envelope_id}",
            self.base_url
        );
        let response = self.http.get(url).send().await?;
        let status = response.status();
        if status.is_success() {
            Ok(response.bytes().await?.to_vec())
        } else {
            Err(map_error_body(status, response.text().await?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::types::{EmailSearchFilter, SortBy};
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sample_envelope_json() -> serde_json::Value {
        json!({
            "id": "e1", "message_id": "<m1@example.test>", "account_id": 42,
            "mailbox_id": 1, "uid": 7, "subject": "Hello", "preview": "Hi",
            "from": "s@example.test", "to": [], "cc": [], "bcc": [],
            "date": 1, "internal_date": 2, "ingest_at": 3, "size": 10,
            "thread_id": "t1", "attachment_count": 0,
            "regular_attachment_count": 0, "content_hash": "h1"
        })
    }

    #[test]
    fn new_rejects_a_token_with_invalid_header_characters() {
        // A newline cannot appear in an HTTP header value.
        let result = BichonClient::new("https://bichon.example", "bad\ntoken");
        assert!(matches!(result, Err(IngestError::Config(_))));
    }

    #[test]
    fn debug_output_does_not_leak_the_token() {
        let client = BichonClient::new("https://bichon.example", "super-secret-token").unwrap();
        let rendered = format!("{client:?}");
        assert!(!rendered.contains("super-secret-token"));
        assert!(rendered.contains("bichon.example"));
    }

    #[tokio::test]
    async fn list_accounts_returns_the_minimal_account_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/minimal-account-list"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"id": 1, "email": "a@example.test"},
                {"id": 2, "email": "b@example.test"}
            ])))
            .mount(&server)
            .await;

        let client = BichonClient::new(server.uri(), "token").unwrap();
        let accounts = client.list_accounts().await.unwrap();

        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[1].email, "b@example.test");
    }

    #[tokio::test]
    async fn list_accounts_sends_the_bearer_token() {
        let server = MockServer::start().await;
        // The mock only matches when the Bearer header is present, so an
        // unauthenticated request would fall through to a 404.
        Mock::given(method("GET"))
            .and(path("/api/v1/minimal-account-list"))
            .and(header("authorization", "Bearer s3cr3t"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = BichonClient::new(server.uri(), "s3cr3t").unwrap();
        let accounts = client.list_accounts().await.unwrap();

        assert!(accounts.is_empty());
    }

    #[tokio::test]
    async fn search_messages_posts_the_filter_and_parses_the_page() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/search-messages"))
            .and(body_json(json!({
                "filter": {"since": 1_700_000_000_000_i64, "account_ids": [42]},
                "page": 1, "page_size": 200, "sort_by": "DATE", "desc": false
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "current_page": 1, "page_size": 200, "total_items": 1,
                "total_pages": 1, "items": [sample_envelope_json()]
            })))
            .mount(&server)
            .await;

        let request = EmailSearchRequest {
            filter: EmailSearchFilter {
                since: Some(1_700_000_000_000),
                account_ids: Some(vec![42]),
            },
            page: 1,
            page_size: 200,
            sort_by: Some(SortBy::Date),
            desc: Some(false),
        };
        let client = BichonClient::new(server.uri(), "token").unwrap();
        let page = client.search_messages(request).await.unwrap();

        assert_eq!(page.total_items, 1);
        assert_eq!(page.items[0].message_id, "<m1@example.test>");
    }

    #[tokio::test]
    async fn api_error_responses_become_ingest_error_api() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/minimal-account-list"))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_json(json!({"code": 30000, "message": "not found"})),
            )
            .mount(&server)
            .await;

        let client = BichonClient::new(server.uri(), "token").unwrap();
        let error = client.list_accounts().await.unwrap_err();

        match error {
            IngestError::Api {
                status,
                code,
                message,
            } => {
                assert_eq!(status.as_u16(), 404);
                assert_eq!(code, 30000);
                assert_eq!(message, "not found");
            }
            other => panic!("expected IngestError::Api, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn non_json_error_bodies_become_ingest_error_unexpected() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/minimal-account-list"))
            .respond_with(ResponseTemplate::new(502).set_body_string("<html>bad gateway</html>"))
            .mount(&server)
            .await;

        let client = BichonClient::new(server.uri(), "token").unwrap();
        let error = client.list_accounts().await.unwrap_err();

        match error {
            IngestError::Unexpected { status, body } => {
                assert_eq!(status.as_u16(), 502);
                assert!(body.contains("bad gateway"));
            }
            other => panic!("expected IngestError::Unexpected, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn malformed_success_body_becomes_a_decode_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/minimal-account-list"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json at all"))
            .mount(&server)
            .await;

        let client = BichonClient::new(server.uri(), "token").unwrap();
        let error = client.list_accounts().await.unwrap_err();

        assert!(matches!(error, IngestError::Decode(_)));
    }

    #[tokio::test]
    async fn download_message_returns_the_raw_eml() {
        let server = MockServer::start().await;
        let eml: &[u8] = b"From: s@example.test\r\nSubject: Hi\r\n\r\nBody.\r\n";
        Mock::given(method("GET"))
            .and(path("/api/v1/download-message/42/e1"))
            .and(header("authorization", "Bearer tok"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(eml.to_vec()))
            .mount(&server)
            .await;

        let client = BichonClient::new(server.uri(), "tok").unwrap();
        let bytes = client.download_message("42", "e1").await.unwrap();

        assert_eq!(bytes, eml);
    }

    #[tokio::test]
    async fn download_message_reports_an_error_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/download-message/42/missing"))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_json(json!({"code": 30000, "message": "not found"})),
            )
            .mount(&server)
            .await;

        let client = BichonClient::new(server.uri(), "tok").unwrap();
        let error = client.download_message("42", "missing").await.unwrap_err();

        assert!(matches!(error, IngestError::Api { .. }));
    }
}
