# Provider import architecture

This document records the v1.2 architecture for **provider import mode**: Gmail
or another mailbox provider remains the public mail edge, but hail imports mail
into local Stalwart and continues to run the existing Stalwart/JMAP-backed UI.
It refines Mode P2 from [provider-backed-modes.md](./provider-backed-modes.md)
and the Gmail client choice in
[gmail-api-rust-client-spike.md](./gmail-api-rust-client-spike.md).

## Decision summary

- **Provider as public edge:** Gmail/provider receives public mail, performs its
  normal spam/delivery policy, and exposes messages through its API.
- **Stalwart as local source of truth:** after a message is imported, the hail UI
  and user-facing mail APIs read and mutate Stalwart through JMAP exactly as they
  do in the normal deployment. `hail.db` does not become a mail store.
- **One-way import for v1.2:** Gmail/provider -> Stalwart is the only mailbox
  synchronization direction for imported mail. Hail-side archive/delete/read
  state does not mutate Gmail in v1.2.
- **Outbound is a hook, not sync:** sending may later use provider SMTP/API or a
  Stalwart smarthost, but outbound work must preserve local sent-state semantics
  in Stalwart and avoid turning v1.2 import into bidirectional mailbox sync.
- **OAuth and tokens stay server-side:** the browser only starts/observes OAuth
  through `hail-api`. Refresh tokens are encrypted in `hail.db`; access tokens
  are short-lived memory values and are never returned to the SPA.

## Source-of-truth rules

| State | Source of truth | Provider-import rule |
| --- | --- | --- |
| Public inbound delivery, spam filtering, provider labels | Gmail/provider | Provider-owned. Hail may read labels/history as import hints, but v1.2 does not write them. |
| Imported messages, threads, bodies, attachments, blobs | Stalwart | Hail imports raw RFC822 into Stalwart, then uses existing JMAP paths. |
| Hail views and product keywords (`$hail_imbox`, Screener, Feed, Paper Trail, etc.) | Stalwart keywords plus `hail.db` product tables | Routing runs after import via existing worker/JMAP primitives. Provider labels do not become the authoritative hail view model. |
| Import cursors, provider account metadata, dedupe mappings, sync status, retry state | `hail.db` | Sidecar state only; no duplicate message body/archive store. |
| OAuth refresh tokens | `hail.db`, encrypted with hail server-key crypto | Plaintext only in process memory while refreshing. Never logged or exposed to browser/API clients. |
| Outbound sent-copy state | Stalwart for local UI | Provider send hooks must either import/dedupe the sent copy or rely on Stalwart's local sent object according to the outbound strategy doc. |

The invariant is: **once a provider message has crossed the import boundary,
Stalwart is authoritative for what hail shows**. If Gmail later changes a label,
archive state, read state, or deletion state, v1.2 may use that as future import
input only when explicitly implemented; it must not silently override local hail
state.

## Gmail-to-Stalwart flow

```text
Gmail/provider mailbox (public edge)
  -> Gmail API client wrapper in hail-worker
  -> raw RFC822 bytes + provider metadata
  -> provider import/dedupe transaction in hail.db
  -> Stalwart RFC822 import primitive
  -> Stalwart JMAP messages/blobs/threads
  -> existing hail-worker routing/reconcile
  -> hail-api + SPA through existing mail APIs
```

The initial Gmail implementation uses a hail-owned `reqwest` wrapper with a
`yup-oauth2`-backed token source, as decided in the Gmail API spike. The narrow
read/import surface is:

1. `users.getProfile` to verify the connected mailbox and capture the current
   `historyId`.
2. `users.messages.list` to page historical/import windows, usually by label or
   date slice.
3. `users.messages.get(format=raw)` to fetch base64url RFC822 bytes.
4. `users.history.list` later for incremental sync from the stored cursor, with
   a full-sync fallback when Gmail expires history.

Import must prefer raw RFC822 over parsed provider JSON. MIME parsing,
threading, blobs, search, and message identifiers remain Stalwart's job wherever
Stalwart can own them.

## Major components

### `provider_accounts` schema

`hail.db` needs provider account rows keyed to hail users. The schema should
store at least:

- hail user id / JMAP account id binding;
- provider kind (`gmail` initially), provider account id/email, display email;
- granted OAuth scopes and consent time;
- encrypted refresh token envelope and token metadata;
- latest access-token expiry metadata if useful for observability, not as a
  durable secret;
- sync mode/status (`disabled`, `initial_sync`, `active`, `error`, `revoked`);
- history/cursor checkpoints and backfill window progress;
- last successful sync, last attempted sync, error class/message safe for UI;
- revocation/deauthorization timestamps.

Do not store message bodies or raw RFC822 in this table family. Any import queue
payload that temporarily contains RFC822 must have an explicit retention policy
and must not be treated as the local archive.

### Token crypto and OAuth boundaries

Provider refresh tokens use the same security posture as JMAP bearer tokens:
AES-GCM encryption with key material from `[secrets].server_key` /
`HAIL_SECRETS__SERVER_KEY`, authenticated metadata, and no plaintext in logs.
The OAuth callback is handled by `hail-api`, but production token refresh should
be exposed to importer code through a small trait so worker tests can use fake
tokens and do not require Google credentials.

Initial Gmail scopes are intentionally narrow:

- `gmail.readonly` for one-way import;
- add `gmail.send` only when outbound sending through Gmail is implemented;
- avoid `gmail.modify` and `https://mail.google.com/` until hail deliberately
  mutates provider mailbox state.

