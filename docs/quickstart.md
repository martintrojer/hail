# Quickstart: receive mail in about 10 minutes

This guide takes a fresh host to one received message in hail. Replace
`example.com` and `mail.example.com` with your real domain and host name.

## 1. Prerequisites

You need:

- A domain you control, with DNS access.
- A host with TCP `25` and `8080` reachable from the internet, a Cloudflare
  account for the web tunnel / Email Routing recipe, or a VPS gateway for the
  WireGuard MX recipe.
- Podman with `podman compose`, or Docker with the Compose plugin.
- `openssl`.

Check tools:

```bash
openssl version
podman --version && podman compose version
# or: docker --version && docker compose version
```

If port 25 is blocked or you are behind CGNAT, use `docs/cloudflare-tunnel.md`.
The most realistic home-hosted mail ingress is often Recipe C: DNS-only MX to a
small VPS gateway, then WireGuard to the home Stalwart host. Cloudflare Email
Routing remains documented for forwarding/import-based setups. You can still
start the local stack with this guide.

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

Use the Compose service URL for Stalwart and set your public URL:

```toml
database_url = "sqlite:///var/lib/hail/hail.db"

[stalwart]
jmap_url = "http://stalwart:8080"

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
2. Create the admin user, for example `you@example.com`.
3. Enter a strong password and display name.
4. Add your mail domain, for example `example.com`.
5. Submit, then sign in with that admin account.

For production, put real TLS in front of hail with Caddy, Traefik, or
Cloudflare Tunnel before inviting other users.

## 8. Publish DNS for direct SMTP

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

If port 25 fails from outside your network, fix firewall/NAT rules or use
Cloudflare Email Routing for blocked port 25 / CGNAT deployments.

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
