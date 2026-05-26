# Cloudflare mail testbed runbook

This runbook is an operator-assisted smoke procedure for hail with Cloudflare
Tunnel, optional Cloudflare Email Routing, optional VPS/WireGuard MX gateway,
and the checked-in synthetic mail fixtures. It complements
[cloudflare-tunnel.md](./cloudflare-tunnel.md), which documents the supported
deployment recipes.

> **Uncertainty boundary:** Cloudflare dashboard labels, Email Routing Worker
> capabilities, MX priorities, and outbound mail products change over time. Use
> the values shown in your Cloudflare zone as the source of truth. This repo does
> not yet ship a production Email Routing relay; any relay/import bridge below is
> operator-provided and must be secured.

## Goal

Prove, with recorded evidence, that:

1. `https://mail.example.com` reaches `hail-api` through Cloudflare Tunnel.
2. One inbound mail path accepts mail for `example.com`:
   - Cloudflare Email Routing accepts inbound mail; or
   - a DNS-only MX points to a VPS gateway that forwards over WireGuard to home
     Stalwart.
3. If an operator relay/import bridge or VPS gateway exists, routed mail lands
   in Stalwart and appears in hail's Screener / Imbox / Feed / Paper Trail
   views.
4. Outbound delivery expectations are explicit: inbound routing/gatewaying does
   not by itself make Stalwart a reputable outbound sender.

## Placeholders

| Placeholder | Meaning |
| --- | --- |
| `example.com` | Cloudflare zone under test |
| `mail.example.com` | Public hail web URL served through Tunnel |
| `mx.example.com` | Optional DNS-only MX host pointing at a VPS gateway |
| `inbound-relay.example.com` | Optional authenticated relay endpoint for routed mail |
| `smoke@example.com` | Mailbox/routing address under test |
| `operator@example.net` | External mailbox used for verification/capture |
| `CLOUDFLARE_TUNNEL_TOKEN` | Dashboard-generated cloudflared connector token |

## Required files

- `deploy/docker-compose.yml` — canonical Stalwart + hail stack.
- `deploy/docker-compose.cloudflare.yml` — cloudflared overlay. See the
  "Cloudflare Tunnel overlay" section below.
- `tests/fixtures/mail/*.eml` — synthetic RFC822 messages.
- `tests/fixtures/mail/README.md` — fixture purpose and expected view.

Default smoke corpus:

| Fixture | Expected purpose |
| --- | --- |
| `personal-simple.eml` | Unknown personal sender; Screener first, then Imbox after approval |
| `newsletter-tracking-pixel.eml` | Newsletter / Feed candidate with remote tracking pixel |
| `receipt-papertrail.eml` | Receipt / Paper Trail candidate |

## Smoke modes

### Mode 1: full Cloudflare Email Routing-to-hail path

Use only when a working authenticated bridge carries Cloudflare Email Routing
messages into Stalwart/hail. Possible shapes:

- Email Routing to a Worker, then Worker POSTs raw message content to an
  authenticated private relay exposed through Tunnel or another private ingress;
- Email Routing to a verified catch mailbox, then an operator-controlled fetcher
  imports messages into Stalwart; or
- another bridge preserving enough RFC822 content for hail routing/rendering.

Do **not** claim Mode 1 passed unless the message traversed Cloudflare Email
Routing and arrived in Stalwart/hail without manual `Email/import` bypassing the
Cloudflare hop.

### Mode 2: Cloudflare ingress + controlled JMAP import

Use when the relay bridge is unavailable. This proves the web tunnel and
Cloudflare Email Routing destination, then imports the same fixture corpus via
JMAP to validate hail UI behavior. It is **not** an end-to-end proof of
Cloudflare delivery into Stalwart; label notes as "ingress + controlled import".

### Mode 3: VPS/WireGuard MX gateway to home Stalwart

Use when `example.com` has DNS-only MX records pointing at a public VPS gateway,
and the gateway forwards SMTP to Stalwart over WireGuard. This proves a more
traditional SMTP path than Email Routing while still hiding the home IP and
working around residential port blocks.

Do **not** claim Mode 3 passed unless the message was accepted on the public VPS
MX and arrived in home Stalwart through the tunnel. If the gateway uses
NAT/MASQUERADE, record that Stalwart may see the VPS/tunnel IP rather than the
remote sender. Prefer HAProxy PROXY protocol or a real MTA relay if preserving
sender IP matters for the run.
## One-time setup

