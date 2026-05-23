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
