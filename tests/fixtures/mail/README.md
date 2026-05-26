# Synthetic mail fixtures

This directory contains reusable raw RFC822 (`.eml`) messages for unit tests,
local/direct mail testbeds, Cloudflare-assisted smoke tests, and human smoke
runs. The messages use reserved `.example` / `.test` domains and contain no real
credentials or personal data.

The helper crate `crates/hail-test` embeds these files, lists the corpus, reads
raw bytes, and parses basic top-level headers.

## Fixtures

- `personal-simple.eml` — plain-text personal message from a new sender. Use for
  Screener pending state, sender approval, and basic Imbox rendering.
- `personal-thread-reply.eml` — reply to `personal-simple.eml` with
  `In-Reply-To` and `References`. Use for thread assembly and quoted plain-text
  rendering.
- `newsletter-tracking-pixel.eml` — multipart newsletter with `List-ID`,
  `List-Unsubscribe`, campaign links, and a 1x1 remote `open.gif`. Use for Feed
  classification and tracker stripping/counting.
- `receipt-papertrail.eml` — transactional receipt with order number, itemized
  total, payment tail, and shipping address. Use for Paper Trail routing and
  receipt rendering.
- `attachment-small-text.eml` — personal message with a base64 text attachment
  named `planning-notes.txt`. Use for attachment listing/download smoke tests.
- `quoted-gmail.eml` — Gmail-style reply with `gmail_quote` HTML and matching
  plain-text `On ... wrote:` quote. Use for quoted-reply stripping.
- `quoted-outlook.eml` — Outlook-style reply with `-----Original Message-----`
  plain text and HTML `From/Sent/To/Subject` quote header. Use for quoted-reply
  stripping.
- `malicious-html.eml` — multipart message containing a `<script>`, `onerror`,
  `javascript:` links, CSS `javascript:` URL, and a remote pixel. Use only for
  sanitizer and tracker tests.

## Gmail import fixture set

`crates/hail-test::gmail_import_fixtures` maps the raw RFC822 corpus into a
provider-shaped Gmail catalog. It reuses the `.eml` files above and adds stable
Gmail ids, thread ids, history ids, expected bare RFC822 `Message-ID` values,
and optional routing expectations. The catalog covers:

- raw RFC822 historical import and provider mapping persistence;
- dedupe/idempotency when a second Gmail id exposes the same RFC822 `Message-ID`;
- expired Gmail history cursor fallback to bounded full import;
- routing outcomes for Screener, Imbox, Feed, and Paper Trail;
- explicit sent-copy import as one-way/local state (Gmail labels are read-only
  import bounds, not mirrored state).

The helper also emits Gmail-shaped JSON for `messages.list`, `messages.get` raw
responses, and `history.list` message-added records for protocol/client tests.

## Conventions

- Keep every fixture valid UTF-8 unless a test explicitly requires binary body
  data.
- Include at least `From`, `To`, `Subject`, `Date`, `Message-ID`,
  `MIME-Version`, and `Content-Type` headers.
- Use stable `Message-ID` values so future tests can assert threading.
- Preserve raw RFC822 shape; prefer adding new fixture files over mutating an
  existing fixture that downstream tests may depend on.
- `X-Hail-Intended-View` and `X-Hail-Fixture-Use` are test-only hints. They are
  not production mail metadata.
