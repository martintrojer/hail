//! Reusable Gmail-provider import fixture catalog.
//!
//! The fixtures in this module intentionally stay provider-shaped but not tied to
//! `hail-worker` types. Worker tests can map them into fake Gmail sources, while
//! client/protocol tests can serialize the same cases as Gmail-style JSON. Raw
//! message bodies always come from the checked-in RFC822 corpus under
//! `tests/fixtures/mail`.

use base64::Engine as _;
use serde_json::{json, Value};

use crate::{mail_fixture, MailFixture};

/// Gmail import behavior covered by a reusable fixture case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    /// Gmail label ids attached to this provider message in Gmail-shaped JSON.
    /// These are import hints only; worker tests assert hail does not mirror them.
    pub label_ids: &'static [&'static str],
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
            "labelIds": self.label_ids,
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
        label_ids: &["INBOX"],
        rfc822_message_id: "personal-simple-20250520@personal.example",
        intended_route: None,
    },
    GmailImportFixture {
        scenario: GmailImportScenario::DedupeIdempotency,
        gmail_id: "gmail-duplicate-personal",
        thread_id: "gmail-thread-duplicate-personal",
        history_id: "gmail-history-1002",
        mail_fixture_name: "personal-simple.eml",
        label_ids: &["INBOX"],
        rfc822_message_id: "personal-simple-20250520@personal.example",
        intended_route: None,
    },
    GmailImportFixture {
        scenario: GmailImportScenario::ExpiredCursorFallback,
        gmail_id: "gmail-fallback-quoted",
        thread_id: "gmail-thread-fallback-quoted",
        history_id: "gmail-history-2500",
        mail_fixture_name: "quoted-gmail.eml",
        label_ids: &["INBOX"],
        rfc822_message_id: "quoted-gmail-20250522@gmail.example",
        intended_route: None,
    },
    GmailImportFixture {
        scenario: GmailImportScenario::RoutingScreener,
        gmail_id: "gmail-route-screener",
        thread_id: "gmail-thread-route-screener",
        history_id: "gmail-history-3001",
        mail_fixture_name: "personal-simple.eml",
        label_ids: &["INBOX"],
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
        label_ids: &["INBOX"],
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
        label_ids: &["CATEGORY_PROMOTIONS"],
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
        label_ids: &["INBOX"],
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
        label_ids: &["SENT"],
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

    const ALL_SCENARIOS: &[GmailImportScenario] = &[
        GmailImportScenario::RawRfc822Import,
        GmailImportScenario::DedupeIdempotency,
        GmailImportScenario::ExpiredCursorFallback,
        GmailImportScenario::RoutingScreener,
        GmailImportScenario::RoutingImbox,
        GmailImportScenario::RoutingFeed,
        GmailImportScenario::RoutingPapertrail,
        GmailImportScenario::SentCopyOneWay,
    ];

    const GMAIL_HISTORICAL_IMPORT_TESTS: &str =
        include_str!("../../../crates/hail-worker/tests/gmail_historical_import.rs");
    const GMAIL_INCREMENTAL_SYNC_TESTS: &str =
        include_str!("../../../crates/hail-worker/tests/gmail_incremental_sync.rs");

    #[derive(Debug, Clone, Copy)]
    struct ExpectedFixtureContract {
        scenario: GmailImportScenario,
        gmail_id: &'static str,
        thread_id: &'static str,
        history_id: &'static str,
        mail_fixture_name: &'static str,
        label_ids: &'static [&'static str],
        rfc822_message_id: &'static str,
        route: Option<GmailImportRoute>,
        exercised_by: &'static [&'static str],
    }

    const EXPECTED_CONTRACTS: &[ExpectedFixtureContract] = &[
        ExpectedFixtureContract {
            scenario: GmailImportScenario::RawRfc822Import,
            gmail_id: "gmail-raw-personal",
            thread_id: "gmail-thread-personal",
            history_id: "gmail-history-1001",
            mail_fixture_name: "personal-simple.eml",
            label_ids: &["INBOX"],
            rfc822_message_id: "personal-simple-20250520@personal.example",
            route: None,
            exercised_by: &[
                "hail-test::gmail_json_shapes_are_camel_case_and_raw_is_base64url_rfc822_for_every_fixture",
                "hail-worker::gmail_historical_import::imports_gmail_pages_into_stalwart_and_records_mapping_and_audit",
                "hail-worker::gmail_historical_import::dedupes_different_provider_id_by_rfc822_message_id",
            ],
        },
        ExpectedFixtureContract {
            scenario: GmailImportScenario::DedupeIdempotency,
            gmail_id: "gmail-duplicate-personal",
            thread_id: "gmail-thread-duplicate-personal",
            history_id: "gmail-history-1002",
            mail_fixture_name: "personal-simple.eml",
            label_ids: &["INBOX"],
            rfc822_message_id: "personal-simple-20250520@personal.example",
            route: None,
            exercised_by: &[
                "hail-worker::gmail_historical_import::dedupes_different_provider_id_by_rfc822_message_id",
            ],
        },
        ExpectedFixtureContract {
            scenario: GmailImportScenario::ExpiredCursorFallback,
            gmail_id: "gmail-fallback-quoted",
            thread_id: "gmail-thread-fallback-quoted",
            history_id: "gmail-history-2500",
            mail_fixture_name: "quoted-gmail.eml",
            label_ids: &["INBOX"],
            rfc822_message_id: "quoted-gmail-20250522@gmail.example",
            route: None,
            exercised_by: &[
                "hail-worker::gmail_incremental_sync::expired_history_cursor_runs_bounded_full_sync_and_audits_fallback",
            ],
        },
        ExpectedFixtureContract {
            scenario: GmailImportScenario::RoutingScreener,
            gmail_id: "gmail-route-screener",
            thread_id: "gmail-thread-route-screener",
            history_id: "gmail-history-3001",
            mail_fixture_name: "personal-simple.eml",
            label_ids: &["INBOX"],
            rfc822_message_id: "personal-simple-20250520@personal.example",
            route: Some(GmailImportRoute {
                sender: "maya@personal.example",
                classify_as: None,
                keyword: None,
            }),
            exercised_by: &[
                "hail-worker::gmail_historical_import::routed_import_sends_unknown_sender_to_screener_pending",
            ],
        },
        ExpectedFixtureContract {
            scenario: GmailImportScenario::RoutingImbox,
            gmail_id: "gmail-route-imbox",
            thread_id: "gmail-thread-route-imbox",
            history_id: "gmail-history-3002",
            mail_fixture_name: "attachment-small-text.eml",
            label_ids: &["INBOX"],
            rfc822_message_id: "attachment-small-text-20250522@studio.example",
            route: Some(GmailImportRoute {
                sender: "jordan@studio.example",
                classify_as: Some("imbox"),
                keyword: Some("$hail_imbox"),
            }),
            exercised_by: &[
                "hail-worker::gmail_historical_import::routed_import_applies_allowed_sender_classifications",
            ],
        },
        ExpectedFixtureContract {
            scenario: GmailImportScenario::RoutingFeed,
            gmail_id: "gmail-route-feed",
            thread_id: "gmail-thread-route-feed",
            history_id: "gmail-history-3003",
            mail_fixture_name: "newsletter-tracking-pixel.eml",
            label_ids: &["CATEGORY_PROMOTIONS"],
            rfc822_message_id: "newsletter-20250521@northwind.example",
            route: Some(GmailImportRoute {
                sender: "news@northwind.example",
                classify_as: Some("feed"),
                keyword: Some("$hail_feed"),
            }),
            exercised_by: &[
                "hail-worker::gmail_historical_import::imports_gmail_pages_into_stalwart_and_records_mapping_and_audit",
                "hail-worker::gmail_historical_import::routed_import_applies_allowed_sender_classifications",
            ],
        },
        ExpectedFixtureContract {
            scenario: GmailImportScenario::RoutingPapertrail,
            gmail_id: "gmail-route-papertrail",
            thread_id: "gmail-thread-route-papertrail",
            history_id: "gmail-history-3004",
            mail_fixture_name: "receipt-papertrail.eml",
            label_ids: &["INBOX"],
            rfc822_message_id: "receipt-pt-1042@papertrail-books.example",
            route: Some(GmailImportRoute {
                sender: "receipts@papertrail-books.example",
                classify_as: Some("papertrail"),
                keyword: Some("$hail_papertrail"),
            }),
            exercised_by: &[
                "hail-worker::gmail_historical_import::routed_import_applies_allowed_sender_classifications",
            ],
        },
        ExpectedFixtureContract {
            scenario: GmailImportScenario::SentCopyOneWay,
            gmail_id: "gmail-sent-copy-one-way",
            thread_id: "gmail-thread-sent-copy-one-way",
            history_id: "gmail-history-4001",
            mail_fixture_name: "personal-thread-reply.eml",
            label_ids: &["SENT"],
            rfc822_message_id: "personal-thread-reply-20250520@hail.test",
            route: None,
            exercised_by: &[
                "hail-worker::gmail_historical_import::explicit_sent_copy_import_can_disable_default_sent_exclusion",
            ],
        },
    ];

    #[test]
    fn gmail_import_catalog_matches_pinned_contracts() {
        let mut scenarios = HashSet::new();
        for fixture in GMAIL_IMPORT_FIXTURES {
            assert!(
                scenarios.insert(fixture.scenario),
                "duplicate scenario {:?}",
                fixture.scenario
            );
        }

        assert_eq!(GMAIL_IMPORT_FIXTURES.len(), EXPECTED_CONTRACTS.len());
        for expected in EXPECTED_CONTRACTS {
            let fixture = gmail_import_fixture(expected.scenario);
            assert_eq!(
                fixture.gmail_id, expected.gmail_id,
                "{:?} gmail id drifted",
                expected.scenario
            );
            assert_eq!(
                fixture.thread_id, expected.thread_id,
                "{:?} thread id drifted",
                expected.scenario
            );
            assert_eq!(
                fixture.history_id, expected.history_id,
                "{:?} history id drifted",
                expected.scenario
            );
            assert_eq!(
                fixture.mail_fixture_name, expected.mail_fixture_name,
                "{:?} RFC822 backing fixture drifted",
                expected.scenario
            );
            assert_eq!(
                fixture.label_ids, expected.label_ids,
                "{:?} labels drifted",
                expected.scenario
            );
            assert_eq!(
                fixture.rfc822_message_id, expected.rfc822_message_id,
                "{:?} Message-ID contract drifted",
                expected.scenario
            );
            assert_eq!(
                fixture.intended_route, expected.route,
                "{:?} route contract drifted",
                expected.scenario
            );
            assert!(
                !expected.exercised_by.is_empty(),
                "{:?} must be exercised or explicitly marked by contract",
                expected.scenario
            );
        }
    }

    #[test]
    fn gmail_import_catalog_references_valid_rfc822_fixtures() {
        let mut gmail_ids = HashSet::new();
        let mut thread_ids = HashSet::new();
        let mut history_ids = HashSet::new();
        for fixture in GMAIL_IMPORT_FIXTURES {
            assert!(
                gmail_ids.insert(fixture.gmail_id),
                "duplicate gmail id {}",
                fixture.gmail_id
            );
            assert!(
                thread_ids.insert(fixture.thread_id),
                "duplicate thread id {}",
                fixture.thread_id
            );
            assert!(
                history_ids.insert(fixture.history_id),
                "duplicate history id {}",
                fixture.history_id
            );
            assert!(
                fixture.gmail_id.starts_with("gmail-"),
                "{} should be a stable synthetic Gmail id",
                fixture.gmail_id
            );
            assert!(
                fixture.thread_id.starts_with("gmail-thread-"),
                "{} should be a stable synthetic Gmail thread id",
                fixture.thread_id
            );
            assert!(
                fixture.history_id.starts_with("gmail-history-"),
                "{} should be a stable synthetic Gmail history id",
                fixture.history_id
            );
            assert!(
                !fixture.label_ids.is_empty(),
                "{} should pin Gmail label hints",
                fixture.gmail_id
            );
            let mail = fixture.mail_fixture();
            let headers = mail.headers().expect("fixture headers parse");
            assert_eq!(
                headers.get("message-id"),
                Some(format!("<{}>", fixture.rfc822_message_id).as_str())
            );
            assert!(
                headers.get("from").is_some(),
                "{} should have From",
                fixture.mail_fixture_name
            );
            assert!(
                headers.get("date").is_some(),
                "{} should have Date",
                fixture.mail_fixture_name
            );
            assert!(
                headers.get("subject").is_some(),
                "{} should have Subject",
                fixture.mail_fixture_name
            );
            assert!(
                fixture
                    .raw_rfc822()
                    .windows(4)
                    .any(|window| window == b"\r\n\r\n")
                    || fixture
                        .raw_rfc822()
                        .windows(2)
                        .any(|window| window == b"\n\n"),
                "{} should contain RFC822 header/body separator",
                fixture.mail_fixture_name
            );
            if let Some(route) = fixture.intended_route {
                assert!(
                    headers
                        .get("from")
                        .is_some_and(|from| from.to_ascii_lowercase().contains(route.sender)),
                    "{:?} route sender {} should match From {:?}",
                    fixture.scenario,
                    route.sender,
                    headers.get("from")
                );
            }
        }
    }

    #[test]
    fn gmail_json_shapes_are_camel_case_and_raw_is_base64url_rfc822_for_every_fixture() {
        let list = gmail_list_messages_json(GMAIL_IMPORT_FIXTURES, Some("page-2"));
        assert_eq!(list["nextPageToken"], "page-2");
        assert_eq!(list["resultSizeEstimate"], GMAIL_IMPORT_FIXTURES.len());

        let history = gmail_history_json(GMAIL_IMPORT_FIXTURES, "100", "101");
        assert_eq!(history["historyId"], "101");

        for (idx, fixture) in GMAIL_IMPORT_FIXTURES.iter().copied().enumerate() {
            assert_eq!(list["messages"][idx]["id"], fixture.gmail_id);
            assert_eq!(list["messages"][idx]["threadId"], fixture.thread_id);
            assert_eq!(
                history["history"][idx]["messagesAdded"][0]["message"],
                fixture.list_message_json()
            );

            let raw = fixture.raw_message_json();
            assert_eq!(raw["id"], fixture.gmail_id);
            assert_eq!(raw["threadId"], fixture.thread_id);
            assert_eq!(raw["historyId"], fixture.history_id);
            assert_eq!(raw["labelIds"], serde_json::json!(fixture.label_ids));
            let encoded = raw["raw"].as_str().expect("raw string");
            assert!(
                !encoded.contains(['+', '/', '=']),
                "{} raw should use Gmail base64url without padding",
                fixture.gmail_id
            );
            let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(encoded)
                .unwrap_or_else(|err| panic!("{} raw base64url decodes: {err}", fixture.gmail_id));
            assert_eq!(
                decoded,
                fixture.raw_rfc822(),
                "{} raw decodes to checked-in RFC822",
                fixture.gmail_id
            );
            let parsed = crate::parse_headers(&decoded).unwrap_or_else(|err| {
                panic!("{} decoded RFC822 headers parse: {err}", fixture.gmail_id)
            });
            assert_eq!(
                parsed.get("message-id"),
                Some(format!("<{}>", fixture.rfc822_message_id).as_str()),
                "{} decoded RFC822 Message-ID matches catalog",
                fixture.gmail_id
            );
        }
    }

    fn exercised_marker_exists(marker: &str) -> bool {
        let Some((module, test_name)) = marker.rsplit_once("::") else {
            return false;
        };
        let needle = format!("fn {test_name}");
        match module {
            "hail-test" => test_name
                == "gmail_json_shapes_are_camel_case_and_raw_is_base64url_rfc822_for_every_fixture",
            "hail-worker::gmail_historical_import" => {
                GMAIL_HISTORICAL_IMPORT_TESTS.contains(&needle)
            }
            "hail-worker::gmail_incremental_sync" => GMAIL_INCREMENTAL_SYNC_TESTS.contains(&needle),
            _ => false,
        }
    }

    #[test]
    fn every_declared_scenario_is_present_and_exercised_or_marked() {
        for scenario in ALL_SCENARIOS {
            let expected = EXPECTED_CONTRACTS
                .iter()
                .find(|contract| contract.scenario == *scenario)
                .unwrap_or_else(|| panic!("{scenario:?} missing expected contract"));
            let fixture = gmail_import_fixture(*scenario);
            assert_eq!(fixture.scenario, *scenario);
            assert!(
                !expected.exercised_by.is_empty(),
                "{scenario:?} has no tests listed; add worker coverage or an explicit non-empty marker"
            );
            for marker in expected.exercised_by {
                assert!(
                    exercised_marker_exists(marker),
                    "{scenario:?} exercise marker {marker} should name an existing fixture test"
                );
            }
        }
    }
}