### 1. Prepare hail config and Compose

Create local deployment files as in [quickstart.md](./quickstart.md):

```bash
cp deploy/hail.example.toml deploy/hail.toml
cp deploy/stalwart.example.toml deploy/stalwart.toml
cp deploy/.env.example deploy/.env 2>/dev/null || touch deploy/.env
```

Set the public URL and Stalwart hostname:

```toml
# deploy/hail.toml
[server]
public_url = "https://mail.example.com"
```

```toml
# deploy/stalwart.toml
[server]
hostname = "mail.example.com"
```

Generate and store a stable server key. On Fedora/toolbox hosts, use
`tbx openssl ...` if `openssl` is not available directly.

```bash
openssl rand -hex 32
```

```dotenv
# deploy/.env
HAIL_PUBLIC_URL=https://mail.example.com
HAIL_SERVER_KEY=REPLACE_WITH_64_HEX_CHARS
```

### 2. Create the Cloudflare Tunnel token

Dashboard flow to verify against the current Cloudflare UI:

1. Open **Zero Trust**.
2. Go to **Networks → Tunnels**.
3. Create a `cloudflared` tunnel named `hail-mail`.
4. Choose Docker/Compose connector instructions.
5. Copy the generated token.
6. Add a public hostname:
   - Hostname: `mail.example.com`
   - Service type: `HTTP`
   - Service URL: `http://hail-api:8080`
7. Wait for the connector to show healthy after the stack starts.

Add the token to `deploy/.env`; do not commit it:

```dotenv
CLOUDFLARE_TUNNEL_TOKEN=PASTE_DASHBOARD_TOKEN_HERE
```

### 3. Start with the Cloudflare overlay

`deploy/docker-compose.cloudflare.yml` adds `cloudflared` to the canonical stack
and removes the direct host port for `hail-api:8080`; the Tunnel becomes the
public web surface. It intentionally does not create the Email Routing relay for
Mode 1. For Recipe B, the overlay contains a commented Stalwart port override
that should only be enabled after you have moved inbound SMTP to Cloudflare Email
Routing and built the secured relay/destination path described here.

Podman:

```bash
podman compose \
  -f deploy/docker-compose.yml \
  -f deploy/docker-compose.cloudflare.yml \
  --env-file deploy/.env \
  up -d --build
```

Docker:

```bash
docker compose \
  -f deploy/docker-compose.yml \
  -f deploy/docker-compose.cloudflare.yml \
  --env-file deploy/.env \
  up -d --build
```

Check status and logs:

```bash
podman compose -f deploy/docker-compose.yml -f deploy/docker-compose.cloudflare.yml ps
podman compose -f deploy/docker-compose.yml -f deploy/docker-compose.cloudflare.yml logs --tail=100 cloudflared hail-api hail-worker stalwart
# or replace `podman compose` with `docker compose`
```

Expected web checks:

```bash
curl -i https://mail.example.com/healthz
curl -i https://mail.example.com/readyz
```

`/healthz` should show liveness. `/readyz` should succeed only after SQLite and
Stalwart/JMAP are reachable. If `/healthz` works but `/readyz` fails, debug
hail/Stalwart before mail tests.

### 4. DNS/MX for inbound mail

Choose exactly one public inbound path for a smoke run.

#### Option A: Cloudflare Email Routing

In Cloudflare **Email → Email Routing**, enable routing for `example.com` and
let Cloudflare create or recommend DNS. The common shape at the time of writing
is:

| Type | Name | Content | Priority | Proxy |
| --- | --- | --- | --- | --- |
| `MX` | `example.com` | `route1.mx.cloudflare.net` | `4` | DNS only |
| `MX` | `example.com` | `route2.mx.cloudflare.net` | `8` | DNS only |
| `MX` | `example.com` | `route3.mx.cloudflare.net` | `81` | DNS only |
| `TXT` | `example.com` | `v=spf1 include:_spf.mx.cloudflare.net -all` | n/a | DNS only |
| `TXT` | `_dmarc.example.com` | `v=DMARC1; p=quarantine; rua=mailto:operator@example.net` | n/a | DNS only |
| `CNAME` | `mail.example.com` | Cloudflare Tunnel target | n/a | Proxied |

