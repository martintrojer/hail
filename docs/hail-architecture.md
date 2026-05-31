# hail architecture

> Status: design doc for the unified hail architecture. Captures motivation,
> layering, and the key technical decisions. Implementation tasks live in mu
> under workstream `hail`, not here.

## Motivation

Two real-world deployments of hail are useful:

- **Gmail flavour** — connect a Gmail account, get a HEY-style UX over it.
  Inbound mail arrives at Google. Outbound mail goes via Gmail SMTP using
  XOAUTH2. No DNS, no SPF/DKIM, no MTA to run.
- **Self-host flavour** — run a full Stalwart mail server with hail as the
  webmail UI. Own your domain, own deliverability, own everything.

These look like different products but they are the same product with a
different upstream. The whole hail UX — Screener, Imbox/Feed/Papertrail,
Set Aside, Reply Later, Bubble Up, Power Through, labels, notes, workflow
rules, compose with tracker-stripping and remote-image policy, j/k navigation,
threaded grouping, sectioned views, sender reputation, scheduled send — is
backend-agnostic. It operates on "messages with keywords and mailboxes".

We unify on a single architecture so:

- One SPA, one set of routes, one docs site, one CI, one test suite.
- Every feature lands once and works in both flavours.
- The Stalwart-on-everywhere assumption that bled into v1.2 is replaced by a
  clean seam.
- Adding M365 / Fastmail / IMAP later is a new backend implementation, not a
  fork.

The cost is one trait, one cache layer, and a config knob for the flavour.
The benefit is that the variant we built last week (provider-import-mode atop
Stalwart) and the variant we want next (Gmail-only with local archive) are
two configurations of the same binary.

## High-level shape

```
+-----------------------------------------------+
| SPA (webapp)                                  |
+-----------------------------------------------+
                       v
+-----------------------------------------------+
| hail-api (routes, views, screener, classify,  |
|          compose, workflow, labels, notes)    |
+-----------------------------------------------+
                       v
+-----------------------------------------------+
| hail-cache (read-through + write-through)     |
|   - SQLite metadata + FTS5                    |
|   - filesystem CAS blob store                 |
+-----------------------------------------------+
              v                 v
       (cache miss /      (writes / send /
        sync events)       label modify)
              v                 v
+-----------------------------------------------+
| MailBackend trait                             |
+-----------------+-----------------------------+
                  |
       +----------+------------+
       v                       v
+--------------+        +-----------------+
| GmailBackend |        | JmapBackend     |
| (Gmail REST  |        | (Stalwart /     |
|  + XOAUTH2   |        |  any JMAP       |
|  SMTP)       |        |  server)        |
+--------------+        +-----------------+
```

Everything above `MailBackend` is shared. Everything below is per-flavour.
The cache is universal: it is the right architecture for both flavours and
its absence (live-only mode) is a config knob, not a code path.

## Layers

### SPA

Unchanged in shape. The SPA continues to talk to hail-api over its existing
REST surface. Backend-aware behaviour is driven by a small `capabilities`
object hail-api exposes (e.g. `supports_bulk_import`, `supports_eventsource`),
not by hand-rolled feature flags.

### hail-api

Routes stay the same. Internally they call into `hail-cache` instead of
directly into `crates/hail-jmap`. The conceptual change is:

- `JmapMailViewProvider` becomes one of two `MailBackend` implementations
  rather than the only path.
- View / list / thread / search / compose routes acquire their data from
  the cache; the cache decides when to hit the backend.

Routes are flavour-agnostic. They do not know whether they are talking to
Gmail or to a JMAP server.

### hail-cache

Read-through + write-through cache. Every read goes through it. Every
mutation goes through it. The cache:

- Returns cached data for cache hits.
- On miss, fetches from the backend, populates the cache subject to the
  configured cache mode and budget, and returns.
- On mutation, applies optimistically to the cache, queues the mutation
  on the backend, and on backend success durably confirms.

The cache has three modes (see below): `off` (live, no cache),
`bounded` (keep recent / metadata-full / blobs within budget), and `full`
(mirror everything).

### MailBackend

A small async trait. The full operation surface is documented under
"MailBackend trait" below. Two implementations ship in tree:

