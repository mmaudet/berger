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

//! Notification-service detection: dimension 6 of the scan (PRD v1.1 §4.2).
//!
//! Machine-to-human notification mail — password resets, receipts, alerts,
//! CI results — is recognised by its automation markers rather than by a
//! mailing-list footer. A message counts as a notification when it carries
//! an `Auto-Submitted` header other than `no`, a `Precedence` of `bulk` /
//! `list` / `junk`, or a `no-reply` / `do-not-reply` From address. The
//! detected messages are grouped by sender domain so the report can
//! suggest one rule per notifying service.

use std::collections::HashMap;

use crate::scan::address::{domain_of, extract_address};
use crate::scan::analyzer::ScannedMessage;

/// How many of the busiest notification domains to report. The PRD sets no
/// cap; 50 keeps the report bounded on a large inbox.
const TOP_NOTIFICATION_SERVICES: usize = 50;

/// One notification service and how much notification mail the inbox
/// received from it (dimension 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationService {
    /// The lowercased sender domain.
    pub domain: String,
    /// Notification messages received from this domain.
    pub messages: usize,
}

/// Dimension 6: keeps the scanned messages that look like automated
/// notifications, groups them by sender domain, and returns the busiest
/// services, most mail first (ties broken by domain).
pub fn detect_notification_services(inbox: &[ScannedMessage]) -> Vec<NotificationService> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for message in inbox {
        if !is_notification(message) {
            continue;
        }
        if let Some(address) = extract_address(&message.envelope.from)
            && let Some(domain) = domain_of(&address)
        {
            *counts.entry(domain.to_string()).or_default() += 1;
        }
    }
    let mut services: Vec<NotificationService> = counts
        .into_iter()
        .map(|(domain, messages)| NotificationService { domain, messages })
        .collect();
    services.sort_by(|a, b| {
        b.messages
            .cmp(&a.messages)
            .then_with(|| a.domain.cmp(&b.domain))
    });
    services.truncate(TOP_NOTIFICATION_SERVICES);
    services
}

/// Whether `message` carries an automation marker: an `Auto-Submitted`
/// header other than `no`, a bulk/list/junk `Precedence`, or a no-reply
/// From address.
fn is_notification(message: &ScannedMessage) -> bool {
    auto_submitted_marks_automation(message.headers.auto_submitted.as_deref())
        || precedence_marks_automation(message.headers.precedence.as_deref())
        || is_no_reply_sender(&message.envelope.from)
}

/// Whether an `Auto-Submitted` header value marks automated mail: present
/// and not `no` (RFC 3834).
fn auto_submitted_marks_automation(value: Option<&str>) -> bool {
    match value {
        Some(value) => !value.trim().eq_ignore_ascii_case("no"),
        None => false,
    }
}

/// Whether a `Precedence` header value marks bulk automated mail.
fn precedence_marks_automation(value: Option<&str>) -> bool {
    match value {
        Some(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "bulk" | "list" | "junk"
        ),
        None => false,
    }
}