Prefer dashboard-provided records if they differ. Do not put a direct SMTP A/MX
record at the same name as a tunnel CNAME unless intentionally using Recipe A in
`docs/cloudflare-tunnel.md`.

#### Option B: VPS/WireGuard MX gateway

Use this for `docs/cloudflare-tunnel.md` Recipe C. Cloudflare is DNS-only for
mail; the public SMTP connection goes to the VPS, then across WireGuard to the
home Stalwart host.

| Type | Name | Content | Priority | Proxy |
| --- | --- | --- | --- | --- |
| `A` | `mx.example.com` | VPS public IPv4 | n/a | DNS only |
| `MX` | `example.com` | `mx.example.com` | `10` | DNS only |
| `TXT` | `example.com` | smarthost/provider SPF | n/a | DNS only |
| `TXT` | `_dmarc.example.com` | `v=DMARC1; p=none; rua=mailto:operator@example.net` | n/a | DNS only |
| `CNAME` | `mail.example.com` | Cloudflare Tunnel target | n/a | Proxied |

Keep `mx.example.com` and `mail.example.com` separate. Do not use URL syntax in
DNS values. Verify the VPS provider allows inbound TCP/25 before running the
smoke.

Verify from outside your LAN if possible:

```bash
dig +short MX example.com
dig +short A mx.example.com
dig +short TXT example.com
dig +short TXT _dmarc.example.com
dig +short CNAME mail.example.com
```

### 5. Email Routing destinations, rules, or VPS gateway

For Cloudflare Email Routing, minimum ingress-only setup:

1. Add and verify `operator@example.net` as a destination address.
2. Create a rule for `smoke@example.com` or `*@example.com` to that destination.
3. Send an ordinary external email to `smoke@example.com` and confirm it reaches
   `operator@example.net`.

Full-path setup adds a secured bridge:

1. Configure a Worker/destination/fetcher/relay that receives Email Routing
   messages.
2. Authenticate Cloudflare or the Worker before accepting imports. Examples:
   Cloudflare Access service token, mTLS, strong shared secret, signed requests,
   or another auditable control.
3. Inject into Stalwart locally or via authenticated JMAP import.
4. Record relay logs during the first run.

Do **not** expose unauthenticated SMTP or HTTP import endpoints through Tunnel.

For a VPS/WireGuard MX gateway:

1. Start WireGuard on the VPS and home host; record `wg show` handshake evidence.
2. Start the VPS forwarding layer: HAProxy with PROXY protocol, Postfix/OpenSMTPD
   relay, or a consciously accepted NAT gateway.
3. Ensure the home Stalwart listener is reachable from the VPS over WireGuard.
4. Record whether original sender IP is preserved. HAProxy PROXY protocol or a
   real MTA relay is preferred; NAT/MASQUERADE may hide the sender behind the
   VPS tunnel address.
5. Keep Stalwart/hail web access through Cloudflare Tunnel to `hail-api`; do not
   publish Stalwart `:8080` directly.

### 6. Smarthost note for outbound mail

Cloudflare Email Routing is inbound for this runbook. For reply/send smoke tests,
configure Stalwart with an authenticated smarthost such as Postmark, Mailgun,
Amazon SES, SMTP2GO, or a VPS relay you control. Record provider, submission
host/port, SPF/DKIM/DMARC changes, and DKIM selector without committing secrets.
If no smarthost is configured, write "outbound not tested" and skip outbound
assertions.

## Sending synthetic fixtures through the inbound path

The fixtures use reserved `.test` / `.example` domains. For public smoke tests,
send with a real SMTP envelope recipient at your zone. Create temporary rewritten
copies for clearer UI checks:

```bash
mkdir -p /tmp/hail-cf-fixtures
for name in personal-simple newsletter-tracking-pixel receipt-papertrail; do
  perl -0pe 's/^To: .*/To: smoke@example.com/m' \
    "tests/fixtures/mail/${name}.eml" \
    > "/tmp/hail-cf-fixtures/${name}.eml"
done
```

Keep originals unchanged in git.

### Option A: direct SMTP to the public MX with swaks

