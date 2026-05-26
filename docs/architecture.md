# hail — Architecture

> A short, durable map of how hail's pieces fit together. The product
> roadmap lives elsewhere (mu task graph) and the design rationale lives
> in [`docs/design.md`](./design.md); this document is for "what is
> running where, and who talks to whom?"
>
> Update this when you change the **shape** of the system — adding a
> service, splitting a process, changing a protocol boundary. Do not
> update it for ordinary feature work.

## 1. The picture

```
   ╔══════════════════════════════════════════════════════════════╗
   ║  Browser                                                     ║
   ║  ┌────────────────────────────────────────────────────────┐  ║
   ║  hail SPA  (React + Vite, static bundle)               │  ║
   ║  │  - single-column layout, no sidebar, dropdown nav     │  ║
   ║  │  - renders pre-shaped JSON                             │  ║
   ║  │  - never speaks JMAP, never knows $hail_* keywords     │  ║
   ║  └────────────────────────────────────────────────────────┘  ║
   ╚══════════════════════════════════════════════════════════════╝
                                │
                                │  HTTPS (single origin)
                                │  • GET  /              → SPA bundle
                                │  • */   /api/*         → REST endpoints
                                │  • WS   /api/ws        → push events
                                │  Cookie: hail_session (HttpOnly+Secure)
                                ▼
   ┌──────────────────────────────────────────────────────────────┐
   │  hail-api  (Rust / Axum, one binary, one port)               │
   │                                                              │
   │  Responsibilities:                                           │
   │    • Serve the SPA bundle    (tower-http::ServeDir)          │
   │    • Task-oriented REST API  (see docs/design.md §7)         │
   │    • WebSocket multiplexer   (broadcasts API + worker events)│
   │    • Auth: cookie session, AES-GCM-encrypted JMAP token      │
   │    • Sanitise inbound HTML, strip tracking pixels            │
   │    • Build outbound MIME from structured client input        │
   │                                                              │
   │  Stateless except for the cookie session table.              │
   └──────────────────────────────────────────────────────────────┘
            │                                       │
            │ sqlx (SQLite, WAL)                    │ JMAP (HTTP + EventSource)
            │ + hail-core crypto                    │ + Bearer auth (encrypted at rest)
            ▼                                       ▼
       ┌─────────┐                          ┌──────────────────┐
       │ hail.db │◄────────sqlx (WAL)───────│  Stalwart        │
       │ SQLite  │                          │  (unmodified     │
       └─────────┘                          │   upstream)      │
            ▲                               │                  │
            │ sqlx                          │  - SMTP / IMAP   │
            │                               │  - JMAP server   │
            │                               │  - Antispam      │
            │                               │  - Auth backend  │
            │                               └──────────────────┘
            │                                       ▲
            │                                       │ JMAP (HTTP + EventSource per user)
            │                                       │ + Bearer auth
   ┌────────┴────────────────────────────────────────┴───────────┐
   │  hail-worker  (Rust / tokio, one binary, no inbound ports)  │
   │                                                             │
   │  Responsibilities:                                          │
   │    • Per-user EventSource subscription to Stalwart          │
   │    • Catch-up replay since stored jmap_state cursor         │
   │    • Screener routing  (new mail → Imbox/Screener/Trash)    │
   │    • Apply $hail_* classification keywords                  │
   │    • Scheduled jobs    (bubble-up, send-later, reconcile)   │
   │                                                             │
   │  Restartable; persists its position in jmap_state.          │
   └─────────────────────────────────────────────────────────────┘
```

## 2. Processes and binaries

| Process       | Binary          | Image         | Inbound | Outbound | Storage     |
| ------------- | --------------- | ------------- | ------- | -------- | ----------- |
| `hail-api`    | `hail-api`      | `hail:latest` | :8080   | Stalwart | `hail.db`   |
| `hail-worker` | `hail-worker`   | `hail:latest` | none    | Stalwart | `hail.db`   |
| `stalwart`    | (upstream)      | `stalwartlabs/stalwart:latest` | 25, 143, 465, 587, 993 (mail) | smarthost relay (outbound mail) | `/opt/stalwart` |

