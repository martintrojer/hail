# Deployment options

This is the top-level deployment map for hail. Use it to choose a hosting shape;
then follow the focused deep-dive linked from that option.

hail has two separable surfaces:

- **Web/API access:** browsers reach `hail-api` over HTTPS. This can be direct,
  reverse-proxied, or exposed through Cloudflare Tunnel.
- **Mail ingress/egress:** Stalwart receives and sends mail. This cannot be
  solved by Cloudflare Tunnel alone because public SMTP delivery needs a real MX
  path on TCP/25 or a provider/import bridge.

## Quick chooser

| Option | Best for | Inbound mail | Web UI | Complexity | Recommended when |
| --- | --- | --- | --- | --- | --- |
| Direct VPS/all-in-one | Operators with a public VPS and port 25 allowed | Remote MTAs deliver directly to Stalwart on the VPS | HTTPS reverse proxy or Cloudflare Tunnel | Low | You are comfortable running the mailbox on a VPS. |
| Home server + direct SMTP | Static home IP/business ISP with TCP/25 open | Remote MTAs deliver directly to home Stalwart | Reverse proxy or Cloudflare Tunnel | Low-medium | Your home network can really receive public SMTP. |
| Home server + VPS/WireGuard MX gateway | Home storage behind CGNAT/residential blocks | DNS-only MX points to a small VPS, then WireGuard to home Stalwart | Cloudflare Tunnel to `hail-api` | Medium | You want mail data at home while preserving normal SMTP delivery. |
| Cloudflare Tunnel + Email Routing/import bridge | No public SMTP server and Cloudflare-managed domain | Cloudflare receives mail and forwards/imports into hail/Stalwart | Cloudflare Tunnel | Medium-high | You accept import/forwarding semantics instead of original SMTP sessions. |
| Gmail/provider import into Stalwart | Existing Gmail/provider mailbox users | Gmail/provider remains public edge; hail imports via provider API | Cloudflare Tunnel or normal HTTPS | Medium | You want the hail UX and a local Stalwart archive without operating public MX first. |

## Recommended shapes

### 1. Simple VPS deployment

Use this when your VPS provider allows inbound TCP/25 and you are comfortable
storing mail on the VPS.

```text
Remote sender --> SMTP/25 --> VPS Stalwart --> JMAP --> hail-api/hail-worker
Browser --> HTTPS --> reverse proxy or Cloudflare Tunnel --> hail-api
Stalwart --> SMTP smarthost or direct outbound --> recipients
```

Start with [quickstart.md](./quickstart.md). Add [reverse-proxy.md](./reverse-proxy.md)
if you terminate TLS yourself, and [backup.md](./backup.md) before relying on it.

### 2. Home server with direct SMTP

Use this only if your home/business network can receive public TCP/25 and you
are willing to publish the home IP as an MX target.

```text
Remote sender --> SMTP/25 --> home Stalwart
Browser --> HTTPS/Tunnel --> hail-api at home
Home Stalwart --> smarthost strongly recommended --> recipients
```