- `hail-gmail::GmailBackend` — wraps the existing Gmail OAuth + REST client
  and adds XOAUTH2 SMTP send.
- `hail-jmap::JmapBackend` — wraps `jmap-client` and exposes the Stalwart-
  compatible operations.

A backend is one trait impl plus a small `Capabilities` const describing what
it supports. New backends (M365, Fastmail, IMAP) plug in here without
touching anything above.

### Blob store

Content-addressed filesystem under `/var/lib/hail/blobs/`. Key is BLAKE3
of the uncompressed bytes. Files are zstd-compressed at rest. Layout:

```
blobs/
  ab/cd/abcd1234...ef89.eml.zst         # RFC822 message body
  ab/cd/abcd1234...ef89.att.zst         # attachment payload
```

Two-level fanout (first 4 hex chars) caps directory size for filesystems
that don't love huge dirs.

### SQLite

Metadata and indexes. Schema additions land in `crates/hail-db/migrations`.
The blob bytes themselves do not live in SQLite — only the blob_id reference.
Full-text search runs through FTS5 over the text-extracted body.

### Worker

`hail-worker` is largely preserved. It is the only writer of:

- Backend sync (poll/watch + apply).
- Scheduled send.
- Workflow rule evaluation.
- Outbound write queue draining.
- Cache eviction sweep.

The worker is backend-agnostic too: it talks to `hail-cache`, which talks
to the configured `MailBackend`.

## Crate layout (target)

```
crates/
  hail-core          shared types: keywords, MailClassification, sanitizer,
                     mail render, sender normalization. Already crate-shaped;
                     unchanged.

  hail-db            SQLite migrations + queries. Schema grows to include
                     the cache tables (messages, message_keywords, attachments,
                     FTS5 virtual table).

  hail-blob-store    NEW. Content-addressed zstd filesystem store. Implements
                     put / get / delete / verify / sweep. Used by hail-cache.

  hail-backend       NEW. Defines the MailBackend trait + Capabilities +
                     shared backend-agnostic types (Msg, FullMsg, Envelope,
                     Cursor, Change). No implementations.

  hail-gmail         NEW. GmailBackend impl. Wraps the existing OAuth +
                     REST client (moved out of hail-worker) and the
                     XOAUTH2 SMTP path (already prototyped in
                     gmail_outbound_smtp). One self-contained backend
                     crate.

  hail-jmap          Existing. JmapBackend impl. Internals reorganised so
                     that the trait impl is the public API; private
                     helpers stay private.

  hail-cache         NEW. The read-through / write-through cache. Holds:
                       - cache policy + mode
                       - eviction sweeper
                       - outbound write queue
                       - sync application (incoming events from backend)
                     Depends on hail-db + hail-blob-store + hail-backend.

  hail-api           Routes. Depends on hail-cache only. Does not depend on
                     hail-gmail / hail-jmap directly; the chosen backend is
                     injected at startup.

  hail-worker        Long-running tasks. Depends on hail-cache + the
                     chosen backend (via runtime dispatch over hail-backend).
                     Scheduled send, workflow rules, sync poll, cache sweep.
```

Today's tree maps as follows:

- `crates/hail-worker/src/gmail_*.rs` → `crates/hail-gmail/` as the new
  backend impl behind the trait.
- `crates/hail-jmap` stays but the `JmapMailViewProvider` style API is
  replaced by the `MailBackend` trait surface.
- `crates/hail-api/src/routes/views.rs` collapses: `JmapMailViewProvider`
  goes away, the routes consume `hail-cache`. The MailBackend trait does
  not appear in route signatures.

## MailBackend trait

The trait is intentionally narrow. We push policy upward into the cache.

