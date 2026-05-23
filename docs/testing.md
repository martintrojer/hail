# Testing hail

This document covers local test helpers that intentionally sit below the normal
Compose deployment. Production deployment remains documented in
`docs/quickstart.md` and `deploy/docker-compose.yml`.

## Stalwart integration fixture

`crates/hail-test` exposes a Stalwart fixture scaffold for local integration
and smoke tests:

```rust
use hail_test::stalwart::start_stalwart_fixture;

let fixture = start_stalwart_fixture().await?;
let jmap_url = fixture.jmap_url();
```

The fixture currently does the parts we can automate reliably:

- allocates free loopback host ports;
- creates a temporary config/data directory;
- renders a minimal Stalwart TOML matching `deploy/stalwart.example.toml`'s
  SQLite/filesystem/internal-directory shape;
- starts `docker.io/stalwartlabs/stalwart:latest` with Podman;
- waits for `/.well-known/jmap` to respond;
- removes the container on drop.

The fixture intentionally does **not** pretend that mailbox provisioning works.
`seed_user_domain()` and `login_seeded_user()` currently return
`UserProvisioningNotImplemented`. Stalwart v0.13+ moved more state into the
WebUI/JMAP-managed configuration surface, and hail still needs to pin the exact
management API/auth flow before automated domain/user creation is safe. Future
work should replace those placeholders with real calls and then obtain a
`hail_jmap::Session` for the seeded user.

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

The fixture writes a config equivalent to this run shape. For manual debugging,
create a temporary root and copy/edit `deploy/stalwart.example.toml` into it:

```bash
root=$(mktemp -d)
mkdir -p "$root/etc" "$root/data"
cp deploy/stalwart.example.toml "$root/etc/config.toml"

podman run --rm --name hail-stalwart-manual \
  --publish 127.0.0.1:18080:8080 \
  --publish 127.0.0.1:10025:25 \
  --volume "$root/etc/config.toml:/opt/stalwart/etc/config.toml:ro" \
  --volume "$root/data:/var/lib/stalwart:Z" \
  docker.io/stalwartlabs/stalwart:latest
```

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
3. waits for Stalwart JMAP and hail API health;
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
curl -fsS http://127.0.0.1:18081/api/health
# Browser: http://127.0.0.1:18081
```

### Current provisioning blocker

Automatic Stalwart domain/user provisioning is still intentionally blocked on
pinning Stalwart's management API/auth bootstrap flow. The script starts the
stack and then fails clearly before import if `HAIL_TESTBED_PASSWORD` is absent.
It does **not** claim successful end-to-end mail injection until a real mailbox
exists.

Manual path for now:

1. start the testbed:
   ```bash
   scripts/local-mail-testbed.sh
   ```
2. create domain `hail.test` and mailbox `alice@hail.test` in Stalwart's
   WebUI/admin surface (or with the Stalwart CLI for the pinned image);
3. rerun import against the existing stack:
   ```bash
   HAIL_TESTBED_PASSWORD='<password>' scripts/local-mail-testbed.sh --no-build
   ```

Override defaults when needed:

```bash
HAIL_TESTBED_EMAIL='alice@hail.test' \
HAIL_TESTBED_PASSWORD='<password>' \
HAIL_TESTBED_JMAP_URL='http://127.0.0.1:18080' \
HAIL_TESTBED_HAIL_URL='http://127.0.0.1:18081' \
  scripts/local-mail-testbed.sh --no-build
```

The corresponding gated Rust test exercises fixture loading by default and real
JMAP import only when explicitly enabled:

```bash
cargo test -p hail-test --test local_mail_testbed

HAIL_RUN_LOCAL_MAIL_TESTBED=1 \
HAIL_TESTBED_PASSWORD='<password>' \
  cargo test -p hail-test --test local_mail_testbed -- --nocapture
```

## Provisioning TODO

The next harness step is to replace the explicit placeholder with real
Stalwart automation:

1. confirm the management endpoint and authentication mechanism for the pinned
   Stalwart image;
2. create the test domain (`hail.test` by default);
3. create the test user (`alice@hail.test` by default) with a password;
4. log in via `hail_jmap::login_bearer` using hail's base64 `email:password`
   bearer convention;
5. optionally create the `Screener` mailbox via JMAP so local mail-flow smokes
   can assert receive/screener/imbox behavior.

Until those steps land, tests must treat user provisioning as unsupported and
must not report false success.
