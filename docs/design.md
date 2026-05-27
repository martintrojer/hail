# hail — Design Document

> Status: **Design phase complete.** Source of truth for v1 implementation planning.
> Audience: contributors and self-hosters.
> Last updated: 2026-05-24.

## 1. Problem Statement

Self-hosted email is a solved problem at the protocol layer (Stalwart, Postfix+Dovecot, Mailcow) but the **client experience** for self-hosters is stuck in 2005-era webmail (Roundcube, SOGo) or requires gluing together a heavy IMAP client. Meanwhile, [hey.com](https://hey.com) has demonstrated that email *is* still innovable as a product — Screener, Imbox/Feed/Paper Trail triage, the Pile, thread-as-document, spy-pixel blocking — but it is closed, hosted, and subscription-only.

**hail** is a self-hostable email front-end that clones hey.com's distinctive product experience on top of [Stalwart](https://stalw.art) and JMAP. It targets **small-group multi-tenant** deployments (families, friend groups, small teams of 1–20 users) on a single host.

The name is a pun: **h**ey + em**ail**.

## 2. Goals and Non-Goals

### Goals

- Reproduce hey.com's signature user experience (Screener, Imbox/Feed/Paper Trail, the Pile, thread-as-document, spy-pixel blocking, contact notes, bubble-up, send-later) on self-hosted infrastructure.
- Be the **easiest** way to run hey-style email at home or on a small VPS. One `docker compose up`, ten minutes to first received mail.
- Speak JMAP exclusively to the mail server. No IMAP fallback in hail's code.
- Provide working recipes for common self-host networking situations: direct
  port 25, Cloudflare Tunnel + Cloudflare Email Routing, and a VPS/WireGuard MX
  gateway for home networks behind CGNAT or residential ISP blocks.
- Document provider-backed modes, such as Gmail-backed hail or provider import
  into Stalwart, as future alternatives for operators who do not want to run any
  public mail server.
- Stay a thin product layer on top of an unmodified upstream Stalwart. Stalwart should remain swappable in principle.

### Non-Goals

- **Not a mail server.** Stalwart does that.
- **Not a CRM, calendar, or file-storage product.** Stalwart's CalDAV/CardDAV/WebDAV exist; building UIs on top is future work, not v1.
- **Not generic multi-tenant SaaS.** No public signup, no billing, no quotas-as-a-product. Operators are assumed trusted by all users on their instance.
- **Not E2E encrypted.** Like hey.com, hail does not implement PGP/S-MIME UX. Standard TLS in transit and disk encryption are the security model.
- **Not a mobile app (v1).** Responsive web only. Mobile is enabled by the API design but not built.
- **No shared mailboxes** (HEY for Work feature). Out of scope.

## 3. Audience and Deployment Shape

**Target operator**: a technically comfortable individual hosting mail for 1–20 trusted people on a single host (NAS, home server, or small VPS). Comfortable with Docker, willing to configure DNS, may be behind CGNAT.

**Canonical deployment**: Docker Compose (or Podman Compose; identical YAML) bringing up four services:

| Service | Image | Purpose |
|---|---|---|
| `stalwart` | upstream `stalwartlabs/stalwart` | SMTP, JMAP, storage, auth, antispam |
| `hail-api` | our image | HTTP API + WebSocket + serves the SPA |
| `hail-worker` | same image, different `CMD` | JMAP push consumer + scheduler |
| `cloudflared` | upstream (optional overlay) | Cloudflare Tunnel |

Persistent state lives in one Docker volume containing:
- Stalwart's data directory
- `hail.db` (SQLite, WAL mode)
- `hail.toml` (operator config)
- Optional Litestream working directory

No separate Postgres or Redis. Single-binary `hail` (with two entry-points), single SQLite file, single Stalwart instance. The entire stack is four containers, one of which is optional.

**Provider-backed alternatives** are documented but not canonical v1. In those
modes, an existing Gmail/provider mailbox or Cloudflare Email Routing bridge
feeds hail/Stalwart through sync/import instead of public SMTP. See
[`provider-backed-modes.md`](./provider-backed-modes.md).

## 4. Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│  Browser (SPA)                                                  │
│  React + Vite + TanStack Router/Query + Tailwind + shadcn/ui    │
└───────────────────┬─────────────────────────────────────────────┘
                    │ HTTPS (REST + WebSocket)
                    │ Cookie session
                    ▼
┌─────────────────────────────────────────────────────────────────┐
│  hail-api (Rust, Axum)                                          │
│   • REST endpoints (task-oriented, not CRUD)                    │
│   • WebSocket multiplexer (push events to SPA)                  │
│   • Serves SPA static bundle (tower-http::ServeDir)             │
│   • Admin endpoints                                             │
│   • Holds short-term JMAP session per user                      │
└─────────────┬──────────────────────────────┬────────────────────┘
              │ sqlx                         │ jmap-client (HTTP)
              ▼                              ▼
        ┌──────────┐               ┌──────────────────┐
        │ SQLite   │◄──────────────│  Stalwart        │
        │ hail.db  │     sqlx      │  (JMAP server)   │
        └──────────┘               └──────────────────┘
              ▲                              ▲
              │                              │ jmap-client (EventSource + HTTP)
              │ sqlx                         │
┌─────────────┴──────────────────────────────┴────────────────────┐
│  hail-worker (Rust, tokio)                                      │
│   • Long-lived JMAP EventSource per active user                 │
│   • Screener routing (new mail → Imbox / Screener / Trash)      │
│   • Scheduled jobs: bubble-up, send-later, reconciliation       │
│   • Auto-classification of approved senders                     │
└─────────────────────────────────────────────────────────────────┘
```

### 4.1 Three deployable processes, two binaries

- **`hail-api`** — HTTP/WS server. Stateless except for the session cookie cache.
- **`hail-worker`** — push consumer + scheduler. Owns the JMAP EventSource subscriptions.
- **`stalwart`** — unmodified upstream.

`hail-api` and `hail-worker` are produced from the same Cargo workspace and ship in the same Docker image; the container's `CMD` selects which one runs.

### 4.2 Cargo workspace layout

```
hail/
├── Cargo.toml                  # workspace
├── crates/
│   ├── hail-core/              # domain types, shared between api & worker
│   ├── hail-db/                # sqlx queries, migrations, schema
│   ├── hail-jmap/              # thin wrapper over jmap-client with hail conventions
│   ├── hail-api/               # Axum app, REST + WS + SPA serving
│   └── hail-worker/            # tokio app, push + scheduler
├── webapp/                     # React SPA (npm workspace, separate)
├── docs/                       # design.md, quickstart.md, recipes
└── deploy/
    ├── docker-compose.yml
    ├── docker-compose.cloudflare.yml
    ├── hail.example.toml
    └── stalwart.example.toml
```

## 5. Key Design Decisions

Each decision is recorded with the alternatives considered and the rationale. These are the load-bearing choices; everything downstream follows from them.

### DD-1 — JMAP as the only protocol
**Decision:** hail speaks JMAP to Stalwart. No IMAP code in hail.
**Alternatives:** IMAP + SMTP submission (mature, but stateful and chatty); IMAP fallback for non-JMAP servers.
**Rationale:** JMAP's batching, push (EventSource), and state tokens are exactly what a reactive webmail needs. Stalwart is JMAP-native. Avoiding IMAP halves the protocol surface in hail's code.
**Cost:** Locks hail to JMAP-speaking servers. Acceptable given Stalwart is our target.

### DD-2 — Sidecar DB + JMAP keywords for hail-specific state
**Decision:** hail state that has a natural JMAP representation lives as `$hail_*` keywords on messages/threads (Stalwart is source of truth). State that doesn't fit (rules, notes, clips, schedules) lives in a SQLite sidecar.
**Alternatives:** Pure sidecar (Stalwart unaware of hail); generate Sieve scripts to push classification into Stalwart.
**Rationale:** Keywords survive client switches and live with the data. Sieve generation is brittle and ties us to a Stalwart dialect. Pure-sidecar drifts heavily on multi-client use.
**Drift mitigation:** Reactive (worker handles JMAP `Email/changes`), periodic (nightly reconcile), defensive (on-read filter).

### DD-3 — Screener via hail-owned `Screener` mailbox, no Sieve
**Decision:** Each user has a Stalwart mailbox named `Screener`. `hail-worker` watches Inbox for new mail; unknown-sender messages are moved to `Screener`. Approval moves them out + writes a `screener_rules` row.
**Alternatives:** Tag-only filtering in Inbox (leaks if user opens another client); Sieve-script-generated routing (brittle).
**Rationale:** Real mailbox boundary is visible to any JMAP client. Logic stays in Rust, testable. Tolerates worker downtime — mail just queues in Inbox until catch-up.

### DD-4 — Rust on the backend, `jmap-client` (Stalwart Labs) crate
**Decision:** Both `hail-api` and `hail-worker` are Rust.
**Alternatives evaluated:** Go (`go-jmap` less complete, no upside); TypeScript (`jmap-client-ts` lacks WebSocket, EventSource, and Sieve support — would require us to implement push ourselves).
**Rationale:** `jmap-client` is by Stalwart's author, covers RFC 8620 + 8621 + 8887 (WebSocket) + Sieve drafts. Same-vendor alignment matters for an evolving protocol. Tokio handles long-lived push connections cleanly. Single static binary deploy.

### DD-5 — React SPA, no SSR, no Next.js
**Decision:** Vite + React 19 SPA. Built as static files. Served by `hail-api` over `tower-http::ServeDir`.
**Alternatives:** Next.js (adds a Node runtime in production for zero benefit — webmail has no SEO/SSR needs); SvelteKit (smaller ecosystem for the components webmail needs: rich text editor, virtualized lists, complex drag-and-drop).
**Rationale:** Webmail is a logged-in app; SSR earns nothing. Removing Node from the production deployment is a meaningful self-host win. React's ecosystem (Tiptap, TanStack, shadcn/ui) directly maps to what hail needs.

### DD-6 — "Fat Rust, Slim SPA" — task-oriented API
**Decision:** All business logic lives in Rust. The SPA is a view layer with optimistic interactions. The API is **task-oriented** (verbs the user does), not REST-CRUD over JMAP objects.
**Rationale:** Single source of truth for "what is the Imbox?" etc.; JMAP details never leak to the client; security boundary; worker runs without the SPA; future second clients (mobile, CLI) inherit all behavior.
**Consequence:** SPA never sees JMAP IDs, never builds a result-reference batch, never knows the `$hail_*` keyword vocabulary.

### DD-7 — SQLite + WAL + Litestream
**Decision:** SQLite via `sqlx` (with `sqlite` feature). WAL mode. Optional Litestream container for continuous replication to S3/R2/local.
**Alternatives:** Postgres (real ops cost for self-hosters; gains we don't need at our scale).
**Rationale:** At 1–20 users on one host, SQLite is genuinely faster end-to-end and removes an entire service. Code uses `sqlx` so a Postgres swap is mechanical if anyone ever needs it.

### DD-8 — Encrypted JMAP token in session row, not re-auth per request
**Decision:** Login produces a JMAP token from Stalwart. Token is encrypted with a server key (env var / keyfile) and stored in `sessions(jmap_token_enc)`. Cookie holds an opaque session id (HttpOnly, Secure, SameSite=Lax). 30-day sliding TTL.
**Rationale:** Acceptable for the trusted-operator threat model. CSRF covered by SameSite + custom request header.

### DD-9 — Compose canonical; first-run wizard AND config-file admin both supported
**Decision:** `docker-compose.yml` is the canonical deployment. First-run UX caters to both camps: a `/setup` wizard appears iff (a) no admin user exists in `hail.db` AND (b) `hail.toml` has no `[admin]` block. Config-file purists set `[admin]` and never see the wizard; both paths converge on the same DB state.
**Alternatives rejected:** Single binary embedding Stalwart (AGPL entanglement, couples upgrade cycles, two admin UIs problem); Helm + Nix + Compose all first-class (maintenance burden).

## 6. Data Model

### 6.1 State that lives in Stalwart (via JMAP)

The source of truth for anything email-shaped. hail never duplicates this in its sidecar.

- All messages, threads, mailboxes, drafts, identities, submissions, blobs.
- Standard JMAP keywords: `$seen`, `$flagged`, `$draft`, `$answered`.
- hail-defined keywords (encoded on messages/threads via JMAP `Email/set`):

| Keyword | Meaning | Mutually exclusive with |
|---|---|---|
| `$hail_imbox` | Classified to Imbox | `$hail_feed`, `$hail_papertrail` |
| `$hail_feed` | Classified to Feed | `$hail_imbox`, `$hail_papertrail` |
| `$hail_papertrail` | Classified to Paper Trail | `$hail_imbox`, `$hail_feed` |
| `$hail_screened` | Sender decision has been made (any direction) | — |
| `$hail_setaside` | Thread is on the Set Aside pile | — |
| `$hail_replylater` | Thread is on the Reply Later pile | — |
| `$hail_seen_together` | Thread read as one document (replaces per-message `$seen` semantics) | — |

A dedicated `Screener` mailbox per user holds pending-sender mail (see DD-3).

### 6.2 State that lives in `hail.db` (SQLite sidecar)

**Migration tracking.** Schema migrations are managed by [`sqlx::migrate!()`](https://docs.rs/sqlx/latest/sqlx/macro.migrate.html), which auto-creates and maintains its own `_sqlx_migrations` table on first run. Migration files live in `crates/hail-db/migrations/NNNN_description.sql` and are embedded in the `hail-api` binary at compile time. On startup, `hail-api` runs any pending migrations inside a transaction before accepting requests; `hail-worker` waits for the schema version it was compiled against. The schema below is the v1 baseline; subsequent changes are additive migration files, never edits to baseline.

The `_sqlx_migrations` table sqlx maintains looks like:

```sql
-- Managed by sqlx — DO NOT create or modify manually.
CREATE TABLE _sqlx_migrations (
  version        BIGINT PRIMARY KEY,           -- numeric prefix of the migration filename
  description    TEXT NOT NULL,
  installed_on   TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  success        BOOLEAN NOT NULL,
  checksum       BLOB NOT NULL,                -- SHA-384 of the migration SQL
  execution_time BIGINT NOT NULL               -- nanoseconds
);
```

Application-owned tables follow.

```sql
-- Users mapped 1:1 to Stalwart accounts.
CREATE TABLE users (
  id              INTEGER PRIMARY KEY,
  email           TEXT NOT NULL UNIQUE,
  jmap_account_id TEXT NOT NULL,
  display_name    TEXT,
  is_admin        INTEGER NOT NULL DEFAULT 0,
  created_at      TEXT NOT NULL
);

-- Encrypted JMAP token; one row per active login session.
CREATE TABLE sessions (
  id              TEXT PRIMARY KEY,          -- opaque cookie value (256-bit)
  user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  jmap_token_enc  BLOB NOT NULL,             -- AES-GCM, server key
  user_agent      TEXT,
  expires_at      TEXT NOT NULL,
  created_at      TEXT NOT NULL,
  last_used_at    TEXT NOT NULL
);
CREATE INDEX idx_sessions_user ON sessions(user_id);

-- Screener decisions, one row per (user, sender).
CREATE TABLE screener_rules (
  user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  sender_address  TEXT NOT NULL,             -- normalized lowercase
  decision        TEXT NOT NULL CHECK (decision IN ('allow','deny','pending')),
  classify_as     TEXT     CHECK (classify_as IN ('imbox','feed','papertrail')),
  decided_at      TEXT,
  first_seen_at   TEXT NOT NULL,
  PRIMARY KEY (user_id, sender_address)
);

-- Per-contact private notes (markdown).
CREATE TABLE contact_notes (
  user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  address         TEXT NOT NULL,             -- normalized lowercase
  markdown        TEXT NOT NULL,
  updated_at      TEXT NOT NULL,
  PRIMARY KEY (user_id, address)
);

-- Stack ordering for Reply Later and Set Aside.
CREATE TABLE stack_positions (
  user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  stack           TEXT NOT NULL CHECK (stack IN ('reply_later','set_aside')),
  thread_id       TEXT NOT NULL,             -- JMAP thread id
  position        INTEGER NOT NULL,
  added_at        TEXT NOT NULL,
  PRIMARY KEY (user_id, stack, thread_id)
);
CREATE INDEX idx_stack_order ON stack_positions(user_id, stack, position);

-- Scheduled "bubble up" — re-mark a thread unread at surface_at.
CREATE TABLE bubble_ups (
  id              INTEGER PRIMARY KEY,
  user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  thread_id       TEXT NOT NULL,
  surface_at      TEXT NOT NULL,
  fired_at        TEXT,
  created_at      TEXT NOT NULL
);
CREATE INDEX idx_bubble_ups_pending ON bubble_ups(surface_at) WHERE fired_at IS NULL;

-- Scheduled outbound mail.
CREATE TABLE scheduled_sends (
  id              INTEGER PRIMARY KEY,
  user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  draft_email_id  TEXT NOT NULL,             -- JMAP email id of the draft
  send_at         TEXT NOT NULL,
  status          TEXT NOT NULL CHECK (status IN ('pending','sent','cancelled','failed')),
  sent_at         TEXT,
  error           TEXT,
  created_at      TEXT NOT NULL
);
CREATE INDEX idx_scheduled_sends_due ON scheduled_sends(send_at) WHERE status = 'pending';

-- Per-user preferences blob (signature, default classifications, theme, etc.)
CREATE TABLE user_prefs (
  user_id         INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  prefs_json      TEXT NOT NULL DEFAULT '{}'
);

-- Worker resume marker — JMAP state cursor per (user, type).
CREATE TABLE jmap_state (
  user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  type_state      TEXT NOT NULL,             -- 'Email','Thread','Mailbox','EmailSubmission'
  state           TEXT NOT NULL,             -- opaque JMAP state token
  updated_at      TEXT NOT NULL,
  PRIMARY KEY (user_id, type_state)
);

-- Audit log (admin actions, screener decisions, sends). Append-only.
CREATE TABLE audit_log (
  id              INTEGER PRIMARY KEY,
  user_id         INTEGER REFERENCES users(id) ON DELETE SET NULL,
  action          TEXT NOT NULL,
  payload_json    TEXT,
  created_at      TEXT NOT NULL
);
```

### 6.3 v1.1 additions (sketch, not v1)

```sql
-- Yellow inline notes on specific emails (Tier 2 #12).
CREATE TABLE email_notes (
  user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  email_id        TEXT NOT NULL,
  markdown        TEXT NOT NULL,
  updated_at      TEXT NOT NULL,
  PRIMARY KEY (user_id, email_id)
);

-- Highlighted text snippets (Tier 2 #13).
CREATE TABLE clips (
  id              INTEGER PRIMARY KEY,
  user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  source_thread_id TEXT NOT NULL,
  source_email_id TEXT NOT NULL,
  text            TEXT NOT NULL,
  context_before  TEXT,
  context_after   TEXT,
  created_at      TEXT NOT NULL
);
CREATE INDEX idx_clips_thread ON clips(user_id, source_thread_id);
```

## 7. API Shape

`hail-api` exposes a **task-oriented** HTTP+JSON API at `/api/`, plus a WebSocket at `/api/ws`. Endpoints map to user intents, not CRUD over JMAP objects.

All endpoints are authenticated by the session cookie. Mutations require an `X-Hail-Request: 1` header (CSRF defense).

### 7.1 Views (read)

```
GET  /api/views/imbox?cursor=<opaque>&limit=50
GET  /api/views/feed?cursor=<opaque>&limit=50
GET  /api/views/papertrail?cursor=<opaque>&limit=50
GET  /api/views/screener
GET  /api/views/set-aside
GET  /api/views/reply-later
GET  /api/views/bubble-up              # scheduled bubble-up threads
GET  /api/views/search?q=<text>&scope=mail|clips|notes|all
GET  /api/threads/:thread_id              # assembled thread-as-document
GET  /api/contacts/:address               # contact + notes + thread history
GET  /api/contacts/:address/files         # attachments for a contact (v1.1)
GET  /api/files                           # global files view (v1.1)
GET  /api/workflows                       # mail rules list (v2)
GET  /api/workflows/:id                   # single workflow detail (v2)
```

Each view returns **pre-shaped JSON tailored to its UI** — the Feed endpoint returns inline-rendered HTML excerpts; the Paper Trail endpoint returns merchant-grouped compact rows; the Imbox endpoint returns thread cards with sender avatars and unread badges. The SPA renders, doesn't compute.

### 7.2 Verbs (mutate)

```
POST /api/screener/decisions
        { sender, decision: "approve"|"deny",
          classify_as?: "imbox"|"feed"|"papertrail",
          apply_to_history: bool }

POST /api/threads/:id/classify     { to: "imbox"|"feed"|"papertrail" }
POST /api/threads/:id/set-aside
POST /api/threads/:id/reply-later
POST /api/threads/:id/bubble-up    { at: ISO8601 }
POST /api/threads/:id/clip         { text, context_before?, context_after?, source_email_id }   # v1.1
POST /api/threads/:id/note         { markdown, email_id? }     # inline thread notes
POST /api/threads/:id/archive
POST /api/threads/:id/trash
POST /api/threads/:id/mark         { read: bool }

PUT  /api/contacts/:address/note   { markdown }
DELETE /api/contacts/:address/note

POST /api/compose
        { to, cc?, bcc?, subject, body_markdown,
          attachments?: [blob_id], send_at?: ISO8601,
          in_reply_to?: thread_id }
POST /api/threads/:id/reply        { body_markdown, attachments?, send_at? }
POST /api/drafts                   { ... }                       # auto-save
PATCH /api/drafts/:id              { ... }
DELETE /api/scheduled-sends/:id                                  # cancel send-later

POST /api/blobs                    multipart upload → blob_id
```

### 7.3 Auth and admin

```
POST /api/auth/login               { email, password }
POST /api/auth/logout
GET  /api/auth/me

# Admin (requires is_admin=1)
GET    /api/admin/users
POST   /api/admin/users            { email, password, display_name }
DELETE /api/admin/users/:id
POST   /api/admin/users/:id/reset-password
GET    /api/admin/domains
POST   /api/admin/domains          { domain }
DELETE /api/admin/domains/:domain

# First-run wizard (only active when no admin user exists AND hail.toml has no [admin])
GET  /api/setup/state
POST /api/setup/admin              { email, password, display_name, domain }
```

Admin endpoints proxy to Stalwart's management API where appropriate (we do not reimplement user/domain provisioning).

### 7.4 WebSocket events

Single connection per logged-in tab at `/api/ws`. Server pushes app-level events translated from JMAP push:

```
{ "type": "imbox.new",         "thread_id": "...", "preview": {...} }
{ "type": "feed.new",          "thread_id": "...", "preview": {...} }
{ "type": "screener.pending",  "sender": "...", "preview": {...} }
{ "type": "thread.updated",    "thread_id": "...", "changes": [...] }
{ "type": "thread.removed",    "thread_id": "..." }
{ "type": "bubble.fired",      "thread_id": "..." }
{ "type": "send.completed",    "scheduled_send_id": 42 }
{ "type": "send.failed",       "scheduled_send_id": 42, "error": "..." }
```

The SPA uses these to invalidate / patch its TanStack Query cache. No polling.

### 7.5 Health and operations

```
GET /healthz                       # liveness
GET /readyz                        # readiness: SQLite open, JMAP session OK
GET /metrics                       # Prometheus (opt-in via env)
```

### 7.6 Type sharing

`hail-api` emits an OpenAPI 3.1 schema (via `utoipa`). The webapp generates TS types with `openapi-typescript`. One source of truth, no hand-maintenance.

## 8. The Worker

`hail-worker` is the long-lived process that makes hail feel real-time.

### 8.1 Responsibilities

1. **JMAP push subscriptions.** One EventSource (via `jmap-client::Client::event_source`) per active user, subscribed to `Email`, `EmailDelivery`, `Mailbox`, `EmailSubmission`. Resubscribes on disconnect with exponential backoff.
2. **Inbound routing.** On `Email/changes` for a user's Inbox:
   - Fetch envelope of new messages.
   - Lookup sender in `screener_rules`.
   - `allow` → set `$hail_<classification>` keyword per rule, fan-out `imbox.new` / `feed.new` / `papertrail.new` WS events.
   - `deny` → move to Trash.
   - No rule → move to `Screener` mailbox, insert `screener_rules` row as `pending`, fire `screener.pending` WS event.
3. **Sidecar reconciliation.** Handles destroyed/moved JMAP objects from change feeds; prunes `stack_positions`, `bubble_ups`, etc.
4. **Scheduler.** Polls `bubble_ups` and `scheduled_sends` tables on a fixed tick (60s default).
   - Due bubble-ups: clear `$seen` on the thread, fire `bubble.fired` WS event.
   - Due scheduled sends: call JMAP `EmailSubmission/set`, update status, fire `send.completed` / `send.failed`.
5. **Catch-up on restart.** On startup, replay JMAP `Email/changes` since the stored `jmap_state` cursor per user, before reopening push subscriptions. No missed mail across restarts.
6. **Nightly reconciliation.** Per user, verify referenced JMAP objects still exist via batched `Thread/get`; prune orphans.

### 8.2 Concurrency model

- One tokio task per active user holds the EventSource stream.
- Shared `Arc<AppState>` holds the SQLite pool and a per-user JMAP `Client` cache.
- Scheduled jobs run in a single coordination task that dispatches to a bounded worker pool.
- All writes through `sqlx` with explicit transactions; SQLite WAL mode handles the api+worker concurrent-writer case.

### 8.3 Failure modes

| Failure | Behavior |
|---|---|
| Worker crash | systemd / Docker restart. Catch-up via `jmap_state` cursors. |
| Stalwart down | Exponential backoff on reconnect. Mail queues at SMTP layer — no data loss. |
| SQLite corruption | Litestream replay from last replicated state. |
| JMAP token revoked | Worker logs, marks session expired, drops EventSource until user re-auths. |
| Bubble-up fires while user offline | Stored as `fired_at` set; WS event delivered on next connect via state diff. |

## 9. The Webapp (SPA)

### 9.1 Stack

| Layer | Choice |
|---|---|
| Build | Vite |
| Framework | React 19 |
| Routing | TanStack Router (type-safe) |
| Data | TanStack Query + WebSocket-driven cache invalidation |
| Styling | Tailwind CSS + shadcn/ui (Radix primitives under the hood) |
| Rich text | Tiptap (ProseMirror) |
| Drag & drop | dnd-kit |
| Virtualized lists | TanStack Virtual |
| Types | Generated from OpenAPI via `openapi-typescript` |
| State | TanStack Query + small Zustand store for UI-only state (selected thread, pile open/closed, undo toast) |

### 9.2 Top-level routes

```
/login
/setup                  # first-run wizard (only when active)
/                       # → /imbox
/imbox
/feed
/papertrail
/screener
/set-aside
/reply-later
/bubble-up              # scheduled bubble-up threads list
/thread/:id
/contacts/:address
/search?q=...
/compose
/compose/reply/:thread_id
/files                  # v1.1 — all attachments browser
/clips                  # v1.1
/workflows              # v2 — mail rules / automated routing
/workflows/:id          # v2
/settings
/admin                  # is_admin only
```

### 9.3 Layout primitives

- **Single-column shell:** top strip (logo/wordmark, section title, icon cluster + avatar) + center column (720px max-width, `margin: 0 auto`). No persistent sidebar. Navigation lives in a dropdown menu anchored on the logo. See `docs/ui-direction.md` §2.
- **The Pile:** persistent floating bottom-right widget visible from Imbox and other list views. Shows counts for Set Aside + Reply Later items. Clicking expands to show thread previews with quick-remove actions. See `docs/ui-direction.md` §11.
- **Composer:** full-page in-canvas writing surface (same center column). Minimal field styling, quiet Send Later secondary action. Auto-saves to `/api/drafts` every 5s of inactivity. See `docs/ui-direction.md` §7.
- **Undo toast:** dark bottom-center viewport-anchored bar for destructive mutation undo. Auto-dismisses after ~5s. At most one at a time. See `docs/ui-direction.md` §11b.
- **Screener notification banner:** warm in-column banner at the top of Imbox when senders are pending. See `docs/ui-direction.md` §8b.
- **Per-message action popup:** floating card triggered from each message in thread view. Offers Reply, Forward, Set Aside, Reply Later, Bubble Up, Move To, Add Note, Spam, Trash. See `docs/ui-direction.md` §6.
- **Screener routing dropdown:** after approving a sender, a popup lets the user choose Imbox/Feed/Paper Trail destination. See `docs/ui-direction.md` §8b.

### 9.4 Keyboard shortcuts (Tier 1 minimum)

Mirror hey.com's vocabulary where it makes sense.

```
j / k          next / previous thread
e              archive (Imbox & Feed)
#              trash
y              set aside
l              reply later
r              reply
c              compose
/              focus search
g i            go to Imbox
g f            go to Feed
g p            go to Paper Trail
g s            go to Screener
?              shortcut help overlay
```

### 9.5 What the SPA must NOT do

- Compute the contents of Imbox / Feed / Paper Trail.
- Know any `$hail_*` keyword names.
- Talk JMAP directly.
- Strip tracking pixels or sanitize incoming HTML (server already did it).
- Build RFC 5322 messages for outbound (server does it from `body_markdown` + structured fields).

## 10. Security Model

Threat model: trusted operator, semi-trusted users, untrusted senders, hostile network.

### 10.1 Boundaries

| Concern | Mechanism |
|---|---|
| Auth | Cookie session, opaque id, 256-bit, HttpOnly + Secure + SameSite=Lax |
| CSRF | Same-origin cookie + required `X-Hail-Request` header on mutations |
| JMAP token at rest | AES-GCM with server key from env/file; never in DB plaintext |
| Incoming HTML XSS | Server-side sanitization (`ammonia`) before rendering pane is built |
| Tracking pixels | Stripped by server during thread-as-document assembly. Badge counts removed images. |
| Outbound integrity | Server is sole producer of RFC 5322 wire format; client can never inject raw headers |
| Admin endpoints | `is_admin` check + audit log row per call |
| Rate limiting | Tower middleware on `/api/auth/*` and `/api/compose` |
| Logging hygiene | No tokens, passwords, full message bodies in logs. Email addresses redacted at INFO+. |

### 10.2 Explicit non-coverages

- No PGP/S-MIME.
- No per-message ACLs (we don't have shared mailboxes).
- No 2FA in v1 (Stalwart's auth may offer it; we don't add our own factor on top).
- No sandbox iframe for HTML mail in v1 — sanitization is our defense. v1.1 may add iframe sandboxing for defense in depth.

## 11. Cloudflare Tunnel Recipes (v1)

Both included in v1 as documentation + Compose overlays. Located in `deploy/docker-compose.cloudflare.yml` and `docs/cloudflare-tunnel.md`.

### 11.1 Recipe A — Web UI only via Tunnel

Use case: operator can receive mail on port 25 directly (VPS, business ISP) but does not want to expose HTTPS/JMAP/IMAP ports to the public internet.

Flow:
- MX record points to operator's host; port 25 reachable.
- `cloudflared` exposes `mail.example.com` (the SPA + JMAP + IMAP+TLS port if used by external clients).
- DNS for `mail.example.com` is a CF Tunnel CNAME.

### 11.2 Recipe B — Inbound mail via Cloudflare Email Routing + Tunnel

Use case: residential ISP, CGNAT, or any environment where port 25 is unavailable
and the operator accepts a forwarding/import bridge instead of a normal SMTP
session into Stalwart.

Flow:
- MX record points to `route1.mx.cloudflare.net` etc. (Cloudflare Email Routing).
- Cloudflare Email Routing forwards every received message to a destination address — we use a *custom HTTP webhook* or a *relay address* on a hail-operated SMTP listener exposed via the tunnel on a non-25 port.
- `cloudflared` exposes Stalwart's submission port over the tunnel.
- Outbound: Stalwart sends via a smarthost (e.g. Cloudflare's relay, or a paid relay like Mailgun/Postmark on the free tier) since direct port 25 outbound is also blocked.

The doc covers DNS records (SPF, DKIM, DMARC) for both recipes, including the "Cloudflare signs DKIM on your behalf" gotcha when using Email Routing.

### 11.3 Recipe C — VPS MX gateway plus WireGuard home tunnel

Use case: home-hosted mailbox storage with residential port blocks, CGNAT, or a
home IP that should not appear in public DNS. This is now the preferred advanced
home-hosting recipe when the operator wants Stalwart to receive a real SMTP
transaction instead of importing forwarded mail.

Flow:
- MX record points to `mx.example.com`, a DNS-only A record on a lightweight VPS.
- The VPS accepts public SMTP on port 25 and forwards to home Stalwart over
  WireGuard using HAProxy PROXY protocol or an MTA relay. Blind NAT is documented
  as possible but discouraged because it can hide the real sender IP from
  Stalwart.
- `cloudflared` exposes only `hail-api` for the web UI at `mail.example.com`.
- Outbound still uses a smarthost; the VPS gateway is not assumed to be a
  reputable direct sender.

The Cloudflare docs and smoke runbook cover this recipe alongside Email Routing.

## 12. Out of Scope / Open Questions

Decisions deferred to implementation or post-v1:

- **Search backend.** v1 uses JMAP `Email/query` with text filters (Stalwart has full-text). If this is insufficient for unified search (mail + clips + notes), we add Tantivy in v1.1. Open.
- **Push notifications to mobile.** Out of scope until a mobile app exists.
- **Attachment storage limits.** Stalwart's quotas handle it; hail does not impose a separate limit. Operator decides.
- **i18n.** English only in v1. The SPA is structured for it (no hard-coded strings in components), but no translations shipped.
- **Theming.** Dark mode in v1.1. Custom theming v2.
- **Workflows / Mail Rules.** Automated routing rules with conditions and actions (HEY calls these "Workflows"). UI for list, detail, and create/edit. Tracked as `ui-workflows-rules` (DEFERRED). See `docs/ui-direction.md` §17b.
- **All Files view.** Cross-mail attachments browser. Tracked as `ui-all-files-view` (DEFERRED). See `docs/ui-direction.md` §17c.
- **Screener Speakeasy.** Monthly rotating password/passphrase that lets an incoming message bypass the Screener when the current phrase is included in that message. It is not an allowed-senders list, not a private bypass address, and not route management. See `docs/speakeasy-design.md`.
- **Migrating from existing IMAP servers.** Stalwart has an IMAP import tool; we document it but don't wrap it. Open whether to surface in admin UI in v2.
- **Federation / multi-host scaling.** Not a goal. If anyone wants this, fork.

## 13. Roadmap

### v1 — "Yes, this is hey" (MVP)

Tier 1 (all): Screener (with routing dropdown for Imbox/Feed/Paper Trail destination), Imbox/Feed/Paper Trail, the Pile (Reply Later + Set Aside as floating bottom-right widget), thread-as-document, spy pixel blocking, strict JMAP threading, per-message action popup, inline notes.
Tier 2 sticky four: contact notes, bubble-up (with time picker submenu), send-later, unified search.
Plumbing: multi-user auth via Stalwart, minimal admin UI, both Cloudflare Tunnel recipes, first-run wizard + config-file admin path, Litestream backup option.
UI: HEY-inspired warm paper aesthetic (single-column, no sidebar, dropdown menu navigation). See `docs/ui-direction.md`.

### v1.1 — "Power user complete"

Email notes (yellow annotations), Clips + Clips library, Focus & Reply mode, Files view (per-contact, per-thread, global attachments browser), aliases / send-as identities, Screener Speakeasy (monthly bypass password/passphrase), keyboard power-through-Imbox triage mode, Trash view (list/restore/auto-purge after N days), Spam view + mark-as-spam + Stalwart antispam integration.

### v2 — Polish + self-host bonus

Per-identity delivery windows, user-defined auto-classification rules (Workflows — mail rules with conditions/actions, list/detail/edit UI), merge contacts, first-reply auto-promote Feed→Imbox, drag-to-classify in Screener, daily Feed digest, Sieve rules editor, vacation responder UI, multiple identities UI, backup/restore + data export, dark mode, LLM-assisted spam classification (optional, local ollama or API provider).

### v2.1 / later — Alternate clients

- **Node/Ink terminal UI.** A first-class TUI client that talks only to `hail-api` (never directly to Stalwart/JMAP), proving the "fat Rust API, slim clients" architecture. Roadmap tasks in mu: `tui-architecture-spike`, `tui-node-ink-scaffold`, `tui-thread-reader`, `tui-screener`, `tui-offline-cache-spike`. These are tracked but deferred until the web MVP lands.

### Out of scope (Tier 4)

Shared mailboxes, calendar UI, full CRM, Imbox sub-labels, E2E encryption.

## 14. Implementation Tracking

This project uses [`mu`](https://github.com/earendil-works/mu) for task tracking and multi-agent orchestration.
There is no in-repo plan markdown; the task graph is the plan.

```bash
mu state -w hail                 # current state
mu task list -w hail             # all tasks
mu task next -w hail             # what's ready to claim
mu task tree -w hail             # dependency view
```

All task notes, decisions, and evidence live in mu. Use `mu task notes <id>` to read a task's history.

---

*End of design document.*