```rust
#[async_trait]
pub trait MailBackend: Send + Sync + 'static {
    fn capabilities(&self) -> &'static Capabilities;

    // Reads
    async fn list_message_ids(&self, q: &Query, page: &PageRequest)
        -> Result<Page<BackendMsgId>>;
    async fn get_message(&self, id: &BackendMsgId)
        -> Result<RawMessage>;          // RFC822 + JMAP-shaped metadata
    async fn fetch_blob(&self, id: &BlobRef) -> Result<Bytes>;

    // Mutations
    async fn set_keywords(
        &self,
        id: &BackendMsgId,
        add: &[Keyword],
        remove: &[Keyword],
    ) -> Result<()>;
    async fn move_to_role(&self, id: &BackendMsgId, role: MailboxRole)
        -> Result<()>;
    async fn delete_permanently(&self, id: &BackendMsgId) -> Result<()>;

    // Send
    async fn send(
        &self,
        rfc822: &[u8],
        envelope: &Envelope,
    ) -> Result<SubmissionId>;

    // Sync
    async fn poll_changes(&self, cursor: &SyncCursor)
        -> Result<(Vec<Change>, SyncCursor)>;
    async fn watch_changes(&self) -> BoxStream<'static, Change>;
        // GmailBackend may implement this as periodic poll
        // internally; the cache treats both the same.

    // Account-shape ops (capability-gated; not all backends support these)
    async fn list_mailboxes(&self) -> Result<Vec<Mailbox>>;
    async fn list_principals(&self) -> Result<Vec<Principal>>;   // JMAP only
}
```

`Capabilities` is a struct of bools + small typed knobs:

```rust
pub struct Capabilities {
    pub supports_initial_import: bool,   // Gmail: true; JMAP: false (server already has it)
    pub supports_eventsource: bool,      // JMAP: true; Gmail: false (worker polls)
    pub supports_principals_admin: bool, // JMAP: true; Gmail: false
    pub supports_send: bool,
    pub native_threading: bool,          // Gmail: true; JMAP: server-derived
    pub max_attachment_size: u64,
    pub label_path_separator: char,
}
```

Conformance tests live in `crates/hail-backend/tests/`. Every impl runs the
same suite against an in-memory test fixture.

## Cache modes

Cache policy is a config knob, not a flavour distinction.

```toml
[mail.cache]
mode             = "bounded"      # off | bounded | full
keep_days        = 90             # bounded: time horizon
keep_max_msgs    = 50_000         # bounded: count cap
keep_max_bytes   = "5 GiB"        # bounded: size cap (humanized)
backfill         = "incremental"  # off | incremental
```

### off (live)

- No `messages` rows, no blob storage, no FTS5 population.
- Every read hits the backend on the hot path.
- Only sensible when the backend is local (JMAP-on-same-host) and storage
  is precious.
- Mutations still go through hail-cache for the optimistic-update + retry
  queue; they just do not populate cache rows.

### bounded (default for Gmail flavour)

- **Metadata** for every message that sync sees is stored. This is cheap
  (a few hundred bytes per message) and powers instant list views.
- **Raw bodies + attachments** are stored only within the budget:
  - newer than `keep_days`, OR
  - among the most recently accessed `keep_max_msgs`, OR
  - up to `keep_max_bytes` total on disk.
  Whichever cap is the most restrictive wins.
- LRU + age eviction runs as a sweeper in `hail-worker`.
- Pinned items never evict regardless of budget: drafts, scheduled sends,
  anything tied to a hail-side row (notes, screener pending, set-aside,
  reply-later, bubble-up).
- Cache miss on a body falls through to `backend.get_message()` and, if
  the message is still inside the budget at fetch time, the body is
  written back into the cache.

### full (mirror)

- Every message body + attachment cached forever.
- The "I want a complete offline archive of my Gmail" mode.
- Default for users with `backfill="incremental"` and no other cap.

### Search semantics

FTS5 indexes the *cached* body text. Beyond the budget, search falls
through to `backend.list_message_ids(Query::Search(...))` and results are
merged. The SPA labels which results are local vs. backend-served so the
operator knows what is searchable offline.

### Independence: cache size vs backfill

Two orthogonal knobs:

- `backfill = off | incremental` — do we import historical mail at all?
- `cache.mode = off | bounded | full` — how much do we keep locally?

`backfill="off"` + `cache.mode="bounded"` is the "live in a nicer world from
today" configuration: no historical import, cache only holds the recent
post-installation window. Older Gmail mail remains in Gmail and is still
visible through search-fallthrough but is not pre-classified by the screener
or counted in the imbox view.

`backfill="incremental"` + `cache.mode="full"` is the "I want it all locally"
configuration: every historical message imported, every body kept.

`backfill="incremental"` + `cache.mode="off"` is nonsense and rejected at
config load.

## Sync model