/// Whether the raw `From` header is a no-reply address: its local part —
/// everything before the last `@` — lowercased and stripped of `.`, `-`
/// and `_`, equals `noreply` or `donotreply`.
fn is_no_reply_sender(from: &str) -> bool {
    let Some(address) = extract_address(from) else {
        return false;
    };
    let Some((local, _)) = address.rsplit_once('@') else {
        return false;
    };
    let normalized: String = local
        .to_ascii_lowercase()
        .chars()
        .filter(|c| !['.', '-', '_'].contains(c))
        .collect();
    normalized == "noreply" || normalized == "donotreply"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::types::Envelope;
    use crate::scan::headers::ScanHeaders;

    fn envelope(from: &str) -> Envelope {
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

    fn scanned(envelope: &Envelope, headers: ScanHeaders) -> ScannedMessage<'_> {
        ScannedMessage { envelope, headers }
    }

    fn with_auto_submitted(value: &str) -> ScanHeaders {
        ScanHeaders {
            auto_submitted: Some(value.to_string()),
            ..ScanHeaders::default()
        }
    }

    fn with_precedence(value: &str) -> ScanHeaders {
        ScanHeaders {
            precedence: Some(value.to_string()),
            ..ScanHeaders::default()
        }
    }

    #[test]
    fn auto_submitted_header_triggers_detection() {
        let envelopes = [envelope("alerts@service.test")];
        let inbox = [scanned(
            &envelopes[0],
            with_auto_submitted("auto-generated"),
        )];
        assert_eq!(
            detect_notification_services(&inbox),
            [NotificationService {
                domain: "service.test".to_string(),
                messages: 1,
            }]
        );
    }

    #[test]
    fn auto_submitted_no_is_not_a_notification() {
        // RFC 3834: `Auto-Submitted: no` is an ordinary message.
        let envelopes = [envelope("person@service.test")];
        let inbox = [scanned(&envelopes[0], with_auto_submitted("  No  "))];
        assert!(detect_notification_services(&inbox).is_empty());
    }

    #[test]
    fn precedence_bulk_triggers_detection() {
        let envelopes = [envelope("news@service.test")];
        let inbox = [scanned(&envelopes[0], with_precedence("Bulk"))];
        assert_eq!(detect_notification_services(&inbox).len(), 1);
    }

    #[test]
    fn precedence_list_and_junk_trigger_detection() {
        let envelopes = [envelope("a@list.test"), envelope("b@junk.test")];
        let inbox = [
            scanned(&envelopes[0], with_precedence("list")),
            scanned(&envelopes[1], with_precedence("JUNK")),
        ];
        assert_eq!(detect_notification_services(&inbox).len(), 2);
    }

    #[test]
    fn ordinary_precedence_is_not_a_notification() {
        let envelopes = [envelope("person@service.test")];
        let inbox = [scanned(&envelopes[0], with_precedence("first-class"))];
        assert!(detect_notification_services(&inbox).is_empty());
    }

    #[test]
    fn no_reply_sender_triggers_detection() {
        let envelopes = [envelope("noreply@service.test")];
        let inbox = [scanned(&envelopes[0], ScanHeaders::default())];
        assert_eq!(detect_notification_services(&inbox).len(), 1);
    }

    #[test]
    fn punctuated_no_reply_local_parts_trigger_detection() {
        let envelopes = [
            envelope("no-reply@a.test"),
            envelope("no.reply@b.test"),
            envelope("Do_Not_Reply <do_not_reply@c.test>"),
        ];
        let inbox = [
            scanned(&envelopes[0], ScanHeaders::default()),
            scanned(&envelopes[1], ScanHeaders::default()),
            scanned(&envelopes[2], ScanHeaders::default()),
        ];
        assert_eq!(detect_notification_services(&inbox).len(), 3);
    }

    #[test]
    fn an_ordinary_message_is_not_a_notification() {
        // No automation header and a human From address.
        let envelopes = [envelope("Alice <alice@service.test>")];
        let inbox = [scanned(&envelopes[0], ScanHeaders::default())];
        assert!(detect_notification_services(&inbox).is_empty());
    }

    #[test]
    fn a_substring_of_no_reply_is_not_a_notification() {
        // `noreplyhandler` is not the no-reply mailbox.
        let envelopes = [envelope("noreplyhandler@service.test")];
        let inbox = [scanned(&envelopes[0], ScanHeaders::default())];
        assert!(detect_notification_services(&inbox).is_empty());
    }

    #[test]
    fn notifications_are_grouped_by_sender_domain() {
        let envelopes = [
            envelope("noreply@service.test"),
            envelope("alerts@service.test"),
            envelope("noreply@other.test"),
        ];
        let inbox = [
            scanned(&envelopes[0], ScanHeaders::default()),
            scanned(&envelopes[1], with_precedence("bulk")),
            scanned(&envelopes[2], ScanHeaders::default()),
        ];
        assert_eq!(
            detect_notification_services(&inbox),
            [
                NotificationService {
                    domain: "service.test".to_string(),
                    messages: 2,
                },
                NotificationService {
                    domain: "other.test".to_string(),
                    messages: 1,
                },
            ]
        );
    }

    #[test]
    fn services_break_count_ties_by_domain() {
        let envelopes = [
            envelope("noreply@zebra.test"),
            envelope("noreply@apple.test"),
        ];
        let inbox = [
            scanned(&envelopes[0], ScanHeaders::default()),
            scanned(&envelopes[1], ScanHeaders::default()),
        ];
        let services = detect_notification_services(&inbox);
        assert_eq!(services[0].domain, "apple.test");
        assert_eq!(services[1].domain, "zebra.test");
    }

    #[test]
    fn a_notification_with_an_unparseable_domain_is_skipped() {
        let envelopes = [envelope("no address here")];
        let inbox = [scanned(&envelopes[0], with_precedence("bulk"))];
        assert!(detect_notification_services(&inbox).is_empty());
    }

    #[test]
    fn detection_is_capped() {
        let envelopes: Vec<Envelope> = (0..TOP_NOTIFICATION_SERVICES + 10)
            .map(|n| envelope(&format!("noreply@d{n}.test")))
            .collect();
        let inbox: Vec<ScannedMessage> = envelopes
            .iter()
            .map(|envelope| scanned(envelope, ScanHeaders::default()))
            .collect();
        assert_eq!(
            detect_notification_services(&inbox).len(),
            TOP_NOTIFICATION_SERVICES
        );
    }

    #[test]
    fn detection_on_empty_input_is_empty() {
        assert!(detect_notification_services(&[]).is_empty());
    }
}
