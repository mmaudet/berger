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

//! Wire types for the Bichon REST API.
//!
//! These mirror the schemas published in Bichon's OpenAPI document
//! (`/api-docs/spec.json`). Response types capture the full schema so
//! deserialization stays robust; request types carry only the fields
//! Berger actually sends. All timestamps are epoch milliseconds.

use serde::{Deserialize, Serialize};

/// Message metadata as returned by Bichon's search and envelope endpoints.
///
/// `message_id` is the RFC 822 Message-ID — Berger's idempotency key,
/// stable across IMAP `COPY`/`MOVE`. `id` is Bichon's own envelope
/// identifier, used in per-message URLs.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Envelope {
    /// Bichon's internal envelope id.
    pub id: String,
    /// RFC 822 Message-ID.
    pub message_id: String,
    pub account_id: u64,
    pub account_email: Option<String>,
    pub mailbox_id: u64,
    /// Full IMAP folder path the message currently lives in. Optional in
    /// Bichon's schema; absent means the folder is unknown.
    pub mailbox_name: Option<String>,
    pub uid: u32,
    pub subject: String,
    pub preview: String,
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    /// `Date:` header, epoch milliseconds. Sender-controlled.
    pub date: i64,
    /// IMAP INTERNALDATE, epoch milliseconds.
    pub internal_date: i64,
    /// When Bichon indexed the message, epoch milliseconds.
    pub ingest_at: i64,
    pub size: u32,
    pub thread_id: String,
    pub attachment_count: u64,
    pub regular_attachment_count: u64,
    pub tags: Option<Vec<String>>,
    pub content_hash: String,
}

/// A page of results from a Bichon list/search endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DataPage<T> {
    pub current_page: Option<u64>,
    pub page_size: Option<u64>,
    pub total_items: u64,
    pub items: Vec<T>,
    pub total_pages: Option<u64>,
}

/// A Bichon account — id and address only, from `/minimal-account-list`.
/// Unlike `/accounts`, this endpoint never exposes stored IMAP credentials.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MinimalAccount {
    pub id: u64,
    pub email: String,
}

/// Bichon's error envelope, returned with non-2xx responses.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ApiError {
    pub code: u32,
    pub message: String,
}

/// Sort key for `search-messages`. Bichon can only sort by these two
/// fields — notably *not* by ingest time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SortBy {
    Date,
    Size,
}

/// Filter for `search-messages`. Only the fields Berger sets are modelled;
/// unset fields are omitted from the request body (Bichon defaults them).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EmailSearchFilter {
    /// Lower bound on the message `Date:` header, epoch milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<i64>,
    /// Restrict the search to these Bichon account ids.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_ids: Option<Vec<u64>>,
}