Both hail binaries are produced from the same Cargo workspace and ship
in the **same Docker/Podman image**. The container's `CMD` selects which
binary runs:

```
podman run hail:latest                            # default: hail-api
podman run hail:latest /usr/local/bin/hail-worker # worker variant
```

This is set up in `Containerfile` and used by `deploy/docker-compose.yml`.

## 3. Network shape

There is **one origin** facing the browser: `https://hail.example.com`.
`hail-api` serves everything on that origin:

- `GET /`, `/assets/*`, `/index.html` …               — static SPA bundle
- `GET /healthz`, `/readyz`, `/api/openapi.json`      — operational endpoints
- `*   /api/auth/*`, `/api/setup/*`                   — public API (no session required)
- `*   /api/**` (everything else)                     — protected API (cookie session)
- `WS  /api/ws`                                       — push events

The browser **never speaks to Stalwart directly**. There is no other web
server in the production stack. A reverse proxy (Caddy / Traefik) or a
Cloudflare Tunnel can sit in front for TLS termination — that's an
operator choice, not a hail design requirement.

`hail-worker` has **no inbound network surface**. It only connects out:
to Stalwart's JMAP endpoint (one HTTP/EventSource connection per active
user) and to the SQLite file on local disk.

## 4. State boundaries

| Concept                                        | Source of truth      | Notes                                                |
| ---------------------------------------------- | -------------------- | ---------------------------------------------------- |
| Messages, threads, mailboxes, drafts, blobs    | Stalwart             | Accessed via JMAP only.                              |
| Standard JMAP keywords (`$seen`, `$flagged`, …) | Stalwart             | hail just reads/writes them like any JMAP client.    |
| `$hail_imbox`, `$hail_feed`, `$hail_papertrail`, `$hail_setaside`, `$hail_replylater`, `$hail_screened`, `$hail_seen_together` | Stalwart (as keywords on messages/threads) | Survives client switches; portable.                  |
| Screener rules, contact notes, clips, stack ordering, bubble-up schedule, scheduled sends, user prefs, sessions, audit log, app event outbox | `hail.db` (SQLite) | Hail's sidecar. Reconciled against Stalwart on a schedule. App events are durable invalidation hints from worker to API WebSocket clients. |
| User credentials (passwords)                   | Stalwart             | hail never stores passwords. Only encrypted bearer tokens. |
| JMAP bearer tokens                             | `hail.db` (AES-GCM encrypted, key from `[secrets].server_key`) | Plain text only ever in memory.        |

The full schema is in
[`crates/hail-db/migrations/0001_baseline.sql`](../crates/hail-db/migrations/0001_baseline.sql);
the rationale is in `docs/design.md` §6.

## 5. Why this shape (the short version)

- **One Rust binary serves SPA + API + WS.** No Node runtime in
  production, no second container just to host the frontend.
- **Browser never talks to Stalwart.** JMAP details (account ids, the
  `$hail_*` keyword vocabulary, batched method-call references) stay in
  Rust. The SPA is a slim view layer.
- **hail-worker is push-driven, not polling.** Stalwart pushes change
  notifications via JMAP EventSource; the worker resumes from a stored
  cursor on restart so no mail is missed across restarts.
- **SQLite + WAL** keeps the sidecar a single file with no separate
  service to operate. Continuous backups via Litestream are documented
  in `docs/backup.md`.
- **Stalwart is unmodified upstream.** hail never patches Stalwart or
  generates Sieve scripts to embed routing into Stalwart. All product
  logic (Screener decisions, classification, the Pile, bubble-up) is
  hail-side. Operators can upgrade Stalwart independently.
- **Provider-backed modes are documented separately.** The mainline product is
  Stalwart-first, but operators without a public mail server may prefer Gmail or
  Cloudflare import-backed deployments. See `docs/provider-backed-modes.md` for
  the trade-offs and possible implementation paths.
