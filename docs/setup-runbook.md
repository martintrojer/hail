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
set `HAIL_STALWART__MANAGEMENT_URL=http://stalwart:8080` so hail can provision
Stalwart through the management REST API.

## 3. Open the wizard

Open hail in a browser:

- local compose: <http://127.0.0.1:8080>
- production compose: your `HAIL_PUBLIC_URL`

Fill in:

- **Bootstrap token**: `HAIL_SETUP_BOOTSTRAP_TOKEN` from `.env`.
- **Stalwart admin user**: `admin` by default.
- **Stalwart admin password**: `admin1234` by default for local compose because
  `STALWART_RECOVERY_ADMIN=admin:admin1234` is set.
- **Admin email**: the mailbox to create, for example `you@example.com`.
- **Display name**: optional.
- **Mail domain**: the domain part of the admin email, for example
  `example.com`.
- **Password**: mailbox password, at least 12 characters.

On submit, hail authenticates to Stalwart v0.16, creates the domain principal
and mailbox principal with a bearer token, then logs in through JMAP and creates
the hail admin session.

## 4. If setup fails

The wizard should show Stalwart problem details in the **Setup failed** alert.
Check hail-api logs for the management URL, HTTP status, and redacted response
body. Passwords, bearer tokens, and auth codes should not appear in logs.

If you intentionally pre-create Stalwart domains/users by CLI or WebAdmin, unset
`HAIL_STALWART__MANAGEMENT_URL`; the wizard will skip management provisioning
and only verify the mailbox with JMAP login.