/// Request body for `POST /api/v1/search-messages`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EmailSearchRequest {
    pub filter: EmailSearchFilter,
    /// 1-based page number.
    pub page: u64,
    /// Page size; Bichon caps this at 500.
    pub page_size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_by: Option<SortBy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desc: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_a_bichon_envelope() {
        let json = r#"{
            "id": "env-abc",
            "message_id": "<abc-def@interieur.gouv.fr>",
            "account_id": 8525922389589073,
            "account_email": "mmaudet@linagora.com",
            "mailbox_id": 42,
            "mailbox_name": "INBOX",
            "uid": 1234,
            "subject": "Validation architecture",
            "preview": "Bonjour Michel-Marie",
            "from": "Arnaud Clair <arnaud.clair@interieur.gouv.fr>",
            "to": ["Michel-Marie Maudet <mmaudet@linagora.com>"],
            "cc": [],
            "bcc": [],
            "date": 1779109081851,
            "internal_date": 1779109090000,
            "ingest_at": 1779109099999,
            "size": 20480,
            "thread_id": "thread-xyz",
            "attachment_count": 0,
            "regular_attachment_count": 0,
            "tags": ["work"],
            "content_hash": "blake3-deadbeef"
        }"#;
        let env: Envelope = serde_json::from_str(json).unwrap();
        assert_eq!(env.message_id, "<abc-def@interieur.gouv.fr>");
        assert_eq!(env.account_id, 8_525_922_389_589_073);
        assert_eq!(env.mailbox_name.as_deref(), Some("INBOX"));
        assert_eq!(env.ingest_at, 1_779_109_099_999);
        assert_eq!(env.tags.as_deref(), Some(["work".to_string()].as_slice()));
    }

    #[test]
    fn envelope_optional_fields_default_to_none_when_absent() {
        // account_email, mailbox_name and tags are not in Bichon's
        // `required` set, so a payload may omit them entirely.
        let json = r#"{
            "id": "env-1", "message_id": "<m1@example.test>", "account_id": 1,
            "mailbox_id": 2, "uid": 3, "subject": "s", "preview": "p",
            "from": "a@example.test", "to": [], "cc": [], "bcc": [],
            "date": 0, "internal_date": 0, "ingest_at": 0, "size": 0,
            "thread_id": "t", "attachment_count": 0,
            "regular_attachment_count": 0, "content_hash": "h"
        }"#;
        let env: Envelope = serde_json::from_str(json).unwrap();
        assert_eq!(env.account_email, None);
        assert_eq!(env.mailbox_name, None);
        assert_eq!(env.tags, None);
    }

    #[test]
    fn envelope_deserialization_ignores_unknown_fields() {
        // Forward compatibility: a future Bichon field must not break us.
        let json = r#"{
            "id": "x", "message_id": "<x@x>", "account_id": 1, "mailbox_id": 1,
            "uid": 1, "subject": "", "preview": "", "from": "", "to": [],
            "cc": [], "bcc": [], "date": 0, "internal_date": 0, "ingest_at": 0,
            "size": 0, "thread_id": "", "attachment_count": 0,
            "regular_attachment_count": 0, "content_hash": "",
            "some_future_field": {"nested": true}
        }"#;
        let env: Envelope = serde_json::from_str(json).unwrap();
        assert_eq!(env.id, "x");
    }

    #[test]
    fn deserializes_a_datapage_of_envelopes() {
        let json = r#"{
            "current_page": 1, "page_size": 50, "total_items": 1,
            "total_pages": 1,
            "items": [{
                "id": "e", "message_id": "<e@e>", "account_id": 1,
                "mailbox_id": 1, "uid": 1, "subject": "", "preview": "",
                "from": "", "to": [], "cc": [], "bcc": [], "date": 0,
                "internal_date": 0, "ingest_at": 0, "size": 0,
                "thread_id": "", "attachment_count": 0,
                "regular_attachment_count": 0, "content_hash": ""
            }]
        }"#;
        let page: DataPage<Envelope> = serde_json::from_str(json).unwrap();
        assert_eq!(page.total_items, 1);
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].message_id, "<e@e>");
    }

    #[test]
    fn deserializes_minimal_account() {
        let json = r#"{"id": 1417038252461348, "email": "michel.maudet@gmail.com"}"#;
        let account: MinimalAccount = serde_json::from_str(json).unwrap();
        assert_eq!(account.id, 1_417_038_252_461_348);
        assert_eq!(account.email, "michel.maudet@gmail.com");
    }

    #[test]
    fn deserializes_api_error() {
        let json = r#"{"code": 30000, "message": "not found"}"#;
        let error: ApiError = serde_json::from_str(json).unwrap();
        assert_eq!(error.code, 30000);
        assert_eq!(error.message, "not found");
    }

    #[test]
    fn serializes_search_request_with_the_fields_berger_sets() {
        let request = EmailSearchRequest {
            filter: EmailSearchFilter {
                since: Some(1_779_109_081_851),
                account_ids: Some(vec![8_525_922_389_589_073]),
            },
            page: 1,
            page_size: 200,
            sort_by: Some(SortBy::Date),
            desc: Some(false),
        };
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["page"], 1);
        assert_eq!(value["page_size"], 200);
        assert_eq!(value["sort_by"], "DATE");
        assert_eq!(value["desc"], false);
        assert_eq!(value["filter"]["since"], 1_779_109_081_851_i64);
        assert_eq!(value["filter"]["account_ids"][0], 8_525_922_389_589_073_u64);
    }

    #[test]
    fn search_filter_omits_unset_fields() {
        let request = EmailSearchRequest {
            filter: EmailSearchFilter::default(),
            page: 1,
            page_size: 50,
            sort_by: None,
            desc: None,
        };
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["filter"], serde_json::json!({}));
        assert!(value.get("sort_by").is_none());
        assert!(value.get("desc").is_none());
    }
}