The SPA may show connect/disconnect/status UI, but it must never receive access
or refresh tokens. Disconnect should revoke/disable local sync and, where
supported, revoke the provider token.

### Gmail client wrapper

The Gmail wrapper owns HTTP request shapes, pagination, error mapping, quota
backoff, `Retry-After`, and base64url raw decoding. It should expose provider
semantics, not generated Google types, to the importer:

- profile/mailbox identity and history id;
- message ids and page tokens;
- raw RFC822 bytes for a provider message id;
- incremental history deltas when implemented;
- typed retryable/permanent/auth/quota errors.

Tests should use fake HTTP responses and fake token sources. No test should need
real Google credentials.

### Stalwart RFC822 import primitive

The Stalwart import primitive is the only boundary allowed to create local mail
objects from provider RFC822. It should accept raw bytes plus minimal envelope or
import hints and return the Stalwart/JMAP identifiers needed for mapping and
routing.

The primitive must be idempotent from the caller's perspective. If Stalwart
already has the same provider message or RFC822 `Message-ID`, the importer should
recover the existing local identity or classify the item as an intentional
skip/duplicate rather than creating another visible copy.

### Dedupe mapping

`hail.db` should record a durable mapping from provider identity to local
Stalwart identity, for example:

- provider account id;
- provider message id and thread id;
- Gmail `historyId` or source cursor when imported;
- RFC822 `Message-ID` and content hash where available;
- Stalwart/JMAP message id, mailbox/thread ids where returned;
- import status and timestamps.

The provider message id is the primary idempotency key for Gmail imports. RFC822
`Message-ID` and content hashes are secondary duplicate signals because they can
be missing, reused, or differ between provider copies. The mapping lets retries
resume safely after crashes between fetch, import, route, and status update.

### Sync scheduler

Provider import runs in `hail-worker` as cancellation-aware jobs. The scheduler
should support:

- initial historical backfill in bounded pages/windows;
- per-account incremental sync from stored Gmail history cursors;
- retry with backoff for transient provider/network/Stalwart failures;
- quota-aware pacing and `Retry-After` handling;
- per-account pause/disable/revoke states;
- clean shutdown with no long await lacking a cancellation branch.

The scheduler must not block the existing Stalwart EventSource/routing loop for
other users. Imported messages should flow into the same routing/reconcile path
used for ordinary inbound mail.

### Status API and UI

`hail-api` should expose task-oriented provider account/status endpoints for the
SPA:

- connect OAuth start/callback;
- list connected provider accounts and granted scopes;
- show initial sync progress, last success, last safe error, and whether import
  is paused/revoked;
- request pause/resume/disconnect;
- expose coarse counts and timestamps, not message bodies or tokens.

The UI should be explicit that Gmail/provider is still the public mailbox while
hail imports a local Stalwart copy. During one-way v1.2 import, UI copy must not
promise that Gmail archive/delete/read state is changed by hail actions.

### Review and smoke gates

Provider import changes the mail ingress boundary and token-risk profile, so it
needs explicit gates before shipping:

- code review for provider import boundaries, dedupe, token handling, and
  cancellation behavior;
- test review for idempotency, fake-provider confidence, and Stalwart import
  coverage;
- Gmail fixture tests for historical and incremental import;
- idempotency tests that re-run failed imports and verify no duplicate visible
  mail;
- human smoke with a real Gmail account connecting, importing, and viewing mail
  through the normal hail UI;
- outbound/sent-copy smoke when provider send hooks are enabled.

## Failure, retry, and idempotency principles

Provider import is allowed to be eventually consistent. It is not allowed to
create silent duplicates, expose secrets, or lose the ability to resume.

- **Checkpoint after durable effects.** Advance Gmail history/backfill cursors
  only after dedupe mapping and Stalwart import results are durably recorded.
- **Make every stage restartable.** A crash after fetching raw RFC822, after
  Stalwart import, or after routing should converge on one local message when
  the job retries.
- **Classify errors.** Auth/revocation, quota, transient network, malformed
  provider response, malformed RFC822, and Stalwart import errors need separate
  handling and UI-safe status.
- **Respect provider backoff.** Honor `Retry-After`, exponential backoff, and
  quota limits. Do not hot-loop failed accounts.
- **Keep poisoned messages isolated.** A permanently malformed or rejected
  message should be recorded with safe metadata and skipped or quarantined
  without blocking the entire account forever.
- **Do not log content or secrets.** Logs and audit events may include provider
  account ids, message ids, local ids, status, and error classes; never tokens,
  raw RFC822, message bodies, or attachment content.
- **Preserve local UI truth.** Provider-side changes after import must not
  silently undo hail-side routing, Screener decisions, clips, notes, or local
  read/archive/delete semantics unless a future bidirectional-sync decision says
  so explicitly.

## Outbound strategy hooks

Provider import mode must leave room for outbound delivery without forcing it
into the initial importer:

- Stalwart may submit through a provider SMTP smarthost; or
- hail may call Gmail `messages.send` using an added `gmail.send` scope; or
- a later provider-specific sender may import/dedupe the provider's sent copy.

Whichever strategy is chosen, the local Stalwart sent state remains what the hail
UI reads. Provider-created sent copies must be deduped against local sent mail so
the thread does not show duplicate outbound messages.

## Non-goals for v1.2

- Bidirectional Gmail label/archive/delete/read synchronization.
- Gmail as the source of truth for hail views after import.
- A hail-native message/blob store replacing Stalwart.
- Browser-side Gmail API access.
- Broad Gmail scopes or destructive provider mutations.
- Support for every provider API before Gmail import is proven.
