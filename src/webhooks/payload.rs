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

//! The canonical webhook payload (PRD §5.6).
//!
//! Every webhook receives the same JSON schema by default; the consumer
//! routes on the `tags`. The field order of these structs is the field
//! order of the emitted JSON — kept identical to the PRD §5.6 example so
//! the payload is byte-for-byte recognisable.

use mail_parser::MessageParser;
use serde::Serialize;

use crate::ingest::types::Envelope;
use crate::llm::classifier::Classification;

/// The fixed `event` value of every Berger webhook (PRD §5.6).
const EVENT: &str = "berger.tag_applied";

/// The canonical webhook payload — the top-level object (PRD §5.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WebhookPayload {
    /// Always `berger.tag_applied`.
    pub event: &'static str,
    /// The Berger version that emitted the event.
    pub berger_version: &'static str,
    /// When the event was emitted, RFC 3339 UTC.
    pub timestamp: String,
    /// The account the message belongs to.
    pub account: String,
    /// The tags applied to the message.
    pub tags: Vec<String>,
    /// Human-readable identifiers of the filters that fired.
    pub filters_matched: Vec<String>,
    /// The message itself.
    pub message: MessagePayload,
    /// The LLM classification, or `null` when no LLM ran (PRD §5.3).
    pub classification: Option<ClassificationPayload>,
    /// A URI pointing back at the message's copy in Bichon.
    pub bichon_message_uri: String,
}

/// The `message` block of the canonical payload (PRD §5.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MessagePayload {
    /// RFC 822 Message-ID.
    pub id: String,
    /// The conversation thread identifier.
    pub thread_id: String,
    /// The sender.
    pub from: Address,
    /// The `To:` recipients.
    pub to: Vec<Address>,
    /// The `Cc:` recipients.
    pub cc: Vec<Address>,
    /// The `Subject:` header.
    pub subject: String,
    /// The `Date:` header, RFC 3339 UTC.
    pub date: String,
    /// The plain-text body.
    pub body_text: String,
    /// The HTML body.
    pub body_html: String,
    /// Whether the message carries attachments.
    pub has_attachments: bool,
}

/// One mail address — display name plus address (PRD §5.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Address {
    /// The display name; empty when the header carried only an address.
    pub name: String,
    /// The email address.
    pub email: String,
}

/// The `classification` block of the canonical payload (PRD §5.6).
///
/// The PRD example also shows `language` and `sentiment`; those are listed
/// as extensible (PRD §5.3) and Berger's typed [`Classification`] does not
/// produce them at the MVP, so they are not emitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClassificationPayload {
    /// The category label.
    pub category: String,
    /// Whether the message expects a personal reply.
    pub needs_reply: bool,
    /// Urgency, 1–5.
    pub priority: u8,
}

impl From<&Classification> for ClassificationPayload {
    fn from(classification: &Classification) -> Self {
        Self {
            category: classification.category.clone(),
            needs_reply: classification.needs_reply,
            priority: classification.priority,
        }
    }
}

/// Everything the emitter needs to build a [`WebhookPayload`].
pub struct PayloadContext<'a> {
    /// The Bichon envelope of the message.
    pub envelope: &'a Envelope,
    /// The raw RFC 822 bytes, for body and attachment extraction.
    pub eml: &'a [u8],
    /// The tags applied to the message.
    pub tags: &'a [String],
    /// Human-readable identifiers of the filters that fired.
    pub filters_matched: &'a [String],
    /// The LLM classification, when one was produced.
    pub classification: Option<&'a Classification>,
    /// The Bichon base URL, used to build `bichon_message_uri`.
    pub bichon_base_url: &'a str,
}

impl WebhookPayload {
    /// Builds the canonical payload (PRD §5.6) from a [`PayloadContext`].
    ///
    /// `timestamp` is the emission instant (epoch milliseconds), passed in
    /// so it can be pinned in tests; `message.date` comes from the
    /// envelope's `Date:` header.
    pub fn build(context: &PayloadContext<'_>, timestamp_ms: i64) -> Self {
        let envelope = context.envelope;
        let (body_text, body_html, has_attachments) = extract_bodies(context.eml);
        Self {
            event: EVENT,
            berger_version: env!("CARGO_PKG_VERSION"),
            timestamp: epoch_ms_to_rfc3339(timestamp_ms),
            account: envelope.account_email.clone().unwrap_or_default(),
            tags: context.tags.to_vec(),
            filters_matched: context.filters_matched.to_vec(),
            message: MessagePayload {
                id: envelope.message_id.clone(),
                thread_id: envelope.thread_id.clone(),
                from: parse_address(&envelope.from),
                to: envelope.to.iter().map(|a| parse_address(a)).collect(),
                cc: envelope.cc.iter().map(|a| parse_address(a)).collect(),
                subject: envelope.subject.clone(),
                date: epoch_ms_to_rfc3339(envelope.date),
                body_text,
                body_html,
                has_attachments,
            },
            classification: context.classification.map(ClassificationPayload::from),
            bichon_message_uri: bichon_message_uri(context.bichon_base_url, &envelope.id),
        }
    }
}

