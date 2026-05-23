//! Reusable test fixtures and lightweight helpers for hail tests.
//!
//! This crate intentionally stays small: it exposes the checked-in RFC822 mail
//! corpus and a minimal header parser that is sufficient for fixture smoke
//! tests and for future local/E2E testbeds to inject raw messages.

pub mod local_mail_testbed;
pub mod stalwart;

use std::collections::BTreeMap;

/// Relative path to the synthetic RFC822 corpus from the workspace root.
pub const MAIL_FIXTURE_DIR: &str = "tests/fixtures/mail";

const PERSONAL_SIMPLE: &[u8] = include_bytes!("../../../tests/fixtures/mail/personal-simple.eml");
const PERSONAL_THREAD_REPLY: &[u8] =
    include_bytes!("../../../tests/fixtures/mail/personal-thread-reply.eml");
const NEWSLETTER_TRACKING_PIXEL: &[u8] =
    include_bytes!("../../../tests/fixtures/mail/newsletter-tracking-pixel.eml");
const RECEIPT_PAPERTRAIL: &[u8] =
    include_bytes!("../../../tests/fixtures/mail/receipt-papertrail.eml");
const ATTACHMENT_SMALL_TEXT: &[u8] =
    include_bytes!("../../../tests/fixtures/mail/attachment-small-text.eml");
const QUOTED_GMAIL: &[u8] = include_bytes!("../../../tests/fixtures/mail/quoted-gmail.eml");
const QUOTED_OUTLOOK: &[u8] = include_bytes!("../../../tests/fixtures/mail/quoted-outlook.eml");
const MALICIOUS_HTML: &[u8] = include_bytes!("../../../tests/fixtures/mail/malicious-html.eml");

/// One checked-in mail fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MailFixture {
    /// File name under [`MAIL_FIXTURE_DIR`].
    pub name: &'static str,
    bytes: &'static [u8],
}

impl MailFixture {
    /// Return the raw RFC822 bytes exactly as stored on disk.
    #[must_use]
    pub const fn bytes(self) -> &'static [u8] {
        self.bytes
    }

    /// Decode the fixture as UTF-8 for text-oriented assertions.
    pub fn as_str(self) -> Result<&'static str, FixtureError> {
        std::str::from_utf8(self.bytes).map_err(FixtureError::Utf8)
    }

    /// Parse the top-level RFC822 headers with folded-line support.
    pub fn headers(self) -> Result<ParsedHeaders, FixtureError> {
        parse_headers(self.bytes)
    }
}

/// All checked-in mail fixtures in stable filename order.
pub const MAIL_FIXTURES: &[MailFixture] = &[
    MailFixture {
        name: "attachment-small-text.eml",
        bytes: ATTACHMENT_SMALL_TEXT,
    },
    MailFixture {
        name: "malicious-html.eml",
        bytes: MALICIOUS_HTML,
    },
    MailFixture {
        name: "newsletter-tracking-pixel.eml",
        bytes: NEWSLETTER_TRACKING_PIXEL,
    },
    MailFixture {
        name: "personal-simple.eml",
        bytes: PERSONAL_SIMPLE,
    },
    MailFixture {
        name: "personal-thread-reply.eml",
        bytes: PERSONAL_THREAD_REPLY,
    },
    MailFixture {
        name: "quoted-gmail.eml",
        bytes: QUOTED_GMAIL,
    },
    MailFixture {
        name: "quoted-outlook.eml",
        bytes: QUOTED_OUTLOOK,
    },
    MailFixture {
        name: "receipt-papertrail.eml",
        bytes: RECEIPT_PAPERTRAIL,
    },
];

/// Return all mail fixtures.
#[must_use]
pub const fn mail_fixtures() -> &'static [MailFixture] {
    MAIL_FIXTURES
}

