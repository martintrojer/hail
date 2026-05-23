# AGENTS.md — hail agent guide

This repo is built by long-lived pi agents coordinated with **mu**. Read this before doing work.

## Project shape

hail is a self-hosted hey.com-style webmail UI on top of Stalwart + JMAP.

Runtime architecture:

- `hail-api` — Rust/Axum server. Serves the React SPA, REST API, and future WebSocket on one port.
- `hail-worker` — Rust/tokio background worker. JMAP EventSource subscriptions, screener routing, schedulers.
- `stalwart` — unmodified upstream mail server.
- `hail.db` — SQLite sidecar for hail-only state.
- `webapp/` — React/Vite SPA. Browser talks **directly to `hail-api`**. There is no TS backend and no Node runtime in production.

Durable docs:

- `docs/architecture.md` — system map, hard decisions, rejected alternatives. Update when system shape changes.
- `docs/design.md` — original v1 design and roadmap. Update only when product/design assumptions change.
- Task tracking is **not** in markdown. It lives in mu.

## Use mu for work tracking

This project uses `mu` as the live task DAG. Do not create `TODO.md`, `PLAN.md`, or markdown checklists for implementation tasks.

**Critical rule: keep refining the DAG as you learn.** The initial task graph is not sacred and is not complete. When implementation reveals a missing prerequisite, split-out subtask, review gate, integration test, or follow-up, add it to mu immediately and wire edges. This is how we harness mu: the graph absorbs discovery so no agent has to keep the plan in memory.

Examples:

```bash
# A large task reveals a reusable primitive that should land first.
mu task add html-sanitize-trackers -w hail \
  -t "Mail render primitive: sanitize inbound HTML and strip/count tracking pixels" \
  -i 90 -e 1 \
  -b auth-middleware,jmap-wrapper
mu task block thread-assembly -w hail --by html-sanitize-trackers

# A feature needs an E2E gate before v1 can ship.
mu task add e2e-receive-screener-imbox -w hail \
  -t "E2E smoke: receive/import mail, Screener pending, approve sender, Imbox visible" \
  -i 95 -e 1 \
  -b stalwart-integration-harness,screener-routing,views-screener,verbs-screener
mu task block v1-ship -w hail --by e2e-receive-screener-imbox

# A task was actually completed by a broader task.
mu task close auth-middleware -w hail \
  --evidence "superseded by auth-login; middleware implemented and tested there"
```

When adding tasks:

- Give each task a concrete title and honest `impact` / `effort-days`.
- Prefer smaller tasks with clear ownership over one vague umbrella task.
- Add `--blocked-by` or `mu task block` edges immediately; do not leave dependency knowledge in prose.
- If a task is discovered during implementation but should not block v1, add it and mark it `DEFERRED` with evidence.
- If a task is no longer needed, `reject` or `close` it with evidence rather than deleting history.
- Add final gates (E2E, security review, test review, docs sync) as tasks that block `v1-ship`; do not rely on the orchestrator remembering them.

Common commands:

```bash
mu state -w hail
mu task next -w hail
mu task list -w hail
mu task notes <task> -w hail
mu workspace list -w hail
```

Agent workflow:

1. Read `mu state -w hail` before acting.
2. Claim before work:
   ```bash
   mu task claim <task> -w hail --for <agent> --evidence "starting"
   ```
3. Work in the agent's mu workspace, not the main checkout.
4. Commit one logical task per commit.
5. Append a task note before closing:
   ```text
   FILES:    paths changed/inspected
   COMMANDS: commands run + exit codes
   DECISION: choices made and why
   VERIFIED: tests/checks that passed
   ODDITIES: weird things / followups
   ```
6. Close with evidence:
   ```bash
   mu task close <task> -w hail --evidence "cargo test ... exit 0"
   ```

Orchestrator workflow:

- Dispatch small waves of disjoint tasks.
- Wait with:
  ```bash
  mu task wait <tasks...> -w hail --any --first --json --on-stall exit
  ```
- Cherry-pick worker commits onto main; do not merge worker branches.
- Verify after each cherry-pick.
- Refresh/recreate workspaces between waves.

## Model choice

Default implementation model: `meta-openai/gpt-5.5:medium`.