Use when `swaks` is available and the selected MX accepts direct test SMTP. For
Cloudflare Email Routing, the server is usually `route1.mx.cloudflare.net`. For
the VPS gateway recipe, use `mx.example.com`.

Cloudflare Email Routing example:

```bash
swaks --server route1.mx.cloudflare.net \
  --from sender-smoke@example.net \
  --to smoke@example.com \
  --data @/tmp/hail-cf-fixtures/personal-simple.eml

swaks --server route1.mx.cloudflare.net \
  --from newsletter@example.net \
  --to smoke@example.com \
  --data @/tmp/hail-cf-fixtures/newsletter-tracking-pixel.eml

swaks --server route1.mx.cloudflare.net \
  --from receipts@example.net \
  --to smoke@example.com \
  --data @/tmp/hail-cf-fixtures/receipt-papertrail.eml
```

VPS gateway example:

```bash
swaks --server mx.example.com \
  --from sender-smoke@example.net \
  --to smoke@example.com \
  --data @/tmp/hail-cf-fixtures/personal-simple.eml

swaks --server mx.example.com \
  --from newsletter@example.net \
  --to smoke@example.com \
  --data @/tmp/hail-cf-fixtures/newsletter-tracking-pixel.eml

swaks --server mx.example.com \
  --from receipts@example.net \
  --to smoke@example.com \
  --data @/tmp/hail-cf-fixtures/receipt-papertrail.eml
```

Record the SMTP transcript. For Cloudflare Email Routing, a `250` response proves
only that Cloudflare accepted the message; hail visibility still depends on the
routing rule and bridge. For the VPS gateway, a `250` from the VPS proves only
edge acceptance; still verify WireGuard forwarding, Stalwart delivery, worker
routing, and hail UI visibility.

### Option B: external mailbox fallback

Use when direct SMTP is blocked or rate-limited:

1. From a mailbox outside the test domain, send to `smoke@example.com`.
2. Attach the fixture as `.eml` only if the relay/fetcher imports attachments;
   otherwise send equivalent subject/body text manually.
3. Record received headers at `operator@example.net` or relay logs.

This fallback is less faithful because webmail often rewrites MIME/HTML.

### Mode 2 controlled JMAP import