Sync is unified at the `MailBackend::poll_changes` / `watch_changes` seam.
Below the seam, backends translate their native protocol into a common
`Change` enum:

```rust
pub enum Change {
    MessageCreated   { id: BackendMsgId, raw_ref: Option<RawMessage> },
    MessageUpdated   { id: BackendMsgId, keywords_added: Vec<Keyword>,
                       keywords_removed: Vec<Keyword> },
    MessageDeleted   { id: BackendMsgId },
    MailboxRoleChanged { id: BackendMsgId, role: MailboxRole },
}
```

- Gmail backend implements `poll_changes` via `users.history.list` keyed
  on `historyId`. `watch_changes` is a periodic poll loop (every 30s).
- JMAP backend implements `poll_changes` via `Email/changes` keyed on the
  JMAP state token. `watch_changes` uses the EventSource stream.

The cache layer is the only consumer. It applies each `Change` to the
local SQLite + blob store, then publishes a small event for the SPA's
existing EventSource (worker → hail-api → SPA).

### Last-writer-wins

When the cache's optimistic state and a `Change` event disagree (the
operator clicked Read locally; meanwhile Gmail's web UI marked the same
thread Unread), last-writer-wins by timestamp. The outbound queue carries
the local timestamp; the backend change carries the backend timestamp; the
cache applies whichever is newer.

Hard exception: Trash is operator-authoritative. If hail moved a message
to Trash and Gmail then removed `TRASH`, hail's local trash wins for the
outbound push and the next sync reconciles. Hail never issues a permanent
delete during sync.

### Outbound write queue

Every mutation goes through:

1. Apply optimistically to the cache.
2. Enqueue an outbound row.
3. Worker drains the queue: batches mutations into the smallest number of
   backend calls (`labels.batchModify` for Gmail, `Email/set` for JMAP).
4. On success, mark the row applied. On failure, exponential backoff +
   attempt cap, then surface as `last_error_class` on the account.

This is the same machinery as the bidirectional-sync work, generalised.
For Gmail flavour it is the only write path. For JMAP flavour it lets us
keep optimistic UX without a synchronous JMAP roundtrip.

### Offline mutations

While the backend is unreachable, mutations still apply to the cache and
queue. The worker drains when connectivity returns. The SPA shows a small
"N pending sync" affordance when the queue is non-empty.

The composer's outbox uses the same queue: send is just a mutation whose
backend call is `MailBackend::send`.

## Schema additions

All additions land in `crates/hail-db/migrations/` as new sequential
migrations. The existing schema (sessions, users, screener_rules,
thread_notes, labels, provider_accounts, workflow_rules, ...) stays put.

### Core cache tables

```sql
-- One row per known message across all accounts. Metadata-only: cheap and
-- always populated regardless of cache mode (except cache.mode='off').
CREATE TABLE messages (
  id              INTEGER PRIMARY KEY,
  account_id      INTEGER NOT NULL REFERENCES mail_accounts(id)
                                   ON DELETE CASCADE,
  backend_msg_id  TEXT    NOT NULL,            -- Gmail id / JMAP id
  thread_id       TEXT    NOT NULL,            -- Gmail thread / JMAP threadId
  internal_date   INTEGER NOT NULL,            -- seconds since epoch
  from_addr       TEXT    NOT NULL,            -- normalized lowercase
  subject         TEXT    NOT NULL,
  preview         TEXT    NOT NULL,            -- sanitized excerpt
  size_bytes      INTEGER NOT NULL,
  body_blob_id    TEXT,                        -- references blob store; NULL when evicted
  body_text       TEXT,                        -- decoded plaintext for FTS; NULL when evicted
  inserted_at     TEXT    NOT NULL,            -- ISO-8601
  accessed_at     TEXT    NOT NULL,            -- ISO-8601; LRU input
  pinned          INTEGER NOT NULL DEFAULT 0,  -- 1 = never evict body
  UNIQUE (account_id, backend_msg_id)
);
CREATE INDEX idx_messages_thread     ON messages(account_id, thread_id);
CREATE INDEX idx_messages_received   ON messages(account_id, internal_date DESC);
CREATE INDEX idx_messages_from       ON messages(account_id, from_addr);
CREATE INDEX idx_messages_lru        ON messages(account_id, pinned, accessed_at)
                                       WHERE body_blob_id IS NOT NULL;

CREATE TABLE message_keywords (
  message_id  INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  keyword     TEXT    NOT NULL,    -- includes $seen, $hail_imbox, label names
  PRIMARY KEY (message_id, keyword)
);
CREATE INDEX idx_message_keywords_keyword ON message_keywords(keyword);

CREATE TABLE attachments (
  id           INTEGER PRIMARY KEY,
  message_id   INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  filename     TEXT    NOT NULL,
  mime_type    TEXT    NOT NULL,
  size_bytes   INTEGER NOT NULL,
  blob_id      TEXT,                -- NULL when not cached (bounded mode)
  inline       INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_attachments_message ON attachments(message_id);

CREATE VIRTUAL TABLE messages_fts USING fts5(
  from_addr, subject, body_text,
  content='messages', content_rowid='id'
);
-- Standard fts5 triggers (insert/update/delete) ship in the migration.
```