Follow [quickstart.md](./quickstart.md) and use the direct DNS section. If you
hide only the web UI behind Cloudflare, use Recipe A in
[cloudflare-tunnel.md](./cloudflare-tunnel.md#recipe-a-web-only-tunnel-smtp-direct-to-your-host).

### 3. Home server with VPS/WireGuard MX gateway

This is the preferred advanced home-hosted setup when TCP/25 or CGNAT blocks
home delivery but you still want Stalwart to receive mail through a normal SMTP
path.

```text
Inbound mail:
Remote MTA
  --> SMTP/25 --> DNS-only mx.example.com on VPS
  --> HAProxy PROXY protocol or MTA relay --> WireGuard
  --> SMTP --> home Stalwart

Web UI:
Browser
  --> HTTPS --> Cloudflare Tunnel
  --> HTTP --> hail-api at home

Outbound:
Home Stalwart
  --> authenticated smarthost/API relay --> recipient inbox
```

Key points:

- Keep `mx.example.com` DNS-only. Do not orange-cloud MX targets.
- Keep `mail.example.com` for the hail web UI through Cloudflare Tunnel.
- Prefer HAProxy with PROXY protocol or a real MTA relay on the VPS over blind
  NAT; blind NAT hides the original sender IP from Stalwart.
- Use a reputable outbound smarthost. Do not rely on residential outbound mail.

Deep dive: [cloudflare-tunnel.md, Recipe C](./cloudflare-tunnel.md#recipe-c-vps-mx-gateway-plus-wireguard-to-home-stalwart).
Operator smoke runbook: [cloudflare-testbed.md](./cloudflare-testbed.md).

### 4. Cloudflare Email Routing / import bridge

Use this when you cannot run public SMTP and accept that Cloudflare receives the
message first. This is not transparent SMTP into Stalwart.

```text
Remote sender
  --> SMTP --> Cloudflare Email Routing MX
  --> forward/Worker/import bridge --> HTTPS import endpoint over Tunnel
  --> queue/import --> Stalwart or hail import path
  --> hail routing/UI --> user
```

This option needs careful replay protection, authentication, queueing, duplicate
detection, and failure handling. Treat it as an import bridge, not as “SMTP over
Cloudflare Tunnel.”

Deep dive: [cloudflare-tunnel.md, Recipe B](./cloudflare-tunnel.md#recipe-b-cloudflare-email-routing-plus-tunnel).

### 5. Gmail/provider import into Stalwart

Use this when a working provider mailbox already exists and you want hail’s UI,
workflow, and local archive without first moving MX records.

```text
Gmail/provider mailbox
  --> provider API --> hail-worker provider import
  --> raw RFC822 import --> local Stalwart
  --> existing JMAP APIs --> hail UI

Outbound, preferred:
hail compose
  --> JMAP submission --> Stalwart
  --> provider smarthost --> recipients
```

Current provider import behavior is **one-way import**: provider -> Stalwart. Hail actions
archive/delete/classify local Stalwart mail; they do not mutate Gmail labels,
read state, archive state, Trash, or Spam.

Deep dive: [provider-import-architecture.md](./provider-import-architecture.md).
Outbound details: [provider-outbound-strategy.md](./provider-outbound-strategy.md).

## DNS and naming rules

- Use separate names for mail ingress and web UI when possible:
  - `mx.example.com`: public SMTP/MX target, DNS-only.
  - `mail.example.com`: hail web UI, reverse proxy or Cloudflare Tunnel.
- Never put `https://` or paths in MX/SPF/DKIM/DMARC values.
- Cloudflare Tunnel is appropriate for HTTP(S) to `hail-api`; it is not a public
  MX endpoint for arbitrary remote MTAs.
- SPF/DKIM/DMARC should describe the system that sends outbound mail. If you use
  a smarthost, follow that provider’s current DNS instructions.

## Which docs should contain what?

To avoid duplicate deployment material:

- This file decides **which deployment option to use**.
- [quickstart.md](./quickstart.md) is the shortest path to first boot and first
  received mail for direct/simple deployments.
- [cloudflare-tunnel.md](./cloudflare-tunnel.md) contains Cloudflare Tunnel,
  Email Routing/import bridge, and VPS/WireGuard MX implementation details.
- [cloudflare-testbed.md](./cloudflare-testbed.md) is an operator smoke-test
  runbook, not conceptual guidance.
- [provider-import-architecture.md](./provider-import-architecture.md) specifies
  the Gmail/provider-import implementation.
- [provider-outbound-strategy.md](./provider-outbound-strategy.md) specifies
  outbound-through-provider behavior and smarthost examples.
- [reverse-proxy.md](./reverse-proxy.md), [backup.md](./backup.md), and
  [upgrade.md](./upgrade.md) are focused operational references.

## Known docs cleanup still worth doing

The current docs are usable but still verbose. Follow-up cleanup should:

- shorten Cloudflare concept material now that this chooser exists;
- make `cloudflare-testbed.md` purely procedural;
- keep provider import implementation details in `provider-import-architecture.md`;
- remove temporary hidden reflow files if they are not intentionally kept.