/// Extracts `(body_text, body_html, has_attachments)` from raw RFC 822
/// bytes. An unparseable message yields empty bodies and no attachments.
fn extract_bodies(eml: &[u8]) -> (String, String, bool) {
    let Some(message) = MessageParser::default().parse(eml) else {
        return (String::new(), String::new(), false);
    };
    let body_text = message
        .body_text(0)
        .map(|text| text.into_owned())
        .unwrap_or_default();
    let body_html = message
        .body_html(0)
        .map(|html| html.into_owned())
        .unwrap_or_default();
    let has_attachments = message.attachment_count() > 0;
    (body_text, body_html, has_attachments)
}

/// Parses a `From:`/`To:`/`Cc:` display string into an [`Address`].
///
/// `"Arnaud Clair <a@x.test>"` splits into name and address; a bare
/// `"a@x.test"` yields an empty name. Surrounding quotes and angle
/// brackets are stripped.
fn parse_address(raw: &str) -> Address {
    let raw = raw.trim();
    if let Some(open) = raw.rfind('<')
        && let Some(close) = raw[open..].find('>')
    {
        let email = raw[open + 1..open + close].trim().to_string();
        let name = raw[..open].trim().trim_matches('"').trim().to_string();
        return Address { name, email };
    }
    Address {
        name: String::new(),
        email: raw.trim_matches('"').trim().to_string(),
    }
}

/// Builds the `bichon_message_uri` from the Bichon base URL and a Bichon
/// envelope id, normalising any trailing slash on the base.
fn bichon_message_uri(base_url: &str, envelope_id: &str) -> String {
    format!(
        "{}/api/v1/messages/{}",
        base_url.trim_end_matches('/'),
        envelope_id
    )
}