- **Alternate clients are expected later.** The planned Node/Ink TUI is
  deliberately a client of `hail-api`, not a parallel mail client that talks to
  Stalwart. That keeps one source of truth for Screener, Pile, search, and
  rendering semantics.

## 6. Hard decisions and rejected alternatives

This section records the architectural forks where we deliberately chose one
path and rejected plausible alternatives. These are the decisions future
contributors should be slow to revisit.

### 6.1 JMAP only; no IMAP fallback inside hail

**Decision:** hail talks to Stalwart using JMAP only.

**Rejected:**

- **IMAP + SMTP submission.** Mature and widely supported, but stateful,
  chatty, harder to batch, and awkward for modern push-driven webmail.
- **Dual JMAP/IMAP support.** Sounds portable, but doubles the protocol
  surface and forces every product feature to handle two data models.

**Why:** Stalwart is JMAP-native, and hail is intentionally a Stalwart-first
product layer. JMAP's batching, state cursors, and EventSource streams match
what a reactive webmail needs. If a server cannot speak JMAP, it is outside
hail's v1 target.

### 6.2 Stalwart owns mail; hail owns product semantics

**Decision:** mail-shaped data lives in Stalwart; hail-specific product state
lives partly as JMAP keywords and partly in `hail.db`.

**Rejected:**

- **Copy all mail metadata into hail.db.** Tempting for query speed, but it
  creates a cache-invalidation project and risks showing ghost mail after a
  user edits mail in another client.
- **Store all hail state only in the sidecar.** Cleaner schema, but state is
  lost/hidden if users open another JMAP client.
- **Push everything into Stalwart Sieve scripts.** More resilient while hail
  is down, but hard to test, Stalwart-dialect-specific, and turns rule edits
  into server-side script generation.

**Why:** `$hail_*` keywords live with the messages and survive client switches;
SQLite holds only the things JMAP does not model (rules, notes, stack order,
schedules). Drift is handled by push-driven reconciliation, nightly pruning,
and defensive filtering on read.

### 6.3 Screener routing is a hail-worker job, not Sieve

**Decision:** unknown senders are moved to a dedicated `Screener` mailbox by
`hail-worker`, via JMAP operations.

**Rejected:**

- **Tag-only hold inside Inbox.** Easy, but another JMAP/IMAP client would
  still show unscreened mail in the normal inbox, leaking the abstraction.
- **Generated Sieve allow/deny scripts.** Strong mailbox separation, but brittle
  and tightly coupled to Stalwart's Sieve behavior.

**Why:** a real mailbox boundary gives clean behavior in other mail clients,
while keeping the logic in Rust keeps it testable and portable. If
`hail-worker` is down, mail queues in Inbox and is routed when the worker
catches up.

### 6.4 Rust backend, React SPA frontend

**Decision:** backend and worker are Rust; the webapp is a static React SPA.

**Rejected:**

- **Go backend.** Boring and productive, but the JMAP client ecosystem is weaker
  and gives no special leverage with Stalwart.
- **TypeScript backend.** Better type-sharing with the frontend, but the TS JMAP
  libraries lacked the EventSource/WebSocket/Sieve coverage hail needed. We
  would have had to implement the hard protocol glue ourselves.
- **Rust/WASM webapp.** Interesting, but webmail UI work is mostly interaction
  state, rich text editing, keyboard shortcuts, virtualized lists, drag/drop,
  and component ecosystem — the JS/TS ecosystem wins decisively.
- **Next.js.** Adds a Node runtime in production for features hail does not need
  (SSR, ISR, server components). Webmail is a logged-in app; SEO is irrelevant.

**Why:** Rust gives us the best JMAP/Stalwart integration (`jmap-client` from
Stalwart Labs), strong long-lived async handling, and a single small runtime
image. React gives the browser UI the right ecosystem. OpenAPI-generated TS
types give us type sharing without adding a TS server.

### 6.5 Fat Rust, slim SPA

**Decision:** business logic lives in Rust. The SPA renders pre-shaped JSON and
performs optimistic UI updates.

**Rejected:**

- **SPA computes views from raw JMAP-ish data.** Would duplicate logic in every
  future client and leak Stalwart/JMAP details into the browser.
