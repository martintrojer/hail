# Testing hail

This document covers local test helpers that intentionally sit below the normal
Compose deployment. Production deployment remains documented in
`docs/quickstart.md` and `deploy/docker-compose.yml`.

## Stalwart integration fixture

`crates/hail-test` exposes a Stalwart fixture for local integration and smoke
and smoke tests:

```rust
use hail_test::stalwart::start_stalwart_fixture;

let fixture = start_stalwart_fixture().await?;
let jmap_url = fixture.jmap_url();
```

The fixture automates a full disposable mailbox:

- allocates free loopback host ports;
- creates temporary `/etc/stalwart` and `/var/lib/stalwart` bind mounts;
- starts `docker.io/stalwartlabs/stalwart:latest` with Podman in v0.16
  bootstrap mode using a pinned `STALWART_RECOVERY_ADMIN`;
- completes the `x:Bootstrap` JMAP setup object with `mail.hail.test` /
  `hail.test`, internal directory, manual DNS/TLS/DKIM, and a loopback
  `STALWART_PUBLIC_URL`;
- restarts Stalwart into normal mode and waits for authenticated JMAP;
- provisions the default user `alice@hail.test` with password
  `hail-test-password` through `x:Account/set`;
- proves login with `hail_jmap`; and
- removes the container on drop.

`create_domain()`, `create_user()`, `seed_user_domain()`, and
`seed_default_user()` are idempotent helpers over Stalwart's v0.16 JMAP
management objects. The fixture uses Basic authentication for the seeded user's
JMAP session because Stalwart v0.16 does not accept hail's legacy
base64-as-bearer convention for normal password login.

## Running the gated Stalwart tests

By default, Stalwart container tests are skipped so `cargo test --workspace`
works without Podman, network image pulls, or privileged ports.

Run the gated tests explicitly:

```bash
HAIL_RUN_STALWART_TESTS=1 RUSTFLAGS="-D warnings" cargo test -p hail-test --test stalwart_fixture -- --nocapture
```

Optional image override:

```bash
HAIL_RUN_STALWART_TESTS=1 \
HAIL_STALWART_IMAGE=docker.io/stalwartlabs/stalwart:latest \
RUSTFLAGS="-D warnings" cargo test -p hail-test --test stalwart_fixture -- --nocapture
```

## Equivalent manual Podman command

The fixture uses Stalwart v0.16's bootstrap flow rather than mounting the older
TOML config shape. For manual debugging, start the same image with writable
config/data mounts and pinned recovery credentials:

```bash
root=$(mktemp -d)
mkdir -p "$root/etc" "$root/data"
podman unshare chown -R 2000:2000 "$root/etc" "$root/data"

podman run --rm --name hail-stalwart-manual \
  -e STALWART_RECOVERY_ADMIN='admin:hail-bootstrap-admin-password' \
  -e STALWART_PUBLIC_URL='http://localhost:18080' \
  --publish 127.0.0.1:18080:8080 \
  --publish 127.0.0.1:10025:25 \
  --volume "$root/etc:/etc/stalwart:Z" \
  --volume "$root/data:/var/lib/stalwart:Z" \
  docker.io/stalwartlabs/stalwart:latest
```

The Rust fixture then performs the equivalent of:

1. `x:Bootstrap/get` / `x:Bootstrap/set` against `http://localhost:18080/jmap/`;
2. container restart;
3. `x:Account/set` for `alice` in the bootstrapped `hail.test` domain.

In another shell, readiness should be visible as a non-5xx response:

```bash
curl -i http://127.0.0.1:18080/.well-known/jmap
```

Clean up if not using `--rm`:

```bash
podman rm --force hail-stalwart-manual
rm -rf "$root"
```

## Local mail testbed

`scripts/local-mail-testbed.sh` is the repeatable local/direct mail smoke
harness. It targets Stalwart + hail on loopback only; it does not require
Cloudflare, public DNS, or privileged host port 25.

Dry-run the harness without containers:

```bash
scripts/local-mail-testbed.sh --dry-run
```

