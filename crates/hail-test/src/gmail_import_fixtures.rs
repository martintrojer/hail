//! Reusable Gmail-provider import fixture catalog.
//!
//! The fixtures in this module intentionally stay provider-shaped but not tied to
//! `hail-worker` types. Worker tests can map them into fake Gmail sources, while
//! client/protocol tests can serialize the same cases as Gmail-style JSON. Raw
//! message bodies always come from the checked-in RFC822 corpus under
//! `tests/fixtures/mail`.

use base64::Engine as _;
use serde_json::{Value, json};

use crate::{MailFixture, mail_fixture};

/// Gmail import behavior covered by a reusable fixture case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GmailImportScenario {
    /// Raw RFC822 import into Stalwart/JMAP and provider mapping persistence.
    RawRfc822Import,
    /// Same RFC822 content reached via another Gmail message id.
    DedupeIdempotency,
    /// Bounded full-history import after Gmail reports an expired history cursor.
    ExpiredCursorFallback,
    /// Unknown sender should land in Screener pending.
    RoutingScreener,
    /// Allowed personal sender should be classified as Imbox.
    RoutingImbox,
    /// Allowed newsletter sender should be classified as Feed.
    RoutingFeed,
    /// Allowed receipt sender should be classified as Paper Trail.
    RoutingPapertrail,
    /// Explicit sent-copy import remains local/one-way and does not mirror labels.
    SentCopyOneWay,
}

/// One Gmail message fixture backed by a raw RFC822 corpus file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GmailImportFixture {
    pub scenario: GmailImportScenario,
    pub gmail_id: &'static str,
    pub thread_id: &'static str,
    pub history_id: &'static str,
    pub mail_fixture_name: &'static str,
    /// The canonical bare RFC822 Message-ID value expected from the fixture.
    pub rfc822_message_id: &'static str,
    /// Optional local sender rule/classification that tests should seed.
    pub intended_route: Option<GmailImportRoute>,
}

impl GmailImportFixture {
    /// Return the checked-in raw RFC822 fixture backing this Gmail message.
    #[must_use]
    pub fn mail_fixture(self) -> MailFixture {
        mail_fixture(self.mail_fixture_name)
            .unwrap_or_else(|| panic!("missing mail fixture {}", self.mail_fixture_name))
    }

    /// Raw RFC822 bytes exactly as Gmail `format=raw` should decode them.
    #[must_use]
    pub fn raw_rfc822(self) -> &'static [u8] {
        self.mail_fixture().bytes()
    }

    /// Gmail `users.messages.list` message reference JSON.
    #[must_use]
    pub fn list_message_json(self) -> Value {
        json!({
            "id": self.gmail_id,
            "threadId": self.thread_id,
        })
    }

    /// Gmail `users.messages.get?format=raw` response JSON.
    #[must_use]
    pub fn raw_message_json(self) -> Value {
        json!({
            "id": self.gmail_id,
            "threadId": self.thread_id,
            "historyId": self.history_id,
            "raw": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.raw_rfc822()),
        })
    }
}

/// Expected local route for a Gmail import fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GmailImportRoute {
    /// Normalized sender address.
    pub sender: &'static str,
    /// Sidecar `screener_rules.classify_as` value when the sender is allowed.
    pub classify_as: Option<&'static str>,
    /// Hail-owned classification keyword expected after routing.
    pub keyword: Option<&'static str>,
}