### Cache policy

```sql
CREATE TABLE cache_policy (
  account_id        INTEGER PRIMARY KEY REFERENCES mail_accounts(id),
  mode              TEXT    NOT NULL CHECK (mode IN ('off','bounded','full')),
  keep_days         INTEGER,
  keep_max_msgs     INTEGER,
  keep_max_bytes    INTEGER,
  backfill          TEXT    NOT NULL CHECK (backfill IN ('off','incremental')),
  updated_at        TEXT    NOT NULL
);
```

### Outbound write queue

```sql
CREATE TABLE outbound_changes (
  id              INTEGER PRIMARY KEY,
  account_id      INTEGER NOT NULL REFERENCES mail_accounts(id)
                                   ON DELETE CASCADE,
  backend_msg_id  TEXT    NOT NULL,
  change_type     TEXT    NOT NULL CHECK (change_type IN (
                    'read','unread','keyword_add','keyword_remove',
                    'role_move','trash','untrash','permanent_delete',
                    'send')),
  payload_json    TEXT    NOT NULL CHECK (json_valid(payload_json)),
  created_at      TEXT    NOT NULL,
  applied_at      TEXT,
  attempt_count   INTEGER NOT NULL DEFAULT 0,
  last_error      TEXT
);
CREATE INDEX idx_outbound_pending ON outbound_changes(account_id, applied_at)
  WHERE applied_at IS NULL;
```

### Accounts

`provider_accounts` is renamed `mail_accounts` (one row per backend
connection). A new column `backend_kind` ('gmail' | 'jmap' | future) replaces
`provider_kind`. JMAP-flavour installs have exactly one row; Gmail-flavour
installs have one row per connected Gmail account.

The current sync-status / OAuth columns stay. Stalwart-only columns (e.g.
JMAP token + bearer) are nullable and only used when `backend_kind='jmap'`.

### Migration discipline

- All new migrations append; nothing is destructive.
- The first migration after the unified architecture lands renames
  `provider_accounts` to `mail_accounts` via a `CREATE TABLE ... AS SELECT`
  + `DROP` dance and updates foreign keys. This is the only schema rename;
  everything else is additive.

## Blob store

Content-addressed, zstd-compressed, filesystem-backed.

```
${HAIL_BLOB_ROOT:-/var/lib/hail/blobs}/
  <aa>/<bb>/<full-blake3-hex>.eml.zst
  <aa>/<bb>/<full-blake3-hex>.att.zst
```

- Hash function: BLAKE3 over the *uncompressed* bytes.
- Compression: zstd level 3 (good ratio, fast decompress).
- Fanout: first two bytes of the hash become two nested directories. Caps
  any single dir at ~65k entries for any plausible store size.
- Suffix encodes content type: `.eml.zst` for RFC822 bodies, `.att.zst` for
  attachments. The store treats them uniformly; the suffix is for operator
  inspection.
- Dedup is free: two copies of the same forwarded attachment land on the
  same hash → one file on disk, two references in `attachments`.

API:

