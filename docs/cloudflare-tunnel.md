# Cloudflare Tunnel recipes for hail

This guide expands the Cloudflare Tunnel deployment shape from
[design.md §11](./design.md#11-cloudflare-tunnel-recipes-v1). It is for
operators who want hail's web UI reachable through Cloudflare while Stalwart
continues to own mail storage, SMTP, and JMAP.

> **Verify against current Cloudflare docs:** dashboard labels and Email
> Routing capabilities change. The commands and DNS shapes below are concrete,
> but treat Cloudflare-specific UI names as a checklist to confirm before a
> production cutover.

## Recommended setup at a glance

For home-hosted hail with private local storage, the recommended production
shape is:

```text
Inbound mail:
Remote sender
  → DNS-only MX: mx.example.com
  → small public VPS gateway
  → WireGuard tunnel
  → home Stalwart server
  → hail routing/UI

Web UI:
Browser
  → https://mail.example.com
  → Cloudflare Tunnel
  → hail-api at home

Outbound mail:
Home Stalwart
  → authenticated smarthost / relay
  → recipient inbox
```

This gives the main self-hosting benefits without relying on residential mail
connectivity:

- mail data stays at home on the operator's Stalwart server;
- the home IP does not appear in public DNS;
- residential port-25 blocks and CGNAT are bypassed;
- inbound mail remains a normal SMTP transaction into Stalwart, not a forwarded
  Cloudflare Email Routing import workaround;
- the hail web UI is exposed through Cloudflare Tunnel instead of direct public
  HTTP ports;
- outbound deliverability uses a reputable smarthost rather than a residential
  or low-reputation VPS IP.

Use separate public names:

```text
mx.example.com      → VPS public IP, DNS-only / grey cloud
mail.example.com    → Cloudflare Tunnel to hail-api, proxied / orange cloud
```

Core DNS shape:

```text
example.com       MX     10 mx.example.com
mx.example.com    A      <VPS_PUBLIC_IP>                DNS only
mail.example.com  CNAME  <tunnel>.cfargotunnel.com      Proxied
```

SPF, DKIM, and DMARC should match the outbound smarthost or signer. Start DMARC
at `p=none` while validating alignment and tighten later after reports are clean.

The VPS is a gateway, not the mailbox host. Prefer HAProxy with PROXY protocol
or a real MTA relay such as Postfix/OpenSMTPD over blind NAT, because
NAT/MASQUERADE can hide original sender IPs from Stalwart. The home server runs
Stalwart, `hail-api`, `hail-worker`, SQLite state, `cloudflared`, and the
WireGuard peer; it should not need public inbound ports.

One-line recommendation: **Cloudflare Tunnel for web, DNS-only VPS MX over
WireGuard for inbound SMTP, and a reputable smarthost for outbound mail.**

## Recipe comparison

| Question | Recipe A: web-only Tunnel, direct SMTP | Recipe B: Cloudflare Email Routing + Tunnel | Recipe C: VPS MX gateway + WireGuard |
| --- | --- | --- | --- |
| Best for | VPS/business ISP/home network with public inbound TCP/25 | CGNAT or blocked TCP/25 when forwarding/import is acceptable | Home-hosted storage behind CGNAT/residential blocks while preserving normal SMTP delivery |
| Public MX points to | Your Stalwart host or direct mail host | Cloudflare MX hosts (`route*.mx.cloudflare.net`) | `mx.example.com` on a small VPS |
| Cloudflare role for mail | DNS only | Inbound mail receiver/forwarder | DNS only |
| Cloudflare role for web | Tunnel to `hail-api` | Tunnel to `hail-api` | Tunnel to `hail-api` |
| Home IP in public DNS | Yes, unless the direct host is a VPS | No | No |
| Inbound mail reaches Stalwart as | Direct SMTP connection | Forwarded/imported message via verified destination, Worker, or custom bridge | SMTP forwarded over WireGuard from the VPS |
| Preserves original SMTP session | Yes | No; Cloudflare receives and forwards/imports | Yes if using HAProxy PROXY protocol or MTA relay; weaker with NAT/MASQUERADE |
| Queueing during home outage | Only if Stalwart/direct host is up or has its own queue | Cloudflare/destination behavior; bridge-dependent | Yes if using a VPS MTA relay; no/limited with pure TCP proxy or NAT |
| Operational complexity | Lowest | Medium to high because a secure bridge/import path is required | Medium; requires VPS, WireGuard, and a forwarding layer |
| Privacy trade-off | Direct host IP is public | Cloudflare sees/handles inbound messages | VPS sees SMTP metadata and may transiently queue mail depending on design |
| Reliability trade-off | Depends on home/direct-host uptime | Depends on Cloudflare Email Routing plus bridge | VPS can absorb public network role; MTA relay variant can queue while home is down |
| Outbound recommendation | Smarthost strongly recommended | Smarthost required for practical sending | Smarthost strongly recommended; do not rely on residential outbound |
| Recommended status | Good for simple VPS/direct-port deployments | Useful fallback/import recipe | Preferred advanced home-hosted setup |

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

Cloudflare Tunnel can proxy HTTP and several client-assisted TCP protocols, but
it does **not** make public SMTP on port 25 reachable from the internet in the
normal MX sense. Recipe A keeps port 25 direct. Recipe B uses Cloudflare Email
Routing for inbound mail and a relay/smarthost for outbound mail. Recipe C uses
a lightweight public VPS as the MX gateway and carries SMTP back to a home
Stalwart host over WireGuard; this is usually the most realistic
CGNAT/residential setup when you want Stalwart to receive the original SMTP
transaction instead of importing forwarded mail.

## Current Cloudflare / mail constraints to verify

As of the May 2026 docs reviewed for this guide:

- Cloudflare DNS proxy status applies to `A`, `AAAA`, and `CNAME` records. Raw
  mail records and MX targets must remain **DNS only**. Do not orange-cloud
  SMTP, IMAP, POP3, or submission hostnames unless you are deliberately using a
  Cloudflare product that supports that exact protocol and client flow.
- Cloudflare Tunnel public hostnames are appropriate for HTTP/HTTPS web surfaces
  such as `hail-api`. Tunnel TCP services are client-assisted
  (`cloudflared access tcp`) and are not a drop-in public MX endpoint for
  arbitrary remote MTAs on port 25.
- `cloudflared` connectors need outbound connectivity to Cloudflare Tunnel
  edges, commonly port `7844` over TCP/UDP depending on protocol fallback.
- Cloudflare Email Routing remains primarily an inbound forwarding/routing
  product. Cloudflare Email Service / Email Sending may be useful for app or
  transactional mail, but is plan/limit dependent and is not a transparent SMTP
  smarthost for Stalwart.
- Many VPS providers block or throttle outbound port 25 by default. Inbound port
  25 for MX may also be policy-gated. Verify your provider before building a
  gateway.
- Stalwart v0.16+ expects the normal WebUI/JMAP public URL to be HTTPS on the
  configured hostname. Plain `:8080` is mainly bootstrap/recovery or
  reverse-proxy upstream, not the day-to-day public admin URL.

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

## Recipe C: VPS MX gateway plus WireGuard to home Stalwart

Use this when the operator wants mailbox data to live at home but residential
inbound TCP/25, CGNAT, or home-IP privacy makes direct MX impractical. A small
public VPS becomes the DNS-visible MX endpoint. It forwards SMTP over a
WireGuard tunnel to Stalwart running on the home host. Cloudflare is DNS for
mail and Tunnel for the hail web UI; Cloudflare is not in the SMTP data path.

This is the preferred advanced recipe when you want Stalwart to receive mail via
normal SMTP rather than Cloudflare Email Routing forwarding/import.

### Flow

```text
Remote MTA ──SMTP/25──> VPS public IP ──HAProxy/Postfix──> WireGuard ──> Home Stalwart
Browser ──HTTPS──> Cloudflare ──Tunnel──> cloudflared ──HTTP──> hail-api
Stalwart outbound ──submission/API──> paid smarthost ──> Recipient MX
```

### DNS records for Recipe C

Keep MX and web names separate. DNS values are hostnames, not URLs; never write
`https://` or `://` in MX, A, SPF, DKIM, or DMARC values.

| Type | Name | Content | Proxy | Purpose |
| --- | --- | --- | --- | --- |
| `A` | `mx.example.com` | `203.0.113.10` | DNS only | VPS gateway public address |
| `MX` | `example.com` | `mx.example.com` priority `10` | DNS only | Public inbound SMTP target |
| `CNAME` | `mail.example.com` | `<uuid>.cfargotunnel.com` | Proxied | hail web UI through Tunnel |
| `TXT` | `example.com` | provider-specific SPF | DNS only | Authorize outbound smarthost, not the home host |
| `TXT` | `_dmarc.example.com` | `v=DMARC1; p=none; rua=mailto:postmaster@example.com` initially | DNS only | DMARC reports during setup |
| `TXT` | `<selector>._domainkey.example.com` | smarthost/Stalwart DKIM key | DNS only | DKIM for the system that signs outbound mail |

Example SPF for a smarthost must come from that provider's current docs. Do not
invent values such as `include:brevo.com` unless the provider tells you to use
that exact include. If only the smarthost sends outbound mail for the domain,
SPF usually authorizes only the smarthost. The VPS receives inbound mail and does
not need SPF authorization unless it also sends.

### VPS provider checklist

Before provisioning, verify all of these with the provider and account age:

- Inbound TCP/25 to the VPS is allowed.
- Outbound TCP/25 policy is understood. Direct outbound is not required if
  Stalwart uses a smarthost, but local queue tests and bounce delivery may still
  touch port 25 depending on gateway design.
- Reverse DNS/PTR can be set for the VPS IP if the VPS ever sends mail directly.
- Abuse limits permit running a personal MX gateway.
- The VPS has a stable IPv4 address. IPv6 is useful but not a replacement for
  IPv4 MX reachability.

### WireGuard addressing

Example private tunnel addresses:

```text
VPS WireGuard:  10.0.0.1/24
Home Stalwart:  10.0.0.2/24
WireGuard UDP:  51820 on the VPS
```

Use normal WireGuard hardening: unique keys, `PersistentKeepalive = 25` on the
home peer, restricted firewall rules, and systemd enablement on both sides. The
home server only needs outbound UDP to the VPS; it does not need public inbound
ports.

### Forwarding design: prefer L4 proxy or MTA relay over blind NAT

Avoid a simple DNAT/MASQUERADE recipe like this as the final design:

```bash
iptables -t nat -A PREROUTING -i eth0 -p tcp --dport 25 -j DNAT --to-destination 10.0.0.2:25
iptables -t nat -A POSTROUTING -o wg0 -j MASQUERADE
```

It may deliver mail, but masquerading makes Stalwart see the VPS tunnel address
instead of the real remote sender IP. That weakens logs, abuse controls,
rate-limits, DNSBL/reputation checks, and forensic value.

Prefer one of these patterns:

1. **HAProxy TCP forwarding with PROXY protocol.** HAProxy listens on the VPS
   public mail ports and forwards to Stalwart over WireGuard using PROXY v2.
   Configure Stalwart to trust the VPS/WireGuard source for PROXY protocol. This
   preserves original client IP metadata while keeping storage at home.
2. **Postfix/OpenSMTPD edge relay.** The VPS is the public MX, accepts mail for
   your domain, queues during home outages, and relays to Stalwart over
   WireGuard. This is operationally robust but transiently stores mail on the
   VPS, so it is a privacy trade-off.

If you intentionally use raw NAT, document the loss of source IP as an accepted
trade-off and compensate with stricter filtering on the VPS edge.

### Ports to expose through the VPS

Minimum viable inbound mail:

| Port | Expose on VPS? | Notes |
| --- | --- | --- |
| `25/tcp` | Yes | Required for remote MTAs to deliver mail. |
| `465/tcp` | Optional | Only if external mail clients submit through Stalwart. Prefer Cloudflare Tunnel/hail web for normal use. |
| `587/tcp` | Optional | STARTTLS submission; newer Stalwart installs may not enable this by default. |
| `993/tcp` | Optional | IMAPS for external native clients. Not needed for hail web UI. |
| `8080/tcp` | No | Stalwart bootstrap/recovery or internal HTTP upstream only. Do not publish directly. |
| `443/tcp` | Usually no for hail | The hail web UI should be reached through Cloudflare Tunnel to `hail-api`. |

For hail, browsers talk to `hail-api`, not directly to Stalwart/JMAP. A Tunnel
public hostname should normally point to `http://hail-api:8080` inside the
Compose network. Expose Stalwart's own WebUI only for initial setup or deliberate
admin operations, and prefer binding it to localhost/VPN/internal networks.

### Cloudflare Tunnel for hail web UI

Dashboard public hostname:

- Hostname: `mail.example.com`
- Service type: `HTTP`
- Service URL: `http://hail-api:8080`

If `cloudflared` runs on the home host outside Compose, the service URL can be a
LAN or WireGuard-reachable address. In the checked-in Compose overlay,
`cloudflared` runs in the Compose network and reaches `hail-api` by service name.

Protect administrative surfaces with hail authentication first. Cloudflare
Access can add another gate around `mail.example.com` or selected paths, but be
careful with path-only rules when an application uses OAuth/JMAP discovery and
redirects. Test login, setup, logout, and API calls after adding Access.

### Outbound mail

Recipe C still should not send directly from the residential home IP. Configure
Stalwart to use an authenticated smarthost such as Postmark, Mailgun, Amazon
SES, SMTP2GO, Brevo, or a VPS relay you control. Record:

- submission host and port, usually `587`, `465`, or provider-specific `2525`;
- SPF include or exact SPF record from the provider;
- DKIM selector and DNS record from the signing system;
- DMARC policy and report mailbox;
- whether Stalwart or the smarthost signs DKIM.

Start DMARC at `p=none` while validating alignment, then move to `quarantine` or
`reject` only after reports look clean.

### Smoke tests for Recipe C

From outside the VPS/home network:

```bash
dig +short MX example.com
dig +short A mx.example.com
nc -vz mx.example.com 25
curl -I https://mail.example.com/healthz
```

On the VPS:

```bash
wg show
ss -tlnp | egrep ':(25|465|587|993)'
# If using HAProxy:
journalctl -u haproxy --since '10 minutes ago'
```

On the home host:

```bash
wg show
ss -tlnp | egrep ':(25|465|587|993|8080)'
docker compose ps stalwart hail-api hail-worker
```

Send a real external test message to `smoke@example.com` and verify:

1. the VPS accepted the SMTP connection;
2. the WireGuard tunnel carried it to the home host;
3. Stalwart delivered it;
4. `hail-worker` routed it;
5. hail shows it in Screener or the expected view.

## Operational checklist

- Keep direct SMTP (`A`/`MX`) and tunnel CNAME names separate if Cloudflare will
  not allow both at `mail.example.com`; the clearest pattern is
  `mx.example.com` for SMTP and `mail.example.com` for hail web.
- Use DNS-only records for MX targets and mail authentication TXT records.
- Do not put URL schemes (`http://`, `https://`, `://`) in DNS mail records.
- Do not expose unauthenticated relay services through Cloudflare Tunnel.
- For home/CGNAT deployments that need real SMTP delivery into Stalwart, prefer
  Recipe C's VPS WireGuard gateway over Cloudflare Email Routing import bridges.
- If a VPS forwards SMTP with NAT/MASQUERADE, understand that Stalwart may lose
  the real sender IP; prefer HAProxy PROXY protocol or a real MTA relay.
- Prefer token-based tunnels for simple Compose deployments.
- Pin `cloudflare/cloudflared` to a version if you require reproducible releases;
  `latest` is convenient but moves.
- Re-run `curl -I https://mail.example.com/healthz` after every tunnel or DNS
  change.
- Re-run SPF, DKIM, and DMARC checks after enabling Email Routing, changing a
  smarthost, or moving SMTP through a VPS gateway.

## Related files

- `docs/design.md` §3 for the deployment shape.
- `docs/design.md` §11 for the supported Cloudflare/VPS recipes.
- `deploy/docker-compose.cloudflare.yml` once the compose overlay task lands.
- Stalwart reverse-proxy and HAProxy docs when using Recipe C with PROXY
  protocol.