/// Stable reusable Gmail import fixture catalog.
pub const GMAIL_IMPORT_FIXTURES: &[GmailImportFixture] = &[
    GmailImportFixture {
        scenario: GmailImportScenario::RawRfc822Import,
        gmail_id: "gmail-raw-personal",
        thread_id: "gmail-thread-personal",
        history_id: "gmail-history-1001",
        mail_fixture_name: "personal-simple.eml",
        rfc822_message_id: "personal-simple-20250520@personal.example",
        intended_route: None,
    },
    GmailImportFixture {
        scenario: GmailImportScenario::DedupeIdempotency,
        gmail_id: "gmail-duplicate-personal",
        thread_id: "gmail-thread-duplicate-personal",
        history_id: "gmail-history-1002",
        mail_fixture_name: "personal-simple.eml",
        rfc822_message_id: "personal-simple-20250520@personal.example",
        intended_route: None,
    },
    GmailImportFixture {
        scenario: GmailImportScenario::ExpiredCursorFallback,
        gmail_id: "gmail-fallback-quoted",
        thread_id: "gmail-thread-fallback-quoted",
        history_id: "gmail-history-2500",
        mail_fixture_name: "quoted-gmail.eml",
        rfc822_message_id: "quoted-gmail-20250522@gmail.example",
        intended_route: None,
    },
    GmailImportFixture {
        scenario: GmailImportScenario::RoutingScreener,
        gmail_id: "gmail-route-screener",
        thread_id: "gmail-thread-route-screener",
        history_id: "gmail-history-3001",
        mail_fixture_name: "personal-simple.eml",
        rfc822_message_id: "personal-simple-20250520@personal.example",
        intended_route: Some(GmailImportRoute {
            sender: "maya@personal.example",
            classify_as: None,
            keyword: None,
        }),
    },
    GmailImportFixture {
        scenario: GmailImportScenario::RoutingImbox,
        gmail_id: "gmail-route-imbox",
        thread_id: "gmail-thread-route-imbox",
        history_id: "gmail-history-3002",
        mail_fixture_name: "attachment-small-text.eml",
        rfc822_message_id: "attachment-small-text-20250522@studio.example",
        intended_route: Some(GmailImportRoute {
            sender: "jordan@studio.example",
            classify_as: Some("imbox"),
            keyword: Some("$hail_imbox"),
        }),
    },
    GmailImportFixture {
        scenario: GmailImportScenario::RoutingFeed,
        gmail_id: "gmail-route-feed",
        thread_id: "gmail-thread-route-feed",
        history_id: "gmail-history-3003",
        mail_fixture_name: "newsletter-tracking-pixel.eml",
        rfc822_message_id: "newsletter-20250521@northwind.example",
        intended_route: Some(GmailImportRoute {
            sender: "news@northwind.example",
            classify_as: Some("feed"),
            keyword: Some("$hail_feed"),
        }),
    },
    GmailImportFixture {
        scenario: GmailImportScenario::RoutingPapertrail,
        gmail_id: "gmail-route-papertrail",
        thread_id: "gmail-thread-route-papertrail",
        history_id: "gmail-history-3004",
        mail_fixture_name: "receipt-papertrail.eml",
        rfc822_message_id: "receipt-pt-1042@papertrail-books.example",
        intended_route: Some(GmailImportRoute {
            sender: "receipts@papertrail-books.example",
            classify_as: Some("papertrail"),
            keyword: Some("$hail_papertrail"),
        }),
    },
    GmailImportFixture {
        scenario: GmailImportScenario::SentCopyOneWay,
        gmail_id: "gmail-sent-copy-one-way",
        thread_id: "gmail-thread-sent-copy-one-way",
        history_id: "gmail-history-4001",
        mail_fixture_name: "personal-thread-reply.eml",
        rfc822_message_id: "personal-thread-reply-20250520@hail.test",
        intended_route: None,
    },
];

/// Find one Gmail import fixture by scenario.
#[must_use]
pub fn gmail_import_fixture(scenario: GmailImportScenario) -> GmailImportFixture {
    GMAIL_IMPORT_FIXTURES
        .iter()
        .copied()
        .find(|fixture| fixture.scenario == scenario)
        .unwrap_or_else(|| panic!("missing Gmail import fixture for {scenario:?}"))
}

/// Gmail `users.messages.list` response JSON for a fixture page.
#[must_use]
pub fn gmail_list_messages_json(
    fixtures: &[GmailImportFixture],
    next_page_token: Option<&str>,
) -> Value {
    json!({
        "messages": fixtures
            .iter()
            .copied()
            .map(GmailImportFixture::list_message_json)
            .collect::<Vec<_>>(),
        "nextPageToken": next_page_token,
        "resultSizeEstimate": fixtures.len(),
    })
}

/// Gmail `users.history.list` response JSON for message-added fixture records.
#[must_use]
pub fn gmail_history_json(
    fixtures: &[GmailImportFixture],
    start_history_id: &str,
    end_history_id: &str,
) -> Value {
    json!({
        "history": fixtures
            .iter()
            .enumerate()
            .map(|(idx, fixture)| json!({
                "id": format!("{start_history_id}-record-{idx}"),
                "messagesAdded": [{ "message": fixture.list_message_json() }],
            }))
            .collect::<Vec<_>>(),
        "historyId": end_history_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn gmail_import_catalog_references_valid_rfc822_fixtures() {
        let mut gmail_ids = HashSet::new();
        for fixture in GMAIL_IMPORT_FIXTURES {
            assert!(gmail_ids.insert(fixture.gmail_id), "duplicate gmail id");
            let mail = fixture.mail_fixture();
            let headers = mail.headers().expect("fixture headers parse");
            assert_eq!(
                headers.get("message-id"),
                Some(format!("<{}>", fixture.rfc822_message_id).as_str())
            );
            assert!(!fixture.raw_rfc822().is_empty());
        }
    }

    #[test]
    fn gmail_json_shapes_are_camel_case_and_raw_is_base64url() {
        let fixture = gmail_import_fixture(GmailImportScenario::RawRfc822Import);
        let list = gmail_list_messages_json(&[fixture], Some("page-2"));
        assert_eq!(list["messages"][0]["threadId"], fixture.thread_id);
        assert_eq!(list["nextPageToken"], "page-2");

        let history = gmail_history_json(&[fixture], "100", "101");
        assert_eq!(history["historyId"], "101");
        assert_eq!(
            history["history"][0]["messagesAdded"][0]["message"]["id"],
            fixture.gmail_id
        );

        let raw = fixture.raw_message_json();
        let encoded = raw["raw"].as_str().expect("raw string");
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .expect("base64url decodes");
        assert_eq!(decoded, fixture.raw_rfc822());
    }
}
