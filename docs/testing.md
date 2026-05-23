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