/// Find a mail fixture by filename.
#[must_use]
pub fn mail_fixture(name: &str) -> Option<MailFixture> {
    MAIL_FIXTURES
        .iter()
        .copied()
        .find(|fixture| fixture.name == name)
}

/// Parsed top-level RFC822 headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedHeaders {
    headers: BTreeMap<String, String>,
}

impl ParsedHeaders {
    /// Get a header value by name, case-insensitively.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// Iterate over lower-case header names and unfolded values.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }
}

/// Fixture helper errors.
#[derive(Debug, thiserror::Error)]
pub enum FixtureError {
    /// Fixture bytes were not UTF-8.
    #[error("fixture is not valid UTF-8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    /// Header block was malformed.
    #[error("malformed RFC822 header line: {0}")]
    MalformedHeader(String),
    /// Message did not contain a header/body separator.
    #[error("message is missing an RFC822 header/body separator")]
    MissingHeaderBodySeparator,
}

/// Parse top-level RFC822 headers with basic continuation unfolding.
pub fn parse_headers(bytes: &[u8]) -> Result<ParsedHeaders, FixtureError> {
    let text = std::str::from_utf8(bytes)?;
    let header_block = text
        .split_once("\r\n\r\n")
        .map(|(headers, _body)| headers)
        .or_else(|| text.split_once("\n\n").map(|(headers, _body)| headers))
        .ok_or(FixtureError::MissingHeaderBodySeparator)?;

    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    let mut current_name: Option<String> = None;

    for raw_line in header_block.lines() {
        if raw_line.starts_with(' ') || raw_line.starts_with('\t') {
            let name = current_name
                .as_ref()
                .ok_or_else(|| FixtureError::MalformedHeader(raw_line.to_owned()))?;
            let value = headers
                .get_mut(name)
                .expect("current header name should exist before continuation");
            value.push(' ');
            value.push_str(raw_line.trim());
            continue;
        }

        let (name, value) = raw_line
            .split_once(':')
            .ok_or_else(|| FixtureError::MalformedHeader(raw_line.to_owned()))?;
        let normalized_name = name.trim().to_ascii_lowercase();
        headers.insert(normalized_name.clone(), value.trim().to_owned());
        current_name = Some(normalized_name);
    }

    Ok(ParsedHeaders { headers })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_mail_fixtures_have_required_headers_and_body() {
        for fixture in mail_fixtures() {
            let text = fixture
                .as_str()
                .unwrap_or_else(|err| panic!("{} should be UTF-8: {err}", fixture.name));
            assert!(
                text.contains("\n\n") || text.contains("\r\n\r\n"),
                "{} should contain an RFC822 header/body separator",
                fixture.name
            );

            let headers = fixture
                .headers()
                .unwrap_or_else(|err| panic!("{} headers should parse: {err}", fixture.name));
            for required in [
                "from",
                "to",
                "subject",
                "date",
                "message-id",
                "mime-version",
            ] {
                assert!(
                    headers.get(required).is_some(),
                    "{} should have {required} header",
                    fixture.name
                );
            }
        }
    }

    #[test]
    fn fixture_lookup_reads_expected_messages() {
        let fixture = mail_fixture("newsletter-tracking-pixel.eml").expect("fixture exists");
        let headers = fixture.headers().expect("headers parse");
        assert_eq!(
            headers.get("from"),
            Some("Northwind Weekly <news@northwind.example>")
        );
        assert_eq!(headers.get("x-hail-intended-view"), Some("Feed"));
        assert!(fixture.as_str().expect("utf8").contains("open.gif"));
    }

    #[test]
    fn parser_unfolds_continuation_lines() {
        let parsed =
            parse_headers(b"Subject: hello\r\n\tcontinued\r\nFrom: a@example.test\r\n\r\nbody")
                .expect("headers parse");
        assert_eq!(parsed.get("subject"), Some("hello continued"));
        assert_eq!(parsed.get("FROM"), Some("a@example.test"));
    }
}
