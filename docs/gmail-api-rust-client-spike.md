# Gmail API Rust client spike

This spike records the recommended Rust approach for Gmail-backed provider import
(Mode P2 in [provider-backed-modes.md](./provider-backed-modes.md)): Gmail remains
the public mailbox, `hail-worker` fetches messages through the Gmail API, and raw
RFC822 is imported into local Stalwart so hail's existing JMAP/UI path stays
unchanged.

## Decision

Use a small hail-owned `reqwest` Gmail wrapper plus `yup-oauth2` for Google OAuth
token refresh.

Do **not** make `google-gmail1` the provider-import boundary for the first
implementation. It remains a useful reference and possible future replacement,
but a thin wrapper is easier to test, easier to bind to encrypted refresh-token
storage, and has a smaller dependency/ergonomics footprint for the narrow calls
we need first.

## Minimum Gmail calls for initial import

Initial/historical import only needs:

1. `GET /gmail/v1/users/me/profile`
   - proves the access token maps to the expected mailbox;
   - returns the current mailbox `historyId`, useful as a later incremental
     sync checkpoint.
2. `GET /gmail/v1/users/me/messages`
   - use `maxResults <= 500`;
   - page with `pageToken`;
   - constrain with `labelIds` such as `INBOX`, `SENT`, or `CATEGORY_*` when we
     want phased backfill;
   - use `q` for date windows or migration slices, but note that `q` is not
     allowed with the `gmail.metadata` scope.
3. `GET /gmail/v1/users/me/messages/{id}?format=raw`
   - returns base64url-encoded RFC822 in the `raw` field;
   - the provider importer decodes this and hands bytes to the Stalwart import
     primitive;
   - tests should inject fake JSON and never require real Google credentials.

Future incremental sync should add:

- `users.history.list(startHistoryId=...)` for changes since the stored cursor;
- full-sync fallback when Gmail returns expired-history errors;
- label listing/mapping before using labels for hail routing.

Google references:

- Gmail messages list: <https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.messages/list>
- Gmail messages get: <https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.messages/get>
- Gmail scopes: <https://developers.google.com/workspace/gmail/api/auth/scopes>

## Compared Rust approaches

### `google-gmail1` + `yup-oauth2`

Crates: <https://crates.io/crates/google-gmail1> and
<https://crates.io/crates/yup-oauth2>.

Pros:

- generated from Google's discovery/API description;
- broad typed coverage: messages, history, labels, drafts, send/import, etc.;
- generated builders already expose the relevant call shapes, e.g.
  `messages_list("me").max_results(...).page_token(...).doit().await` and
  `messages_get("me", id).format("raw").doit().await`;
- shares the `yup-oauth2` ecosystem for Google installed/web flows and token
  refresh.

Cons:

- pulls in the generated Google API stack (`hyper`, `hyper-util`,
  `hyper-rustls`, `google-apis-common`) when hail already uses `reqwest`;
- generated API ergonomics are noisier than our three-call import surface;
- token persistence hooks are designed around `yup-oauth2::TokenStorage`, while
  hail needs explicit encrypted-at-rest provider-account storage and audit
  boundaries;
- harder to mock at the HTTP/request-shape boundary without accepting generated
  types everywhere.

Use it if the Gmail integration expands quickly to broad Gmail surface area and
we want generated coverage more than wrapper control.

### `oauth2` + `reqwest` hand-rolled

Crates: <https://crates.io/crates/oauth2> and hail's existing `reqwest` stack.

Pros:

- smallest conceptual boundary for OAuth standards;
- hail owns all storage, refresh, retry, and HTTP behavior;
- easy to unit-test exact request URLs/headers and fake responses.

Cons:

- Google OAuth details are still ours to wire: `access_type=offline`, consent
  prompting when needed, refresh-token exchange, token expiry, and error shapes;
- more code for no import benefit versus using `yup-oauth2` as the Google token
  source;
- easy to accidentally miss Google-specific conventions already handled by
  `yup-oauth2`.

Use it if `yup-oauth2` cannot cleanly refresh from encrypted database tokens.

### `reqwest` Gmail wrapper + `yup-oauth2` token source (recommended)

Crates: `reqwest` for HTTP, `serde` for response structs, `yup-oauth2` for
Google OAuth token acquisition/refresh.

Pros:

- import code depends on a narrow `GmailTokenSource` trait returning a bearer
  access token; production can adapt `yup-oauth2`, tests can use a fake;
- request shapes stay obvious and stable;
- no generated Google API types leak into importer, Stalwart import, or DB
  schema code;
- preserves the option to swap in `google-gmail1` later behind the same
  higher-level provider-client trait if the surface grows.

Cons:

- hail must maintain small request/response structs for each Gmail endpoint;
- hail must implement Gmail error mapping, retry/backoff, pagination, quota
  accounting, and `Retry-After` handling;
- less compile-time coverage when Google adds/removes fields than generated
  clients.

