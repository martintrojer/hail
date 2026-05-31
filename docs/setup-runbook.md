# First-run setup runbook

Use this for the shipped Compose stack when no hail admin exists yet.

## 1. Prepare environment

From `deploy/`, copy and edit the env file:

```bash
cp .env.example .env
tbx openssl rand -hex 32   # HAIL_SERVER_KEY
tbx openssl rand -hex 32   # HAIL_SETUP_BOOTSTRAP_TOKEN
```

Set `HAIL_SERVER_KEY`, `HAIL_SETUP_BOOTSTRAP_TOKEN`, and `HAIL_PUBLIC_URL` in
`.env`.

## 2. Start the stack

Local smoke:

```bash
podman compose -f deploy/docker-compose.local.yml up -d --build
```

Canonical compose:

```bash
cd deploy
podman compose up -d --build
```

The compose files pin Stalwart to `docker.io/stalwartlabs/stalwart:v0.16` and
run a one-shot `stalwart-init` sidecar after Stalwart becomes healthy. The
sidecar reads `STALWART_RECOVERY_ADMIN` from the compose environment, applies
hail-friendly Stalwart settings idempotently through JMAP, verifies them, and
then exits before `hail-api` and `hail-worker` start. The automatic settings are:

- JMAP upload window: `maxUploadCount = 100000000`,
  `uploadQuota = 1099511627776` bytes, `maxUploadSize = 104857600` bytes, and
  `maxConcurrentUploads = 16`.
- HTTP rate limits: authenticated and anonymous requests both at
  `1000000` requests per `60000` ms.
- Local smoke only: `deploy/docker-compose.local.yml` sets `HAIL_LOCAL_SINK=1`
  on `stalwart-init`, which changes Stalwart's MTA outbound strategy to route
  every queued recipient through the built-in `local` route instead of MX
  delivery. This keeps compose-send smoke tests inside the local Stalwart
  mailbox store, so messages sent from a local user to the same local user can
  be verified without a Gmail round-trip or SPF/DKIM for `hail.test`.

`deploy/docker-compose.yml` **must not** set `HAIL_LOCAL_SINK=1`. Production
uses real SMTP/MX delivery (or a deliberately configured relay) and must not
install the local-only loopback routing rule.

This replaces the old manual "open Stalwart admin and adjust quota/rate-limit
fields" step. The operator still finishes hail's setup wizard and then connects
Gmail/provider accounts as needed.

## 3. Open the wizard

Open hail in a browser:

- local compose: <http://127.0.0.1:8080>
- production compose: your `HAIL_PUBLIC_URL`

Fill in:

- **Bootstrap token**: `HAIL_SETUP_BOOTSTRAP_TOKEN` from `.env`.
- **Stalwart admin user**: `admin` by default.
- **Stalwart admin password**: `admin1234` by default for local compose because
  `STALWART_RECOVERY_ADMIN=admin:admin1234` is set; production compose reads
  your `STALWART_RECOVERY_ADMIN` value from `deploy/.env`.
- **Admin email**: the mailbox to create, for example `you@example.com`.
- **Display name**: optional.
- **Mail domain**: the domain part of the admin email, for example
  `example.com`.
- **Password**: mailbox password, at least 12 characters.

On submit, hail authenticates to Stalwart v0.16, creates or reuses the domain
and mailbox through Stalwart's `urn:stalwart:jmap` management calls, then logs
in through JMAP and creates the hail admin session.

## 4. If setup fails

The wizard should show Stalwart problem details in the **Setup failed** alert.
Check hail-api logs for the management URL, HTTP status, and redacted response
body. Passwords, bearer tokens, and auth codes should not appear in logs.

If you intentionally pre-create Stalwart domains/users by CLI or WebAdmin, unset
`HAIL_STALWART__MANAGEMENT_URL`; the wizard will skip management provisioning
and only verify the mailbox with JMAP login.
