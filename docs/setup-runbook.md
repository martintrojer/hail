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

Choose a Compose flavour:

- **Gmail flavour**: no Stalwart, no `stalwart-init`, no public DNS/MX setup.
  Set `HAIL_MAIL__GMAIL__OAUTH_CLIENT_ID` and
  `HAIL_MAIL__GMAIL__OAUTH_CLIENT_SECRET` in `.env`.
- **Self-host flavour**: adds Stalwart and `stalwart-init`. Set
  `STALWART_RECOVERY_ADMIN` in `.env`.

## 2. Start the stack

Local smoke (unchanged, Stalwart-backed development stack):

```bash
podman compose -f deploy/docker-compose.local.yml up -d --build
```

Production/shared Compose uses `deploy/docker-compose.yml` as the base and a
single flavour overlay selected with `-f` flags.

Gmail flavour:

```bash
cd deploy
podman compose -f docker-compose.yml -f docker-compose.gmail.yml up -d --build
# or: docker compose -f docker-compose.yml -f docker-compose.gmail.yml up -d --build
```

Self-host flavour:

```bash
cd deploy
podman compose -f docker-compose.yml -f docker-compose.selfhost.yml up -d --build
# or: docker compose -f docker-compose.yml -f docker-compose.selfhost.yml up -d --build
```

The shared base starts only `hail-api` and `hail-worker`, with `hail-data` for
`hail.db` and `hail-blobs` for the content-addressed blob store. The Gmail
overlay sets `HAIL_MAIL__BACKEND=gmail` plus Gmail OAuth environment and does
not start Stalwart or expose SMTP/IMAP ports. The self-host overlay adds
Stalwart, the one-shot `stalwart-init` sidecar, and sets
`HAIL_MAIL__BACKEND=jmap`.

The self-host overlay pins Stalwart to `docker.io/stalwartlabs/stalwart:v0.16`
and runs `stalwart-init` after Stalwart becomes healthy. The sidecar reads
`STALWART_RECOVERY_ADMIN` from the compose environment, applies hail-friendly
Stalwart settings idempotently through JMAP, verifies them, and then exits
before `hail-api` and `hail-worker` start. The automatic settings are:

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

`deploy/docker-compose.selfhost.yml` **must not** set `HAIL_LOCAL_SINK=1`.
Production self-host uses real SMTP/MX delivery (or a deliberately configured
relay) and must not install the local-only loopback routing rule.

This replaces the old manual "open Stalwart admin and adjust quota/rate-limit
fields" step. The operator still finishes hail's setup wizard and then connects
Gmail/provider accounts as needed.

### Provider-mode outbound

If the user connects Gmail and composes from that Gmail address, hail sends via
Gmail SMTP (`smtp.gmail.com:465`, implicit TLS, XOAUTH2) using the stored OAuth
refresh token. This bypasses Stalwart's outbound MTA for that identity, so real
recipients can accept mail even when the self-hosted Stalwart domain has no
public SPF/DKIM/DMARC setup. Gmail SMTP automatically places the sent message in
Gmail Sent; hail records a safe `sent_via_provider` audit row and keeps tokens
and message bodies out of logs.

Accounts connected before the `gmail.send` scope was added must reconnect from
Provider Accounts before outbound provider sending is enabled. Import remains
read-only and continues to work while the account is in `needs_reauth`.

## 3. Open the wizard

Open hail in a browser:

- local compose: <http://127.0.0.1:8080>
- Gmail flavour: your `HAIL_PUBLIC_URL`
- self-host flavour: your `HAIL_PUBLIC_URL`

For Gmail flavour, choose **Sign in with Google** and complete OAuth. The wizard
creates the local hail user and seeds the default bounded/incremental cache
policy; there are no Stalwart admin credentials or DNS questions.

For self-host flavour, fill in:

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
