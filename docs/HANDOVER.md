# Orchestrator handover — hail

You are picking up a pi orchestrator role on the `hail` repo. Read this first,
then `AGENTS.md`.

## Where things stand

- Stalwart-flavour hail is running locally on image `cd7a2898e3c8` against
  Stalwart `v0.16`. Setup wizard provisions domain+user via JMAP
  `urn:stalwart:jmap` (no manual Stalwart-admin step). `~17` commits landed
  this past session covering: noreply-as-system-sender fix, INBOX-only Gmail
  import, papertrail sectioned, screener row optimistic removal, thread j/k
  + reply-from-active-message, composer close-after-send, thread grouping in
  lists, stop-import button, Power Through feed-mode, feed images toggle,
  local outbound sink, quota-stuck classification, Gmail XOAUTH2 SMTP send,
  JMAP-everywhere management, bidirectional Gmail sync, per-message
  attachments.
- 311 webapp tests + full Rust workspace tests pass. Lint + build clean.
- Main is far ahead of origin. Push when convenient.

## What changed strategically

Hail is being rearchitected into **one project, two flavours, shared core**:

- Gmail flavour (connect Google account, no MTA, no DNS).
- Self-host flavour (Stalwart underneath, today's deployment).

Read `docs/hail-architecture.md` end to end. It captures:

- `MailBackend` trait seam (GmailBackend, JmapBackend).
- Universal SQLite+filesystem cache layer between hail-api and backends.
- Cache modes (`off` / `bounded` / `full`) and backfill (`off` /
  `incremental`) as orthogonal knobs.
- Last-writer-wins sync, outbound write queue, blob CAS, FTS5 search.
- 10 numbered decisions with rationale (no RocksDB, no fork, no deprecation
  window, etc).

Treat that doc as the spec. Do not relitigate the decisions; if you disagree,
add a note and bring it up explicitly with the operator.

## Where the plan lives

In `mu`, workstream `hail`. The unified-rewrite DAG is rooted at
`v1-unified-ship`. ~26 tasks, all edges wired.

Start here:

```bash
mu state -w hail
mu task next -w hail
mu task tree v1-unified-ship -w hail
```

Five foundation tasks are immediately dispatchable in parallel (different
crates, no overlap):

- `primitive-mailbackend-trait`
- `primitive-blob-store`
- `primitive-schema-cache-tables`
- `primitive-rename-provider-accounts`
- `primitive-config-flavour-cache`

Once those land, `cache-crate-skeleton` fans out to the cache implementation
tasks. Backends and api-routes-on-cache follow. Reviews + human smokes gate
`v1-unified-ship`.

## Workflow reminders

- Workers spawn with `pi --model meta-openai/gpt-5.5:medium`. Higher
  thinking models repeatedly burn budget on exploration loops here.
- Cherry-pick worker commits onto main; do not merge worker branches.
- Workers sometimes report DONE without committing. Fetch their workspace
  branch directly and cherry-pick:
  `git fetch /var/home/martintrojer/.local/state/mu/workspaces/hail/<worker> HEAD`
- The `openapi.example.json` fixture must stay in sync with the Rust
  openapi. After non-trivial server changes, regenerate by running a local
  hail-api binary on `127.0.0.1:18081` and curl'ing `/api/openapi.json`,
  then `cd webapp && npm run api:types`.
- Dispatch in small waves of disjoint files to avoid cherry-pick conflicts.
- Human-smoke tasks are operator-only. Never delegate them to workers.
- For Stalwart v0.16 specifics: `stalwart-init` sidecar applies rate-limit
  and upload-quota patches automatically. Recovery admin = `admin /
  admin1234`. Bootstrap token lives in `deploy/.env`.

## Live state

```
HAIL_SETUP_BOOTSTRAP_TOKEN=2ce2235d142218babdf1d4408ecbf990860799cb8953278ec931961a9bde8849
Stalwart admin (recovery): admin / admin1234
Stalwart WebAdmin URL:     http://127.0.0.1:18080/admin/
hail URL:                  http://127.0.0.1:8080
Stalwart pinned:           docker.io/stalwartlabs/stalwart:v0.16
Current deployed image:    cd7a2898e3c8 (commit 189f08c)
```

## Open backlog ready to dispatch RIGHT NOW (post-unification)

These are NOT blocked by the rewrite — they are improvements that should
land in current Stalwart-hail too. Decide whether to keep building on the
current tree or freeze it and focus on the unified rewrite.

Ready set today (`mu task next -w hail`):

- `primitive-config-flavour-cache`
- `primitive-rename-provider-accounts`
- `primitive-mailbackend-trait`
- `primitive-schema-cache-tables`
- `primitive-blob-store`

## First three things to do

1. Read `docs/hail-architecture.md` fully.
2. Read `mu task tree v1-unified-ship -w hail` to understand the DAG.
3. Dispatch the five foundation tasks in parallel. They have zero file
   overlap and should land cleanly. Use the existing
   `mu agent spawn worker-N --workspace --workspace-backend git -w hail
   --command 'pi --model meta-openai/gpt-5.5:medium'` pattern.

Good luck.