This is the best first implementation because initial import is intentionally
narrow and correctness hinges more on storage/idempotency/import semantics than
on generated Gmail type breadth.

### Other crates

The `gmail` crate (<https://crates.io/crates/gmail>) is a fluent OpenAPI-based
client. It is worth re-checking before implementation, but this spike does not
recommend it as the first boundary because it is less common than the generated
Google API crates and does not remove our need for explicit token storage,
retry/backoff, and import idempotency. Smaller crates such as `rust-gmail` focus
on sending or niche workflows and do not cover historical/incremental import.

Paid unified APIs such as Nylas/Unipile are intentionally out of scope: they add
third-party infrastructure and conflict with hail's self-hosted provider-import
goal.

## OAuth scopes

For the initial read/import spike and historical import:

- Required: `https://www.googleapis.com/auth/gmail.readonly`
  - permits `messages.list`, `messages.get(format=raw)`, labels/history reads,
    and profile reads;
  - sufficient for one-way Gmail -> Stalwart import.

For later outbound through Gmail:

- Add only when implementing send: `https://www.googleapis.com/auth/gmail.send`.

Avoid initially:

- `https://mail.google.com/` — full mailbox access including destructive
  operations; too broad for v1.2 import.
- `https://www.googleapis.com/auth/gmail.modify` — not needed for one-way import
  unless hail begins mutating Gmail labels/archive/read state.
- `https://www.googleapis.com/auth/gmail.metadata` — useful for low-privacy
  metadata-only scans, but it cannot fetch `format=raw` message bodies and does
  not allow `q` on `messages.list`; it is insufficient for import.

OAuth setup notes:

- request offline access so Google returns a refresh token;
- store refresh tokens encrypted at rest in provider account storage;
- never log access tokens, refresh tokens, message bodies, or raw RFC822;
- expose token acquisition through a trait so tests and local compile checks do
  not need Google credentials.

## Compile-only wrapper shape

A small skeleton lives in `crates/hail-worker/src/gmail_client.rs`. It proves the
minimum request/response boundary without adding runtime Google credentials to
tests:

- `GmailTokenSource` — async trait returning a bearer token;
- `GmailClient<T>` — reqwest-backed client with injectable base URL;
- `profile()` — `GET users/me/profile`;
- `list_messages()` — `GET users/me/messages` with page/query/label params;
- `get_raw_message()` — `GET users/me/messages/{id}?format=raw` and decode
  base64url raw RFC822 bytes.

The skeleton is intentionally not wired into `supervisor` yet. Follow-up tasks
should decide production OAuth adapter details after provider account schema and
token encryption land.

## Wrapper foundation notes

`gmail-client-wrapper` promoted the compile-only skeleton into a foundation for
later provider import without wiring it into the scheduler yet:

- the `GmailTokenSource` trait remains the OAuth/storage boundary, so tests use
  static fake tokens and production can later adapt encrypted refresh-token or
  `yup-oauth2` handling;
- Gmail JSON errors are mapped into stable categories (`Unauthorized`,
  `PermissionDenied`, `NotFound`, `RateLimited`, `Transient`, `BadRequest`, and
  `Other`) while retaining status, reason, message, and delta-seconds
  `Retry-After` when present;
- retryable request failures, 429/quota responses, and 5xx responses retry with
  bounded exponential full-jitter backoff, honoring delta-seconds `Retry-After`
  up to the configured maximum delay;
- pagination helpers support both buffered `list_all_messages` and page callback
  traversal, with repeated-page-token detection to avoid infinite loops;
- fake HTTP tests cover request shape, bearer headers, query encoding,
  pagination, retry, raw RFC822 decoding, and error mapping without Google
  credentials.

The wrapper still intentionally exposes only profile, message list, and
`format=raw` fetch. Incremental history sync, label mapping, send, and scheduler
integration should remain in their follow-up tasks.

## Recommended follow-up task adjustments

Existing tasks are still directionally correct. Adjust implementation detail as
follows:

- `gmail-client-wrapper`: implement the reqwest wrapper from the skeleton,
  including error mapping, pagination helpers, Gmail quota/backoff handling,
  tests with fake HTTP, and a production `yup-oauth2` token-source adapter.
- `gmail-oauth-api`: build Google OAuth authorization/callback endpoints that
  request `gmail.readonly` first; defer `gmail.send` until outbound is enabled.
- `provider-accounts-schema` + `provider-token-crypto`: store provider refresh
  tokens encrypted with the same server-key posture as JMAP tokens; include
  provider user id/email, scopes granted, token expiry metadata, and revocation
  state.
- `gmail-initial-sync`: depend on `gmail-client-wrapper` and
  `stalwart-rfc822-import-primitive`; import `format=raw` bytes without storing
  message bodies in `hail.db`.
- Add or refine a future `gmail-history-sync-fallback` task if incremental sync
  does not already cover expired `historyId` full-sync recovery and audit
  logging.
