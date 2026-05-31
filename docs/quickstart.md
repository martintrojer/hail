# Quickstart: direct/simple Compose deployment

This guide takes a fresh host to one received message in hail using the
**direct/simple Stalwart deployment path**. Replace `example.com` and
`mail.example.com` with your real domain and host name.

If you have not chosen a deployment shape yet, start with
[deployment.md](./deployment.md). This quickstart assumes mail can reach your
Stalwart host directly on TCP/25, or that you only want to boot the stack before
following a deeper ingress guide.

For other shapes:

- CGNAT or blocked TCP/25 with mail data at home: see the
  [VPS/WireGuard MX option](./deployment.md#3-home-server-with-vpswireguard-mx-gateway).
- Cloudflare Email Routing/import bridge: see
  [cloudflare-tunnel.md](./cloudflare-tunnel.md#recipe-b-cloudflare-email-routing-plus-tunnel).
- Gmail/provider import: see
  [provider-import-architecture.md](./provider-import-architecture.md).

## 1. Prerequisites

You need:

- A domain you control, with DNS access.
- A host that can run Podman/Docker Compose. For this direct quickstart, public
  TCP `25` should reach Stalwart and public HTTPS should reach `hail-api` or a
  reverse proxy in front of it.
- Podman with `podman compose`, or Docker with the Compose plugin.
- `openssl`.

Check tools:

```bash
openssl version
podman --version && podman compose version
# or: docker --version && docker compose version
```

If port 25 is blocked or you are behind CGNAT, pause here and choose a different
ingress shape in [deployment.md](./deployment.md). You can still use the rest of
this guide to boot the local stack, but public mail will not arrive until the
chosen ingress path is configured.

## 2. Clone and enter the repo

```bash
git clone https://github.com/earendil-works/hail.git
cd hail
```

If you already have the repo, `cd` into that checkout.

## 3. Copy and edit configs

Create local deployment files:

```bash
cp deploy/hail.example.toml deploy/hail.toml
cp deploy/stalwart.example.toml deploy/stalwart.toml
cp deploy/.env.example .env 2>/dev/null || touch .env
```

Edit hail config:

```bash
$EDITOR deploy/hail.toml
```

Use the Compose service URL for Stalwart, enable Stalwart management for
first-run provisioning, and set your public URL:

```toml
database_url = "sqlite:///var/lib/hail/hail.db"

[stalwart]
jmap_url = "http://stalwart:8080"
management_url = "http://stalwart:8080"

[server]
bind = "0.0.0.0:8080"
public_url = "https://mail.example.com"
```

Leave `[admin]` commented out to use the first-run wizard at `/setup`. The
wizard POST is still protected by an operator bootstrap token, configured below.

Edit Stalwart config:

```bash
$EDITOR deploy/stalwart.toml
```

Set the public mail host:

```toml
[server]
hostname = "mail.example.com"
```

Find remaining placeholders:

```bash
grep -n 'example.com\|CHANGE_ME' deploy/stalwart.toml deploy/hail.toml
```

Create `.env`:

```bash
cat > .env <<'EOF'
HAIL_DOMAIN=example.com
HAIL_HOSTNAME=mail.example.com
HAIL_SERVER_KEY=
HAIL_SETUP_BOOTSTRAP_TOKEN=
EOF
```

Keep any extra variables required by your `deploy/docker-compose.yml`. Do not
commit `.env`.

## 4. Generate secrets

hail encrypts JMAP tokens in `hail.db`. Generate a stable 32-byte server key:

```bash
openssl rand -hex 32
```

Generate a separate temporary setup bootstrap token:

```bash
openssl rand -hex 32
```

Paste both values into `.env`:

```dotenv
HAIL_SERVER_KEY=REPLACE_WITH_THE_64_HEX_CHARS_FROM_OPENSSL
HAIL_SETUP_BOOTSTRAP_TOKEN=REPLACE_WITH_A_DIFFERENT_64_HEX_TOKEN
```

Back up `HAIL_SERVER_KEY`; restored sessions cannot be decrypted without it.
Keep `HAIL_SETUP_BOOTSTRAP_TOKEN` private until setup is complete. It is only
needed to authorize the first admin creation form.

## 5. Optional: enable hail.db backups

Litestream backup for the SQLite sidecar is an optional Compose overlay and is
not required for local development:

```bash
cp deploy/litestream.example.yml deploy/litestream.yml
podman compose -f deploy/docker-compose.yml -f deploy/docker-compose.litestream.yml up -d
# or: docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.litestream.yml up -d
```

The example config writes to a local file replica. For production S3/R2 setup
and restore drills, see `docs/backup.md`.

## 6. Start the stack

Podman:

```bash
podman compose -f deploy/docker-compose.yml up -d
podman compose -f deploy/docker-compose.yml ps
podman compose -f deploy/docker-compose.yml logs -f stalwart hail-api hail-worker
```

Docker:

```bash
docker compose -f deploy/docker-compose.yml up -d
docker compose -f deploy/docker-compose.yml ps
docker compose -f deploy/docker-compose.yml logs -f stalwart hail-api hail-worker
```

Check readiness:

```bash
curl -i http://127.0.0.1:8080/readyz
curl -i https://mail.example.com:8080/readyz
```

## 7. Complete the first-run wizard

Open:

```text
https://mail.example.com:8080/setup
```

If DNS is not ready, use SSH port forwarding:

```bash
ssh -L 8080:127.0.0.1:8080 you@mail.example.com
```

Then open `http://127.0.0.1:8080/setup`.

In the wizard:

1. Paste the setup bootstrap token from `HAIL_SETUP_BOOTSTRAP_TOKEN`.
2. Enter Stalwart admin credentials. The local compose default is
   `admin` / `admin1234` from `STALWART_RECOVERY_ADMIN`; production operators
   should use their Stalwart recovery/admin credentials.
3. Create the admin user, for example `you@example.com`.
4. Enter a strong mailbox password and display name.
5. Add your mail domain, for example `example.com`.
6. Submit, then sign in with that admin account.

When `stalwart.management_url` is configured, that single submit authenticates
to Stalwart v0.16's REST API with an `authCode` request, exchanges the client
code for an in-memory bearer token, then calls `POST /api/principal` to create
the domain principal and `POST /api/principal` again to create the individual
mailbox principal. It finally performs a JMAP login to discover the account id
for hail's local user row. The domain value may include a trailing dot in the
UI; hail normalizes it to lowercase without the dot. The admin email must be
under that same domain. Hail never stores or logs the Stalwart admin password,
client code, or bearer token. If `stalwart.management_url` is intentionally
unset, the wizard does **not** mutate Stalwart; pre-create the domain and
account with the Stalwart WebUI/CLI first, then use the same mailbox
credentials in `/setup`.

After setup, signed-in admins can go to **Admin → Domains** to create another
shared domain and **Admin → Create user** to add more mailboxes. User creation
also ensures the email's domain exists before creating the Stalwart principal,
so adding `alice@example.com`, `bob@example.com`, and `team@example.com` under a
shared domain does not require Stalwart config edits or restarts.

For production, put real TLS in front of hail with Caddy, Traefik, or
Cloudflare Tunnel before inviting other users.

## 8. Publish DNS for direct SMTP

This section is only for the direct SMTP deployment shape. If you chose
VPS/WireGuard, Cloudflare Email Routing/import, or Gmail/provider import, use the
matching guide from [deployment.md](./deployment.md) instead.

For direct delivery to Stalwart, create:

```text
example.com.       MX   10 mail.example.com.
mail.example.com.  A       YOUR_HOST_IPV4
mail.example.com.  AAAA    YOUR_HOST_IPV6   # optional
example.com.       TXT     "v=spf1 mx -all"
```

After Stalwart generates DKIM keys, publish the DKIM TXT record it shows. Then
add DMARC:

```text
_dmarc.example.com. TXT "v=DMARC1; p=quarantine; rua=mailto:you@example.com"
```

Verify MX:

```bash
dig +short MX example.com
dig +short A mail.example.com
```

## 9. Send a test email

From an outside mailbox, send:

```text
To: you@example.com
Subject: hail test

hello from outside
```

Open hail and go to **Screener**. Mail from a new sender should appear there.
Approve the sender and choose Imbox, Feed, or Paper Trail. Future mail from
that sender follows the rule. If the sender is already approved, check Imbox.

## 10. Troubleshooting

### Mail is not arriving

```bash
dig +short MX example.com
dig +short TXT example.com
dig +short A mail.example.com
nc -vz mail.example.com 25
podman compose -f deploy/docker-compose.yml logs --tail=200 stalwart
# or: docker compose -f deploy/docker-compose.yml logs --tail=200 stalwart
```

If port 25 fails from outside your network, this direct quickstart cannot
receive public mail yet. Choose an alternate ingress path in
[deployment.md](./deployment.md), such as VPS/WireGuard MX or provider import.

### A container is failing

```bash
podman compose -f deploy/docker-compose.yml ps
podman compose -f deploy/docker-compose.yml logs --tail=200 hail-api
podman compose -f deploy/docker-compose.yml logs --tail=200 hail-worker
podman compose -f deploy/docker-compose.yml logs --tail=200 stalwart
```

Docker equivalent: replace `podman compose` with `docker compose`.
Common causes are a missing `HAIL_SERVER_KEY`, invalid TOML, or a host port that
is already in use.

### The UI loads but says not ready

```bash
curl -i http://127.0.0.1:8080/readyz
curl -i http://127.0.0.1:8080/healthz
```

`/healthz` is liveness. `/readyz` checks dependencies such as SQLite and JMAP.
If `/readyz` fails, read `hail-api` logs first, then Stalwart logs.

### Reset a throwaway test host

This deletes local container state. Do not run it on a real mailbox unless you
have backups.

```bash
podman compose -f deploy/docker-compose.yml down -v
podman compose -f deploy/docker-compose.yml up -d
# or: docker compose -f deploy/docker-compose.yml down -v
#     docker compose -f deploy/docker-compose.yml up -d
```