```rust
pub trait BlobStore: Send + Sync {
    async fn put(&self, kind: BlobKind, bytes: &[u8]) -> Result<BlobId>;
    async fn get(&self, id: &BlobId) -> Result<Bytes>;
    async fn delete(&self, id: &BlobId) -> Result<()>;
    async fn exists(&self, id: &BlobId) -> Result<bool>;
    async fn verify(&self, id: &BlobId) -> Result<()>;     // BLAKE3 round-trip
    async fn sweep_unreferenced(&self, db: &SqlitePool) -> Result<SweepStats>;
}
```

`sweep_unreferenced` joins blob files against `messages.body_blob_id` and
`attachments.blob_id`. Anything on disk with no reference is removed. Safe
to run while the worker is live; uses a two-phase mark + sweep with a
grace window to avoid racing in-flight inserts.

### Why filesystem, not SQLite-BLOB and not RocksDB

We considered storing bytes inside SQLite (single-file simplicity) and
storing everything in RocksDB (Stalwart-style). Both were rejected:

- **SQLite-BLOB**: a single ~50 GB database has unpleasant VACUUM behaviour
  and forces backup tools to copy the whole file on every change. Splitting
  bytes out keeps the DB at a few GB at most and lets `rsync` / `restic` /
  any block-dedup backup do its job on the blob dir.
- **RocksDB**: no SQL, no FTS5, no ad-hoc operator inspection. Stalwart's
  RocksDB store is exactly the surface that made debugging painful this
  week. We are not adopting it.

### Optional at-rest encryption

Out of scope for v1, but the layering supports it cleanly later:

- `hail.db` via SQLCipher.
- Blob files via per-account age key, encrypt-then-hash.

Both compose without architectural change.

## Configuration

`hail.toml`:

```toml
[mail]
backend = "gmail"           # "gmail" | "jmap"

[mail.gmail]
oauth_client_id     = "..."
oauth_client_secret = "..."
# scopes derived from features: readonly + send by default; modify added
# automatically when the operator opts in to bidirectional features.

[mail.jmap]
jmap_url        = "http://stalwart:8080"
management_url  = "http://stalwart:8080"

[mail.cache]
mode             = "bounded"
keep_days        = 90
keep_max_msgs    = 50000
keep_max_bytes   = "5 GiB"
backfill         = "incremental"
blob_root        = "/var/lib/hail/blobs"
```

Env-var overrides follow the existing `HAIL_MAIL__BACKEND` convention.

### Defaults

| Flavour      | `mail.backend` | `cache.mode` | `cache.backfill` |
|--------------|----------------|--------------|------------------|
| Gmail        | `gmail`        | `bounded`    | `incremental`    |
| Self-host    | `jmap`         | `bounded`    | `off`            |
| Pi minimum   | `gmail`        | `bounded`    | `off`            |

`bounded` everywhere is the right safe default. `full` is opt-in.

### Per-account override

`cache_policy` is keyed by `mail_accounts.id`. A user with two Gmail
accounts can run a `full` mirror on the primary and `bounded` on a noisy
secondary.

## Setup

The wizard becomes flavour-aware. `/api/setup/state` already returns
`wizard_active`; it grows a `backend` discriminator so the SPA picks the
right form.

### Gmail flavour

- One screen: "Sign in with Google".
- After OAuth callback, the wizard creates the local hail user, stores the
  encrypted refresh token, and seeds `cache_policy` with bounded defaults.
- No bootstrap token, no Stalwart admin step, no DNS questions.

### Self-host flavour

- Today's wizard, unchanged except internally it calls
  `JmapBackend::provision_principal` (the JMAP-management work we already
  landed in `e2157eb` and `7f79ca4`).
- Domain + principal creation runs through the JMAP urn:stalwart:jmap
  surface, not REST.

Resume-from-failure behaviour and idempotency are shared.

## Send dispatch

A single `/api/compose` route. Internally:

1. Identify the from-address.
2. Look up which `mail_account` owns it.
3. Dispatch to that account's `MailBackend::send`.

For Gmail flavour this is always Gmail SMTP XOAUTH2.

For JMAP flavour the default is JMAP EmailSubmission via Stalwart. The
operator can additionally connect a Gmail account to the same hail user;
sending from the Gmail address routes through Gmail's SMTP rather than
through Stalwart. This is the `feature-outbound-via-provider-smtp` work
landing in `7eb072e`, generalised so the routing decision is
"backend that owns the address".

## Capabilities surfaced to the SPA