/// Formats epoch milliseconds as an RFC 3339 UTC timestamp,
/// `YYYY-MM-DDTHH:MM:SSZ` — second precision, the form the PRD §5.6
/// example uses. Implemented here to avoid a date-crate dependency.
fn epoch_ms_to_rfc3339(epoch_ms: i64) -> String {
    let total_secs = epoch_ms.div_euclid(1000);
    let days = total_secs.div_euclid(86_400);
    let secs_of_day = total_secs.rem_euclid(86_400);
    let (hour, minute, second) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Converts a count of days since the Unix epoch (1970-01-01) into a civil
/// `(year, month, day)`. Howard Hinnant's `civil_from_days` algorithm.
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    // Shift the epoch to 0000-03-01 so leap days fall at the end of the era.
    let z = days_since_epoch + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11], March-based
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_envelope() -> Envelope {
        Envelope {
            id: "abc-def".to_string(),
            message_id: "<abc-def@interieur.gouv.fr>".to_string(),
            account_id: 1,
            account_email: Some("michel-marie@linagora.com".to_string()),
            mailbox_id: 1,
            mailbox_name: Some("INBOX".to_string()),
            uid: 1234,
            subject: "Validation architecture Zero Trust RAG".to_string(),
            preview: String::new(),
            from: "Arnaud Clair <arnaud.clair@interieur.gouv.fr>".to_string(),
            to: vec!["Michel-Marie Maudet <michel-marie@linagora.com>".to_string()],
            cc: Vec::new(),
            bcc: Vec::new(),
            date: 1_779_179_680_000, // 2026-05-19T08:34:40Z
            internal_date: 1_779_179_680_000,
            ingest_at: 1_779_179_680_000,
            size: 0,
            thread_id: "thread-xyz".to_string(),
            attachment_count: 0,
            regular_attachment_count: 0,
            tags: None,
            content_hash: String::new(),
        }
    }

    const EML: &[u8] = b"From: Arnaud Clair <arnaud.clair@interieur.gouv.fr>\r\nSubject: Validation architecture\r\nContent-Type: text/plain\r\n\r\nBonjour Michel-Marie, voici le document.\r\n";

    fn context<'a>(
        envelope: &'a Envelope,
        tags: &'a [String],
        filters: &'a [String],
        classification: Option<&'a Classification>,
    ) -> PayloadContext<'a> {
        PayloadContext {
            envelope,
            eml: EML,
            tags,
            filters_matched: filters,
            classification,
            bichon_base_url: "https://bichon.linagora.io",
        }
    }

    #[test]
    fn epoch_ms_to_rfc3339_renders_a_known_instant() {
        // 1779179680000 ms → 2026-05-19T08:34:40Z (the PRD §5.6 sample date).
        assert_eq!(
            epoch_ms_to_rfc3339(1_779_179_680_000),
            "2026-05-19T08:34:40Z"
        );
    }

    #[test]
    fn epoch_ms_to_rfc3339_handles_the_unix_epoch() {
        assert_eq!(epoch_ms_to_rfc3339(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn epoch_ms_to_rfc3339_handles_a_leap_day() {
        // 2024-02-29T12:00:00Z.
        assert_eq!(
            epoch_ms_to_rfc3339(1_709_208_000_000),
            "2024-02-29T12:00:00Z"
        );
    }

    #[test]
    fn parse_address_splits_name_and_email() {
        let address = parse_address("Arnaud Clair <arnaud.clair@interieur.gouv.fr>");
        assert_eq!(address.name, "Arnaud Clair");
        assert_eq!(address.email, "arnaud.clair@interieur.gouv.fr");
    }

    #[test]
    fn parse_address_handles_a_bare_address() {
        let address = parse_address("noreply@example.test");
        assert_eq!(address.name, "");
        assert_eq!(address.email, "noreply@example.test");
    }

    #[test]
    fn parse_address_strips_quotes_around_the_name() {
        let address = parse_address("\"Clair, Arnaud\" <a@x.test>");
        assert_eq!(address.name, "Clair, Arnaud");
        assert_eq!(address.email, "a@x.test");
    }

    #[test]
    fn bichon_message_uri_normalises_a_trailing_slash() {
        assert_eq!(
            bichon_message_uri("https://bichon.test/", "env-9"),
            "https://bichon.test/api/v1/messages/env-9"
        );
        assert_eq!(
            bichon_message_uri("https://bichon.test", "env-9"),
            "https://bichon.test/api/v1/messages/env-9"
        );
    }

    #[test]
    fn build_produces_the_canonical_top_level_shape() {
        let envelope = test_envelope();
        let tags = ["a-repondre/pro".to_string(), "cat/urgent".to_string()];
        let filters = ["sender_in:gouv-interieur".to_string()];
        let payload = WebhookPayload::build(
            &context(&envelope, &tags, &filters, None),
            1_779_179_680_000,
        );
        assert_eq!(payload.event, "berger.tag_applied");
        assert_eq!(payload.berger_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(payload.timestamp, "2026-05-19T08:34:40Z");
        assert_eq!(payload.account, "michel-marie@linagora.com");
        assert_eq!(payload.tags, tags);
        assert_eq!(payload.filters_matched, filters);
        assert_eq!(
            payload.bichon_message_uri,
            "https://bichon.linagora.io/api/v1/messages/abc-def"
        );
    }

    #[test]
    fn build_fills_the_message_block_from_the_envelope_and_eml() {
        let envelope = test_envelope();
        let payload = WebhookPayload::build(&context(&envelope, &[], &[], None), 0);
        let message = &payload.message;
        assert_eq!(message.id, "<abc-def@interieur.gouv.fr>");
        assert_eq!(message.thread_id, "thread-xyz");
        assert_eq!(message.from.name, "Arnaud Clair");
        assert_eq!(message.from.email, "arnaud.clair@interieur.gouv.fr");
        assert_eq!(message.to.len(), 1);
        assert_eq!(message.to[0].email, "michel-marie@linagora.com");
        assert!(message.cc.is_empty());
        assert_eq!(message.subject, "Validation architecture Zero Trust RAG");
        assert_eq!(message.date, "2026-05-19T08:34:40Z");
        assert!(message.body_text.contains("Bonjour Michel-Marie"));
        assert!(!message.has_attachments);
    }

    #[test]
    fn build_omits_the_classification_when_there_is_none() {
        let envelope = test_envelope();
        let payload = WebhookPayload::build(&context(&envelope, &[], &[], None), 0);
        assert!(payload.classification.is_none());
        // A `None` classification serialises to JSON `null`.
        let json = serde_json::to_value(&payload).unwrap();
        assert!(json["classification"].is_null());
    }

    #[test]
    fn build_includes_the_classification_when_present() {
        let envelope = test_envelope();
        let classification = Classification {
            category: "work".to_string(),
            needs_reply: true,
            priority: 5,
        };
        let payload =
            WebhookPayload::build(&context(&envelope, &[], &[], Some(&classification)), 0);
        let block = payload.classification.unwrap();
        assert_eq!(block.category, "work");
        assert!(block.needs_reply);
        assert_eq!(block.priority, 5);
    }

    #[test]
    fn the_serialised_json_keeps_the_prd_field_order() {
        // serde_json preserves struct field declaration order; the order
        // must mirror the PRD §5.6 example.
        let envelope = test_envelope();
        let payload = WebhookPayload::build(&context(&envelope, &[], &[], None), 0);
        let json = serde_json::to_string(&payload).unwrap();
        let event_at = json.find("\"event\"").unwrap();
        let account_at = json.find("\"account\"").unwrap();
        let message_at = json.find("\"message\"").unwrap();
        let uri_at = json.find("\"bichon_message_uri\"").unwrap();
        assert!(event_at < account_at);
        assert!(account_at < message_at);
        assert!(message_at < uri_at);
    }

    #[test]
    fn an_unparseable_eml_yields_empty_bodies() {
        let (text, html, attachments) =
            extract_bodies(b"this is not a valid email at all \xff\xfe");
        assert_eq!(text, "");
        assert_eq!(html, "");
        assert!(!attachments);
    }

    #[test]
    fn extract_bodies_finds_the_html_part() {
        let eml =
            b"From: a@x.test\r\nSubject: s\r\nContent-Type: text/html\r\n\r\n<p>Hello</p>\r\n";
        let (_text, html, _attachments) = extract_bodies(eml);
        assert!(html.contains("<p>Hello</p>"), "got: {html:?}");
    }
}