Why: in this repo it has been faster, cheaper, and more reliable than opus for concrete implementation tasks. Opus repeatedly got stuck in exploratory/tool-call loops.

Use higher thinking only when a task proves genuinely subtle. Do not default to opus.

Spawn example:

```bash
mu agent spawn worker-1 --workspace --workspace-backend git -w hail \
  --command 'pi --model meta-openai/gpt-5.5:medium'
```

Always send `/new` before assigning unrelated follow-up work to an existing pane:

```bash
mu agent send worker-1 -w hail '/new'
```

## Code review and test review gates

This repo has two review skills available to agents:

- `code-reviewer` — review production code for dead code, duplication,
  unnecessary complexity, and non-idiomatic patterns.
- `test-reviewer` — review tests for false confidence, excessive mocking,
  weak assertions, and missing behavior coverage.

Use them as explicit mu tasks. Do not treat review as an informal chat message.

When to add review tasks:

- After a feature cluster lands (for example: auth/session, worker/eventsource,
  screener routing, composer/send-later, SPA shell/views).
- After any large refactor.
- Before `v1-ship`.
- Whenever tests pass but the implementation relied heavily on mocks or fake
  infrastructure.

Review task naming convention:

```text
review-code-<area>
review-tests-<area>
```

Examples:

```bash
mu task add review-code-auth -w hail \
  -t "Code review: auth/session/setup API for simplicity, dead code, idiomatic Axum/sqlx" \
  -i 80 -e 0.5 \
  -b auth-login,setup-wizard-api

mu task add review-tests-auth -w hail \
  -t "Test review: auth/session/setup tests for false confidence and missing security cases" \
  -i 85 -e 0.5 \
  -b auth-login,setup-wizard-api

mu task block mvp-security-review -w hail --by review-code-auth
mu task block mvp-test-review -w hail --by review-tests-auth
```

How review findings become work:

1. The reviewer writes findings in the task note, grouped as Critical /
   Recommended / Suggestions.
2. The reviewer creates mu tasks for every actionable finding before closing the
   review. Do not leave a long review note for the orchestrator to manually
   split later. Use concrete task names, honest impact/effort, and blocker
   edges.
3. The reviewer wires obvious dependencies immediately:
   - Critical correctness/security findings should usually block
     `mvp-security-review`, `mvp-test-review`, and often `v1-ship`.
   - Test-confidence findings should block `mvp-test-review`.
   - Product-smoke findings should block the relevant `human-smoke-*` task.
4. The reviewer marks low-priority follow-ups `DEFERRED` when they should be
   tracked but not worked for v1.
5. The reviewer adds a final triage note summarizing:
   - tasks created;
   - tasks deferred;
   - findings intentionally rejected/no-tasked and why.
6. The orchestrator still has final authority to re-prioritize, reopen, defer,
   reject, or unblock tasks. But the reviewer should do the first split so the
   orchestrator is not forced to mine a giant note for TODOs.

Example review finalization:

```bash
mu task add fix-thread-render-remote-image-privacy -w hail \
  -t "Thread render: block/proxy external remote images by default" \
  -i 95 -e 1 \
  -b html-sanitize-trackers,thread-assembly
mu task block v1-ship -w hail --by fix-thread-render-remote-image-privacy
mu task block mvp-security-review -w hail --by fix-thread-render-remote-image-privacy

mu task add refactor-mail-render-quote-stripping-dom -w hail \
  -t "Mail render: replace quote-strip string parser with DOM traversal" \
  -i 60 -e 1 \
  -b quoted-reply-stripper
mu task defer refactor-mail-render-quote-stripping-dom -w hail \
  --evidence "tracked from review; not v1 blocking unless quote heuristics expand"

mu task note review-code-thread-rendering -w hail \
  "TRIAGE: created fix-thread-render-remote-image-privacy (v1 blocker); deferred refactor-mail-render-quote-stripping-dom; rejected no findings."
mu task close review-code-thread-rendering -w hail \
  --evidence "review complete; findings split into mu tasks"
```

Important: findings must not live only in review prose. If a finding needs
   action, it becomes a mu task. If it does not need action, the triage reason is
   recorded in the review task notes.