`/api/me` (or `/api/capabilities`) exposes:

```json
{
  "backend": "gmail",
  "cache_mode": "bounded",
  "supports_initial_import": true,
  "supports_principals_admin": false,
  "supports_bulk_archive": true,
  "supports_eventsource": false,
  "label_path_separator": "/",
  "accounts": [
    { "id": 1, "email": "you@gmail.com", "backend": "gmail" }
  ]
}
```

The SPA gates Provider Accounts page, admin pages, and a few stop-import
style affordances on these.

## Key decisions

The decisions taken here, with rationale, so future agents do not relitigate:

1. **One project, two flavours, one binary.** Rejected: fork into
   `hail-cloud` / `hail-server`. Reason: 90% of the product surface is
   shared. Two trees double the bug-fix and feature surface forever; the
   trait-based split costs one crate.

2. **Cache is always-on with a `mode` knob.** Rejected: cache only in Gmail
   flavour, JMAP flavour is live. Reason: offline + local search + local
   archive are universally desirable. Operators who want live-only set
   `mode="off"`; the code path is identical at the cache layer.

3. **SQLite + filesystem blob store. Not RocksDB.** Rejected: RocksDB.
   Reason: SQL inspection, FTS5, single-file portability, mature Rust
   support, dedup via content-addressed files, and avoiding the exact
   debugging pain we just experienced with Stalwart's RocksDB layer.

4. **Backfill and cache size are independent knobs.** Rejected: a single
   "archive level" enum. Reason: real users want "live in a nicer world
   from today" (`backfill=off`, `cache=bounded`) and that combination is
   not expressible by a single dial.

5. **Last-writer-wins per-(message, change-type) with timestamps.**
   Rejected: complex CRDT, operator-pick dialog. Reason: the conflict
   surface in personal mail is small and timestamps are an honest
   tiebreaker. Trash is the only operator-authoritative exception.

6. **Outbound write queue is universal.** Rejected: synchronous backend
   writes for self-host. Reason: optimistic UX + offline mutation + retry
   semantics are useful in both flavours and the queue costs almost
   nothing.

7. **Single super-schema.** Rejected: per-flavour schema files. Reason:
   migrations are linear and a few unused columns are far cheaper than two
   schemas drifting.

8. **No deprecation window.** Rejected: ship Stalwart-only first, refactor
   later. Reason: hail is unreleased. Land the unified architecture now
   while the cost is contained to one team and zero deployed users.

9. **`MailBackend` is narrow, the cache is rich.** Rejected: a richer
   backend trait that handles caching internally. Reason: keeps backends
   small and easy to add (M365, Fastmail, IMAP later) and concentrates
   policy in one place.

10. **Renaming `provider_accounts` to `mail_accounts` is the one schema
    rename allowed.** Everything else is additive.

## Out of scope (this design)

The following are explicitly future work and have their own design when
the time comes:

- IMAP, Microsoft 365, Fastmail backends. The trait supports them; no
  implementations ship in v1 of the unified architecture.
- Push notifications via Gmail Cloud Pub/Sub. Polling is good enough at
  the personal-mail scale we target.
- Multi-tenant SaaS deployments of hail. Single-user (with multiple Gmail
  accounts attached) only.
- At-rest encryption (SQLCipher + per-account age key for blobs). The
  layering supports it; turning it on is a separate task.
- A cross-account unified inbox view. The data model supports it; the UX
  is a follow-on.
- Provider-side filters / Sieve scripts pushed into Gmail or Stalwart.
  Workflow rules stay hail-local for now.

## Related docs

- `docs/architecture.md` — the original system map; this doc supersedes
  its layering section.
- `docs/provider-import-architecture.md` — Gmail OAuth + import details;
  most of it migrates into the GmailBackend implementation.
- `docs/self-hosted-outbound-runbook.md` — DNS/SPF/DKIM for self-host
  flavour; still accurate, lives alongside this doc.
- `docs/setup-runbook.md` — operator setup. Updated to be flavour-aware.
- `docs/compose-rich-text-design.md` — composer body model; unchanged.

## Status

This is a design doc. Implementation tasks live in mu. Edges run from the
leaf primitives (blob store, MailBackend trait) up to v1-unified-ship.
