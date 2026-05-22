# Cloudflare Tunnel recipes for hail

This guide expands the Cloudflare Tunnel deployment shape from
[design.md §11](./design.md#11-cloudflare-tunnel-recipes-v1). It is for
operators who want hail's web UI reachable through Cloudflare while Stalwart
continues to own mail storage, SMTP, and JMAP.

> **Verify against current Cloudflare docs:** dashboard labels and Email
> Routing capabilities change. The commands and DNS shapes below are concrete,
> but treat Cloudflare-specific UI names as a checklist to confirm before a
> production cutover.

## Assumptions and names

The examples use these placeholders:

- Domain: `example.com`
- Public mail/web host: `mail.example.com`
- Operator host public IP: `203.0.113.10`
- Compose project network: `hail_default`
- hail API container name: `hail-api`
- Stalwart container name: `stalwart`
- Cloudflare tunnel name: `hail-mail`

Replace them before copying commands.

Cloudflare Tunnel can proxy HTTP and several TCP protocols, but it does **not**
make public SMTP on port 25 magically reachable from the internet in the normal
MX sense. Recipe A keeps port 25 direct. Recipe B uses Cloudflare Email Routing
for inbound mail and a relay/smarthost for outbound mail.

## Prerequisites

1. Your zone is active in Cloudflare.
2. You can edit DNS records for the domain.
3. Docker Compose or Podman Compose is installed on the hail host.
4. The base hail stack is already running or ready to run.
5. You know which ports Stalwart and hail-api expose internally.

Install `cloudflared` locally for setup commands if you want to manage tunnels
from the CLI:

```bash
# Debian/Ubuntu example; verify current package instructions first.
curl -L https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64 \
  -o /usr/local/bin/cloudflared
chmod +x /usr/local/bin/cloudflared
cloudflared --version
```

You can also avoid local credentials entirely by creating the tunnel in the
Cloudflare dashboard and passing the generated token to the container as
`TUNNEL_TOKEN`.

## Recipe A: web-only tunnel, SMTP direct to your host

Use this when the operator has a VPS, business ISP, or router forwarding that
allows inbound TCP/25 directly to Stalwart. Cloudflare fronts only the browser
surface (`https://mail.example.com`) and optionally any HTTP admin surface you
choose to expose. SMTP stays on the public IP.

### Flow

```text
Browser ──HTTPS──> Cloudflare ──Tunnel──> cloudflared ──HTTP──> hail-api
Remote MTA ──TCP/25──> 203.0.113.10 ──> Stalwart
```

### Cloudflare dashboard steps

1. Open **Zero Trust**.
2. Go to **Networks → Tunnels**.
3. Choose **Create a tunnel**.
4. Select **Cloudflared**.
5. Name it `hail-mail`.
6. Pick Docker as the connector environment and copy the generated token.
7. Add a public hostname:
   - Subdomain: `mail`
   - Domain: `example.com`
   - Type: `HTTP`
   - URL: `http://hail-api:8080` (or the internal port from your compose file)
8. Save the tunnel and wait for the connector to show healthy.

### Token-based Compose overlay snippet

The actual `deploy/docker-compose.cloudflare.yml` overlay is implemented by a
separate task. Until then, this is the intended service shape:

```yaml
services:
  cloudflared:
    image: cloudflare/cloudflared:latest
    restart: unless-stopped
    command: tunnel --no-autoupdate run
    environment:
      TUNNEL_TOKEN: ${CLOUDFLARE_TUNNEL_TOKEN:?set CLOUDFLARE_TUNNEL_TOKEN}
    depends_on:
      - hail-api
```

Run it with the base stack:

```bash
export CLOUDFLARE_TUNNEL_TOKEN='paste-token-from-dashboard'
docker compose \
  -f deploy/docker-compose.yml \
  -f deploy/docker-compose.cloudflare.yml \
  up -d
```

If you use Podman Compose, the command is usually the same with
`podman compose` substituted for `docker compose`.

### Named tunnel config-file alternative

Use this if you prefer `cloudflared login` and checked host config instead of a
single service token.

```bash
cloudflared tunnel login
cloudflared tunnel create hail-mail
cloudflared tunnel route dns hail-mail mail.example.com
```

Example `/etc/cloudflared/config.yml`:

```yaml
tunnel: hail-mail
credentials-file: /etc/cloudflared/hail-mail.json

ingress:
  - hostname: mail.example.com
    service: http://hail-api:8080
  - service: http_status:404
```

Equivalent service shape:

```yaml
services:
  cloudflared:
    image: cloudflare/cloudflared:latest
    restart: unless-stopped
    command: tunnel --config /etc/cloudflared/config.yml run
    volumes:
      - ./cloudflared/config.yml:/etc/cloudflared/config.yml:ro
      - ./cloudflared/hail-mail.json:/etc/cloudflared/hail-mail.json:ro
    depends_on:
      - hail-api
```

### DNS records for Recipe A

Keep SMTP direct. Cloudflare proxying must be **off** for direct mail records.

| Type | Name | Content | Proxy | Purpose |
| --- | --- | --- | --- | --- |
| `MX` | `example.com` | `mail.example.com` priority `10` | DNS only | Inbound SMTP target |
| `A` | `mail.example.com` | `203.0.113.10` | DNS only | Direct SMTP address |
| `CNAME` | `webmail.example.com` | `<uuid>.cfargotunnel.com` | Proxied | Optional alternate web name |
| `TXT` | `example.com` | `v=spf1 mx -all` | DNS only | Basic SPF for direct host |
| `TXT` | `_dmarc.example.com` | `v=DMARC1; p=quarantine; rua=mailto:postmaster@example.com` | DNS only | DMARC policy |

Cloudflare's dashboard-created public hostname usually creates the tunnel CNAME
for `mail.example.com` automatically. If you also need `mail.example.com` as an
A record for SMTP, Cloudflare may not allow a CNAME at the same name. In that
case use one of these patterns:

- `mail.example.com` = A record for SMTP, `webmail.example.com` = tunnel CNAME.
- `mx.example.com` = A record for SMTP/MX, `mail.example.com` = tunnel CNAME.

Pick the pattern before issuing user-facing URLs.

### Smoke tests for Recipe A

From a network outside the host:

```bash
curl -I https://mail.example.com/healthz
nc -vz mail.example.com 25
```

From the hail host:

```bash
docker compose ps cloudflared hail-api stalwart
docker compose logs --tail=100 cloudflared
```

A successful `/healthz` returns `204 No Content`. A successful SMTP connection
should print a `220` banner from Stalwart when tested with `telnet` or `swaks`.

## Recipe B: Cloudflare Email Routing plus Tunnel

Use this when inbound port 25 is unavailable because of CGNAT, a residential ISP,
or a firewall you do not control. Cloudflare receives inbound mail through its
MX hosts. You then forward each message to an operator-controlled destination
that ultimately reaches Stalwart/hail.

### Flow

```text
Remote MTA ──SMTP/25──> Cloudflare MX
Cloudflare Email Routing ──forward/worker──> relay or tunnel-exposed listener
Browser ──HTTPS──> Cloudflare Tunnel ──> hail-api
Stalwart outbound ──submission/API──> paid smarthost
```

The exact hop between Email Routing and your stack is the part most likely to
change. Cloudflare Email Routing commonly forwards to verified destination
addresses and can be paired with Workers for custom logic. Verify current Worker
and destination support before committing to a production architecture.

### DNS records for Recipe B

Enable Email Routing in the Cloudflare dashboard and let it propose DNS records,
or create the equivalent records yourself:

| Type | Name | Content | Priority | Proxy |
| --- | --- | --- | --- | --- |
| `MX` | `example.com` | `route1.mx.cloudflare.net` | `4` | DNS only |
| `MX` | `example.com` | `route2.mx.cloudflare.net` | `8` | DNS only |
| `MX` | `example.com` | `route3.mx.cloudflare.net` | `81` | DNS only |
| `TXT` | `example.com` | `v=spf1 include:_spf.mx.cloudflare.net -all` | n/a | DNS only |
| `TXT` | `_dmarc.example.com` | `v=DMARC1; p=quarantine; rua=mailto:postmaster@example.com` | n/a | DNS only |
| `CNAME` | `mail.example.com` | `<uuid>.cfargotunnel.com` | n/a | Proxied |

Cloudflare may recommend different MX priorities or SPF content. Prefer the
values shown by the dashboard for your zone if they differ.

### Cloudflare dashboard steps

1. Open the zone for `example.com`.
2. Go to **Email → Email Routing** and enable routing.
3. Confirm Cloudflare's MX and TXT records are active.
4. Add destination addresses and complete verification emails.
5. Create a routing rule such as `*@example.com` to the verified destination or
   to a Worker route if your account supports that feature.
6. In **Zero Trust → Networks → Tunnels**, create or reuse the `hail-mail`
   tunnel for the web UI and any non-25 relay listener you operate.

### Worker-defined SMTP/relay route sketch

One workable pattern is:

1. Email Routing accepts `user@example.com`.
2. A Cloudflare Worker normalizes the message and forwards it to a private relay
   endpoint that you expose through the tunnel on HTTPS or a non-25 TCP service.
3. That relay injects into Stalwart locally.

Pseudo-config for the tunnel side:

```yaml
ingress:
  - hostname: mail.example.com
    service: http://hail-api:8080
  - hostname: inbound-relay.example.com
    service: http://stalwart-relay:8025
  - service: http_status:404
```

Token-based Compose snippet:

```yaml
services:
  cloudflared:
    image: cloudflare/cloudflared:latest
    restart: unless-stopped
    command: tunnel --no-autoupdate run
    environment:
      TUNNEL_TOKEN: ${CLOUDFLARE_TUNNEL_TOKEN:?set CLOUDFLARE_TUNNEL_TOKEN}
    depends_on:
      - hail-api
      - stalwart
```

The relay component is deliberately not specified in this task. Do **not** expose
an unauthenticated SMTP listener through a tunnel. Require a shared secret,
Cloudflare Access service token, mTLS, or another authentication layer.

### Outbound smarthost gotcha

Inbound routing does not solve outbound delivery. Most residential ISPs and many
cloud networks block outbound TCP/25 or give IPs poor mail reputation. Configure
Stalwart to send outbound mail through a smarthost using authenticated
submission/API.

Common choices:

- Postmark
- Mailgun
- Amazon SES
- SMTP2GO
- A VPS you control with clean port 25 egress

Cloudflare has announced outbound or Email Routing-adjacent capabilities in some
forms over time, but availability changes. Treat any Cloudflare outbound option
as beta/plan-dependent and verify against current Cloudflare docs before relying
on it. For production, a paid relay is the predictable answer.

Example Stalwart-style intent (verify exact config keys against the Stalwart
version you run):

```toml
[outbound]
smarthost = "smtp.postmarkapp.com:587"
username = "POSTMARK-SERVER-TOKEN"
password = "POSTMARK-SERVER-TOKEN"
starttls = true
```

### DKIM conflict to name explicitly

There are two possible DKIM signers in Recipe B:

1. Cloudflare Email Routing can sign or rewrite mail on your behalf for routed
   messages, depending on current product behavior and settings.
2. Stalwart can sign locally when it sends mail for your domain.

Do not publish overlapping DKIM selectors without understanding which system is
signing which message stream. If Cloudflare signs inbound-forwarded mail and
Stalwart signs outbound mail, use distinct selectors such as:

```text
cf2026._domainkey.example.com       TXT  Cloudflare-provided key
stalwart2026._domainkey.example.com TXT  Stalwart-provided key
```

The conflict: a message modified after signing can fail DKIM, and two systems
claiming the same selector can make verification fail unpredictably. Decide
whether Cloudflare signs on your behalf for routed mail or Stalwart signs
locally for mail it originates, then publish DNS for that decision only.

### Smoke tests for Recipe B

Check DNS propagation:

```bash
dig MX example.com +short
dig TXT example.com +short
dig TXT _dmarc.example.com +short
```

Check the web tunnel:

```bash
curl -I https://mail.example.com/healthz
```

Check Email Routing with a real external mailbox:

```bash
swaks --to test@example.com --server route1.mx.cloudflare.net
```

Then confirm the message arrives at the configured destination and appears in
Stalwart/hail after your relay step. Because the relay shape is operator-defined,
log every hop during first setup: Cloudflare routing event, Worker log, relay log,
Stalwart delivery log, and hail UI visibility.

## Operational checklist

- Keep direct SMTP (`A`/`MX`) and tunnel CNAME names separate if Cloudflare will
  not allow both at `mail.example.com`.
- Use DNS-only records for MX targets and mail authentication TXT records.
- Do not expose unauthenticated relay services through Cloudflare Tunnel.
- Prefer token-based tunnels for simple Compose deployments.
- Pin `cloudflare/cloudflared` to a version if you require reproducible releases;
  `latest` is convenient but moves.
- Re-run `curl -I https://mail.example.com/healthz` after every tunnel or DNS
  change.
- Re-run SPF, DKIM, and DMARC checks after enabling Email Routing or changing a
  smarthost.

## Related files

- `docs/design.md` §3 for the deployment shape.
- `docs/design.md` §11 for the two required Cloudflare recipes.
- `deploy/docker-compose.cloudflare.yml` once the compose overlay task lands.