- **Browser talks directly to Stalwart JMAP.** Would expose JMAP account ids,
  bearer tokens, `$hail_*` semantics, and force the browser to enforce product
  rules that belong on the server.

**Why:** there should be exactly one definition of "what belongs in the Imbox",
"how the Screener approves a sender", "how tracking pixels are stripped", and
"how outbound MIME is assembled". That definition lives in Rust.

### 6.6 SQLite + WAL instead of Postgres

**Decision:** hail uses SQLite with WAL mode. Litestream is the recommended
backup/replication path.

**Rejected:**

- **Postgres by default.** More concurrency, JSONB, and operational familiarity
  for SaaS teams, but a much higher self-hosting tax for 1-20 trusted users.
- **Redis / background queue service.** Another moving part. The current worker
  schedules from SQLite tables directly.

**Why:** hail's target deployment is small-group self-hosting. The real product
risk is operator friction, not write throughput. SQLite also makes backups and
local development dramatically simpler.

### 6.7 Encrypted JMAP session tokens, no separate hail passwords

**Decision:** Stalwart remains the identity provider. hail stores an encrypted
JMAP bearer token per session row and sets an opaque `hail_session` cookie.

**Rejected:**

- **Separate hail password database.** Creates a second identity system,
  password reset flow, and source of drift from Stalwart.
- **Re-auth to Stalwart on every request.** Avoids token-at-rest, but is slow and
  brittle.
- **Plaintext tokens in SQLite.** Operationally easy but unacceptable: compromise
  of `hail.db` would compromise mail.

**Why:** an encrypted token row is the right balance for a trusted-operator
self-host product. The encryption key lives outside SQLite (`[secrets].server_key`
or `HAIL_SECRETS__SERVER_KEY`); losing the key forces re-login, which is
acceptable.

### 6.8 Compose/Podman deployment; no embedded Stalwart

**Decision:** canonical deployment is Compose/Podman with three services:
`stalwart`, `hail-api`, `hail-worker`. Both hail processes use the same image.

**Rejected:**

- **One binary that embeds Stalwart.** Attractive demo, but couples hail to
  Stalwart's release cadence, complicates security updates, and creates AGPL and
  admin-surface entanglement.
- **First-class Helm/Nix/system packages from day one.** Valuable later, but a
  maintenance multiplier before the product is stable.

**Why:** Compose is the lingua franca for self-hosters, works with Docker and
Podman, and lets Stalwart remain unmodified upstream.

### 6.9 First-run wizard and config-file bootstrap both exist

**Decision:** operators can either define an admin in `hail.toml` or leave it
unset and use `/setup`.

**Rejected:**

- **Wizard only.** Friendly, but frustrating for config-management users.
- **Config only.** Pure, but a poor first-run experience.

**Why:** both camps matter in self-hosting. The rule is precise: setup state is
active only when no admin user exists **and** `[admin]` is absent from config.
The mutating wizard POST has an additional operator-only bootstrap guard:
`[setup].bootstrap_enabled = true` plus `bootstrap_token` (prefer
`HAIL_SETUP__BOOTSTRAP_TOKEN`). `/api/setup/state` deliberately stays generic;
without the token, an empty public deployment cannot be claimed.
For the config-file path, Stalwart remains the source of truth: hail does not
store or verify an admin password hash and does not seed a fake user row at
startup. The configured admin email is elevated to `is_admin=1` on its first
successful Stalwart/JMAP login, using the real `jmap_account_id` from that
session. Both paths converge on the same database state.

### 6.10 Cloudflare Tunnel support is MVP, not polish

**Decision:** v1 includes Cloudflare-oriented deployment recipes for web-only
Tunnel, Email Routing/CGNAT-friendly inbound mail, and a VPS WireGuard MX gateway
for home-hosted Stalwart.

**Rejected:**

- **Document only direct port 25.** Simpler, but excludes many residential and
  CGNAT self-hosters.
- **Build a custom Cloudflare integration service first.** Too much product
  surface for v1; docs + compose overlay are enough.