After proving Cloudflare delivered to `operator@example.net`, import the same
fixtures into Stalwart so hail UI checks run against known content. The local
helper is documented in [testing.md](./testing.md#local-mail-testbed):

```bash
HAIL_RUN_LOCAL_MAIL_TESTBED=1 \
HAIL_TESTBED_JMAP_URL='https://mail.example.com/jmap' \
HAIL_TESTBED_EMAIL='smoke@example.com' \
HAIL_TESTBED_PASSWORD='DO_NOT_COMMIT' \
  cargo test -p hail-test --test local_mail_testbed -- --nocapture
```

If JMAP is not exposed publicly, run from the host/SSH session and use the
local/internal Stalwart JMAP URL, for example `http://127.0.0.1:8080` or
`http://stalwart:8080` from the appropriate network namespace. This import is a
known Cloudflare delivery bypass; record it as such.

## Expected hail UI/API results

Sign in to hail at `https://mail.example.com` as the smoke mailbox. If this is a
fresh install, complete setup first as in [quickstart.md](./quickstart.md#6-complete-the-first-run-wizard).

### Readiness

```bash
curl -i https://mail.example.com/healthz
curl -i https://mail.example.com/readyz
```

Expected: `healthz` is alive; `readyz` succeeds only when dependencies are ready.

### Screener and views

For a new mailbox with unknown senders:

1. Open **Screener**.
2. `personal-simple.eml` sender appears as pending after worker processing.
3. Approve that sender as **Imbox**.
4. The message leaves Screener and appears in **Imbox**.
5. Future messages from that sender bypass Screener.

For the other fixtures, first-run behavior may also be Screener pending. Approve
or classify `newsletter-tracking-pixel.eml` into **Feed** and
`receipt-papertrail.eml` into **Paper Trail**, then verify each appears in the
chosen view. The newsletter render should not load the remote tracking pixel.

### Optional protected API checks

After browser login, use an HTTP client with the `hail.sid` cookie. Do not paste
live cookies into notes or committed files.

```bash
curl -i -b 'hail.sid=REDACTED' https://mail.example.com/api/auth/me
curl -i -b 'hail.sid=REDACTED' https://mail.example.com/api/views/screener
curl -i -b 'hail.sid=REDACTED' https://mail.example.com/api/views/imbox
curl -i -b 'hail.sid=REDACTED' https://mail.example.com/api/views/feed
curl -i -b 'hail.sid=REDACTED' https://mail.example.com/api/views/papertrail
```

Expected: authenticated JSON responses; Screener pending list shrinks after
approval; the relevant fixture appears in the approved/classified view.

## Run checklist

Copy this into the smoke-test task note or a temporary run log. Fill every blank
or mark `N/A`.

Create `scripts/cloudflare-smoke-checklist.md` only for an actual run artifact;
do not commit real domains, tunnel tokens, cookies, SMTP transcripts containing
private addresses, or other secrets.

```text
RUN
[ ] Date/time UTC:
[ ] Operator:
[ ] Mode: Email Routing full path | ingress + controlled import | VPS/WireGuard gateway
[ ] Domain:
[ ] Public URL:
[ ] Smoke mailbox:

TUNNEL / COMPOSE
[ ] CLOUDFLARE_TUNNEL_TOKEN stored in deploy/.env only
[ ] Started with deploy/docker-compose.yml + deploy/docker-compose.cloudflare.yml
[ ] cloudflared connector healthy in dashboard
[ ] curl https://mail.example.com/healthz result:
[ ] curl https://mail.example.com/readyz result:
[ ] Relevant logs captured with secrets redacted:

DNS / INBOUND ROUTING
[ ] Inbound mode: Cloudflare Email Routing | VPS/WireGuard MX gateway
[ ] MX records match selected mode:
[ ] SPF/DMARC records recorded:
[ ] If Email Routing: destination operator@example.net verified
[ ] If Email Routing: rule for smoke@example.com or wildcard active
[ ] If Email Routing: ordinary external email reached destination
[ ] If VPS gateway: inbound TCP/25 to VPS confirmed
[ ] If VPS gateway: WireGuard handshake confirmed
[ ] If VPS gateway: forwarding mode recorded: HAProxy PROXY | MTA relay | NAT
[ ] If VPS gateway: sender IP preservation result recorded

SMARTHOST / OUTBOUND
[ ] Smarthost configured? yes | no
[ ] If yes, provider and DNS changes recorded without secrets:
[ ] If no, outbound assertions skipped

FIXTURE SEND / IMPORT
[ ] Temporary fixture copies rewritten to smoke@example.com
[ ] personal-simple sent via selected inbound path; transcript/log:
[ ] newsletter-tracking-pixel sent via selected inbound path; transcript/log:
[ ] receipt-papertrail sent via selected inbound path; transcript/log:
[ ] If Mode 2, controlled JMAP import command succeeded:

HAIL RESULTS
[ ] Browser login succeeded as smoke mailbox
[ ] Screener showed unknown sender(s), or preexisting rule explains bypass
[ ] personal-simple approved/classified to Imbox and visible there
[ ] newsletter-tracking-pixel approved/classified to Feed and visible there
[ ] receipt-papertrail approved/classified to Paper Trail and visible there
[ ] Tracking pixel did not visibly load from remote URL during render
[ ] Optional API checks succeeded with cookie redacted

UNCERTAINTIES / FOLLOWUPS
[ ] Cloudflare UI/product differences noted:
[ ] Relay/import bridge gaps noted:
[ ] Bugs or missing automation filed as mu tasks:
```

## Troubleshooting notes

- Tunnel works but mail does not arrive: inspect the selected inbound path. For
  Email Routing, check Cloudflare routing events, destination verification,
  Worker/relay logs, and Stalwart logs in that order. For a VPS gateway, check
  VPS listener logs, WireGuard handshakes, forwarding/proxy logs, Stalwart logs,
  then hail-worker logs.
- Cloudflare or the VPS accepted SMTP but hail is empty: a `250` from the edge
  only proves edge acceptance; it does not prove your relay/import bridge or
  WireGuard forwarding worked.
- `/readyz` fails: inspect `hail-api` logs first, then Stalwart JMAP health.
- Fixture appears only in Screener: approve/classify the sender; unknown-sender
  Screener behavior is expected on a new mailbox.
- Outbound replies fail: verify the smarthost before debugging Cloudflare
  inbound routing.