The dry-run validates the fixture import plan and prints the expected checks. A
real run:

1. builds `hail:local`;
2. starts `scripts/local-mail-testbed.compose.yml` with the first available
   compose provider (`podman compose`, `podman-compose`, then `docker compose`)
   and a minimal loopback Stalwart config from
   `scripts/local-stalwart-testbed.toml`;
3. waits for Stalwart JMAP and hail API readiness;
4. imports these synthetic inbound messages with JMAP `Email/import`:
   - `personal-simple.eml`;
   - `newsletter-tracking-pixel.eml`;
   - `receipt-papertrail.eml`.

```bash
scripts/local-mail-testbed.sh
```

Useful URLs/checks once the stack is running:

```bash
curl -fsS http://127.0.0.1:18080/.well-known/jmap
curl -fsS http://127.0.0.1:18081/readyz
# Browser: http://127.0.0.1:18081
```

### Provisioning status

The Rust Stalwart fixture now automates disposable v0.16 provisioning for
integration tests. The Compose script still targets the standalone Compose stack
and imports into whatever mailbox is reachable at `HAIL_TESTBED_JMAP_URL`.
Until that script is switched to reuse the Rust fixture/bootstrap helper or a
Compose-native v0.16 apply step, set `HAIL_TESTBED_PASSWORD` when importing into
an already-provisioned stack.

Override defaults when needed:

```bash
HAIL_TESTBED_EMAIL='alice@hail.test' \
HAIL_TESTBED_PASSWORD='<password>' \
HAIL_TESTBED_JMAP_URL='http://127.0.0.1:18080' \
HAIL_TESTBED_HAIL_URL='http://127.0.0.1:18081' \
  scripts/local-mail-testbed.sh --no-build
```

The corresponding gated Rust tests exercise fixture loading by default, real
JMAP import only when explicitly enabled, and the end-to-end local/direct mail
smoke through `hail-api`:

```bash
cargo test -p hail-test --test local_mail_testbed
cargo test -p hail-test --test e2e_local_direct_mail_smoke

HAIL_RUN_LOCAL_MAIL_TESTBED=1 \
HAIL_TESTBED_PASSWORD='<password>' \
  cargo test -p hail-test --test local_mail_testbed -- --nocapture
```

## Local/direct mail E2E smoke

`scripts/e2e-local-direct-mail-smoke.sh` is the preferred automated local smoke
for the core receive flow. It is env-gated because it starts a disposable
Stalwart container and local `hail-api` / `hail-worker` processes:

```bash
# Default cargo test path: explicitly skips with the actionable reason.
cargo test -p hail-test --test e2e_local_direct_mail_smoke -- --nocapture

# Real smoke: starts Stalwart, injects mail via JMAP Email/import, asserts via hail API.
HAIL_RUN_LOCAL_MAIL_TESTBED=1 scripts/e2e-local-direct-mail-smoke.sh
```

The smoke does not fake success. When enabled it:

1. starts the Rust `start_stalwart_fixture()` local mail testbed and provisions
   `alice@hail.test` / `hail-test-password`;
2. starts `hail-api` and `hail-worker` against a temporary SQLite sidecar with
   `HAIL_TICK_SECS=1`;
3. injects a unique synthetic inbound message from
   `maya.e2e-local-direct-mail-smoke@personal.example` through JMAP
   `Email/import`;
4. logs in to `hail-api`, waits for `/api/views/screener` to show the pending
   sender, and approves the sender for Imbox;
5. injects a second synthetic message from the approved sender; and
6. asserts through hail API that `/api/views/imbox` contains the message and
   `GET /api/threads/{thread_id}` renders the thread.

If the enabled run fails before assertions because Podman or Stalwart bootstrap
is unavailable, fix the reported host/tooling issue and rerun the exact command
above. If you need to debug the older compose harness manually, run:

```bash
scripts/local-mail-testbed.sh --dry-run
HAIL_TESTBED_PASSWORD='<password>' scripts/local-mail-testbed.sh --no-build
```