**Why:** receiving mail without opening port 25 on the home network is a
self-hosting killer feature. Cloudflare Email Routing is useful for
forwarding/import setups, but the more realistic normal-SMTP home deployment is
a DNS-only MX on a lightweight VPS that forwards over WireGuard to home Stalwart.
The recipe is part of the value proposition, not an afterthought.

### 6.11 Async shutdown must be cancellation-first

**Decision:** every long-running loop and stream consumer must select on a
`CancellationToken` or shutdown signal. `hail-api` and `hail-worker` should exit
quickly on SIGINT/SIGTERM.

**Rejected / learned the hard way:**

- **Awaiting `stream.next().await` directly.** Can hang forever when the stream
  is idle.
- **Unbounded graceful draining of HTTP keep-alives.** `axum::serve(...)
  .with_graceful_shutdown(...)` can wait indefinitely on idle keep-alive clients
  if used carelessly.
- **Lazy signal-handler installation.** `tokio::signal::unix::signal(...)`
  installs on first poll; install handlers eagerly or a very early signal may
  take the default disposition.

**Why:** the worker holds long-lived JMAP EventSource streams and the API will
hold WebSockets. Clean restarts during upgrades are mandatory. We test shutdown
with real processes and fail if SIGINT/SIGTERM does not complete quickly.

### 6.12 Worker-to-API product events use a SQLite outbox

**Decision:** worker-originated UI events are appended to `hail.db.app_events`.
`hail-api` starts a cancellation-aware polling bridge, advances to the current
max row id, then polls for newer rows and rebroadcasts them through its
process-local WebSocket bus. Events are coarse invalidation hints; v1 clients
refetch current views/threads and tolerate duplicate delivery.

**Rejected:**

- **Redis/Postgres/NATS/pub-sub.** Reliable, but adds another service to operate
  and conflicts with the single-file self-hosting target.
- **An inbound HTTP callback on `hail-api` from the worker.** Simple locally, but
  creates service-discovery/authentication questions and gives the worker an
  API dependency during state transitions.
- **SQLite triggers or file notifications.** Less portable and harder to reason
  about under container bind mounts than bounded polling.

**Why:** SQLite is already shared durable state. A small polling outbox is enough
for v1 realtime invalidation without making WebSocket correctness depend on
process co-location. The bridge intentionally does not replay historical rows on
API startup; offline tabs catch up by normal REST refetch after reconnect.

### 6.13 Provider-backed modes are future deployment variants

**Decision:** document provider-backed modes, but keep v1 Stalwart-first.

**Alternatives captured:**

- Gmail/provider-backed client/cache, where hail syncs from an existing mailbox
  and provides the HEY-style UI over local sidecar/cache state.
- Gmail/provider importer into Stalwart, where Stalwart remains the local mail
  store and hail keeps its current JMAP integration.
- Cloudflare Email Routing import bridge, where a Worker/import endpoint brings
  raw messages into hail/Stalwart without a public server.
- Hail-native mail store, where hail owns message/blob/search/thread storage.

**Why:** these modes match real no-public-server operators, but they change the
source-of-truth boundary. The safest incremental path is importer-into-Stalwart;
hail-native storage is a major architecture change. Details live in
`docs/provider-backed-modes.md`.

### 6.14 Model/work orchestration choice (project process)

**Decision:** implementation tasks live in `mu`, not in markdown checklists.
Agents work in isolated git workspaces, produce one commit per task, and the
orchestrator cherry-picks into main.

**Rejected:**

- **Long markdown implementation plan.** It drifts immediately; the DAG knows
  what is ready, blocked, and closed.
- **Multiple agents in one checkout.** Build artifacts and generated files would
  trample each other.
- **All-opus workers.** During implementation, opus repeatedly got stuck in
  exploratory/tool-call loops. `gpt-5.5:medium` completed several concrete
  implementation tasks faster and with cleaner task closure.

**Why:** the durable source of truth for execution is the mu task DAG. The repo
keeps architecture and design docs; task state, evidence, and handoff context
live in mu notes.