Human-in-the-loop smoke tests follow the same rule. When the operator is asked
to try the app, their notes are not treated as chat-only feedback. Add a
`human-smoke-*` task at the point where the feature is demoable, and add a
follow-up `triage-human-smoke-notes` task that turns every operator note into
mu tasks or explicit rejects/deferrals. Human smoke gates should block
`v1-ship` when they validate core MVP behavior.

Prefer concrete testbeds over vague manual testing. For mail features, maintain
both:

- a local/direct testbed that runs Stalwart + hail and injects synthetic emails
  without Cloudflare or public DNS;
- a Cloudflare-assisted testbed/runbook for Tunnel + Email Routing using the
  operator's real CF account/domain when needed.

Synthetic mail fixtures (newsletters, receipts, personal threads, attachments,
tracking pixels, quoted replies) should be reusable across unit tests, local
E2E, Cloudflare smoke, and human smoke. If a smoke test needs external
credentials or DNS changes, model it as a human-in-the-loop mu task and make the
operator steps explicit.

## Fedora host / toolbox tools

This repo is developed on an immutable Fedora-style host. Do **not** assume every DNF-installed CLI exists directly on the host.

Use `tbx` to run toolbox-installed commands:

```bash
tbx openssl version
tbx openssl rand -hex 32
```

`tbx <cmd> ...` runs `<cmd>` inside the dev toolbox. Prefer this for tools that are normally installed via DNF but missing on the host, such as:

- `openssl`
- `podman-compose` / `docker-compose` if installed in toolbox
- `actionlint`
- `markdownlint`
- other distro packages

If a required tool is missing both on the host and via `tbx`, **ask the operator to install it** rather than inventing a fragile workaround. Say exactly what you need, for example:

```text
Please install actionlint in the dev toolbox so I can validate .github/workflows/ci.yml:
  tbx sudo dnf install actionlint
```

For cryptographic keys in docs/tests, prefer:

```bash
tbx openssl rand -hex 32
```

Fallbacks like Python `secrets.token_hex(32)` are acceptable for one-off local tests, but docs should use `openssl rand -hex 32` unless there is a deliberate reason not to.

## Verification commands

Root Rust workspace:

```bash
RUSTFLAGS="-D warnings" cargo build --workspace
RUSTFLAGS="-D warnings" cargo test --workspace
cargo audit
```

Webapp:

```bash
cd webapp
npm install
npm run api:types
npm run build
npm run lint
npm audit --audit-level=moderate
```

Container:

```bash
podman build -t hail:test .
podman images hail:test --format '{{.Size}}'
```

Compose validation may not be installed on the host. Try, in order:

```bash
podman compose config
podman-compose config
docker compose config
```

If none are available, use YAML parsing as a basic fallback and ask the operator to install a compose provider before claiming full compose validation.

## Coding rules

### Rust

- Keep `RUSTFLAGS="-D warnings"` clean.
- Prefer small modules with testable traits around external services.
- Any long-running async loop must be cancellation-aware:
  ```rust
  tokio::select! {
      _ = cancel.cancelled() => break,
      item = stream.next() => { ... }
  }
  ```
- Never await a long-lived stream/socket/sleep without a cancellation branch.
- SIGINT/SIGTERM shutdown must be tested with a real process where practical.
- Do not log secrets, tokens, passwords, message bodies, or server keys.

### Webapp

- Browser talks only to `hail-api`.
- Do not talk to Stalwart/JMAP directly from the SPA.
- Use generated OpenAPI types from `webapp/src/api/types.ts`.
- Mutating API calls must send `X-Hail-Request: 1` and `credentials: 'include'`.
- No Node runtime assumptions in production; Node is build-time only.

### Database / migrations

- Migrations live in `crates/hail-db/migrations/`.
- sqlx manages `_sqlx_migrations`; do not hand-roll another migrations table.
- SQLite WAL mode is expected.
- Sidecar DB should not duplicate mail metadata unless explicitly designed as a cache.

## Tool-call gotchas

When using the `write` tool, provide both `path` and `content`. Do not call it with only `path`.

When using `edit`, keep replacements small and exact.

When using mu, use `mu agent send`; do not send raw tmux keys directly.

## Current project conventions

- One task = one commit where practical.
- Commit messages are concise imperative summaries.
- Evidence lives in mu task notes.
- Architecture decisions live in `docs/architecture.md`.
- The main checkout is for orchestration and verification; workers edit in mu workspaces.
