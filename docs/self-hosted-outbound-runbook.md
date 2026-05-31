# Self-hosted outbound mail runbook

This runbook is for operators who run hail on their own domain and want
Stalwart to send real outbound mail directly to recipient MX servers with SPF,
DKIM, DMARC, and reverse DNS in place.

If Gmail or another provider remains your public mailbox edge, read
[provider-import-architecture.md](./provider-import-architecture.md) instead.
If you use Cloudflare for the web UI, inbound mail, or a VPS/WireGuard mail
edge, pair this runbook with
[cloudflare-tunnel.md](./cloudflare-tunnel.md).

## Audience and prerequisites

Use this path only if all of the following are true:

- You own a domain, for example `example.com`.
- You can edit DNS for that domain in Cloudflare, Route 53, your registrar DNS,
  or another authoritative DNS provider.
- You have a VPS, cloud host, or business connection with outbound TCP/25
  allowed and a stable public IPv4 address.
- You can ask the hoster or cloud provider to set reverse DNS/PTR for that IPv4
  address.
- You are willing to monitor mail reputation while the IP and domain warm up.

Most residential ISPs do **not** allow this shape. Common blockers are CGNAT,
blocked inbound TCP/25, blocked outbound TCP/25, dynamic IPv4 addresses, and no
way to set PTR records. If that describes your network, use a reputable
smarthost, a VPS mail edge, or provider import mode instead of trying to send
directly from home.

Reverse DNS is not optional for practical delivery. Gmail, Outlook, and many
corporate filters expect the sending IP to have a PTR record that points at your
mail hostname and for that hostname to resolve back to the same IP.

## DNS records

The examples below use:

```text
Domain:       example.com
Mail host:    mail.example.com
Server IPv4:  198.51.100.10
DKIM selector: default
```

Replace every example value before publishing.

### Direct self-hosted MX

Use these records when Stalwart receives inbound mail directly on the same host
or on a host you control:

```dns
example.com.                    IN MX 10 mail.example.com.
mail.example.com.               IN A 198.51.100.10
example.com.                    IN TXT "v=spf1 ip4:198.51.100.10 -all"
default._domainkey.example.com. IN TXT (
  "v=DKIM1; k=rsa; p=REPLACE_WITH_STALWART_RSA_PUBLIC_KEY"
)
_dmarc.example.com.             IN TXT (
  "v=DMARC1; p=quarantine; rua=mailto:postmaster@example.com; pct=100"
)
```

Notes:

- The SPF record above authorizes only `198.51.100.10` to send mail for
  `example.com`. Add only the IPs or `include:` mechanisms that actually send
  mail for your domain.
- The DKIM selector `default` matches Stalwart WebAdmin's default DKIM
  signature selector. If you choose another selector in Stalwart, use that
  selector in DNS.
- Publish the exact DKIM TXT value Stalwart gives you. The `p=` value is long;
  many DNS providers split it across quoted strings automatically.
- Start DMARC at `p=none` if this is a brand-new domain or migration and you
  want reporting before enforcement. Move to `p=quarantine` or `p=reject` after
  SPF and DKIM alignment are clean.

### Cloudflare Email Routing for inbound

If Cloudflare Email Routing is your inbound MX, do **not** publish the direct MX
record above. Use Cloudflare's `route*.mx.cloudflare.net` records as documented
in [cloudflare-tunnel.md](./cloudflare-tunnel.md#recipe-b-cloudflare-email-routing-plus-tunnel).
Outbound mail can still be sent by Stalwart, but your SPF, DKIM, and DMARC
records must describe the system that sends outbound mail.

### Reverse DNS / PTR

Ask your VPS or cloud provider to set:

```text
198.51.100.10 PTR mail.example.com.
```

Then make sure forward DNS agrees:

```dns
mail.example.com. IN A 198.51.100.10
```

Many receivers reject or spam-folder mail when PTR is missing, generic, or does
not match the SMTP hostname.

### Optional DNS records

These are useful after basic delivery works, but they are not required for hail:

- TLS-RPT: reports TLS delivery failures.
- MTA-STS: advertises an HTTPS policy for SMTP TLS expectations.
- BIMI: brand indicator records; usually requires stricter DMARC and a verified
  logo certificate.

Do not add optional records until you understand their operational impact.
Incorrect MTA-STS can break inbound delivery from compliant senders.

## Stalwart configuration steps

Use Stalwart WebAdmin for these steps. In the shipped Compose stack, Stalwart is
unmodified upstream and hail talks to it through JMAP; hail does not configure
public DNS or DKIM for you.

1. Open Stalwart WebAdmin.
2. Go to **Settings → Network**.
3. Set **Service hostname** to your public mail hostname:

   ```text
   mail.example.com
   ```

4. Go to **Settings → Server → DKIM signatures**.
5. Generate both Ed25519 and RSA signing keys if your Stalwart version offers
   both. Publish the TXT records Stalwart returns.
6. Make sure the RSA record for selector `default` is published if you keep the
   default selector:

   ```dns
   default._domainkey.example.com. IN TXT "v=DKIM1; k=rsa; p=..."
   ```

7. Go to **Settings → SMTP outbound**.
8. Confirm the default outbound route is active for internet recipients. Do not
   leave a local-only loopback route in production.

The checked-in local smoke compose file sets `HAIL_LOCAL_SINK=1` for the
`stalwart-init` sidecar. That deliberately patches Stalwart's MTA outbound
strategy so local tests never send to the internet. Production
`deploy/docker-compose.yml` must not set `HAIL_LOCAL_SINK=1`; see
[setup-runbook.md](./setup-runbook.md#2-start-the-stack).

If you previously ran the local sink against a persistent Stalwart volume that
you now want to use for production, remove that dev-only SMTP outbound rule in
Stalwart WebAdmin and restore the normal MX/default route before sending real
mail.

Finally, confirm Stalwart's public URL and JMAP discovery:

- Set `STALWART_PUBLIC_URL` in `deploy/.env` to the URL that should appear in
  Stalwart discovery when accessed publicly. For example:

  ```env
  STALWART_PUBLIC_URL=https://mail.example.com/
  ```

- Confirm discovery from outside the host:

  ```bash
  curl -i https://mail.example.com/.well-known/jmap
  ```

A redirect or JSON response should point at the public HTTPS Stalwart/JMAP URL
you intend clients to use. In the normal hail web shape, browsers use
`hail-api`; expose Stalwart WebAdmin/JMAP publicly only if you deliberately want
native clients or remote admin access.

## Compose deployment for the host

`deploy/docker-compose.yml` is the production Compose stack. It runs:

- `stalwart` for SMTP, IMAP, JMAP, and mail storage;
- `stalwart-init` for hail-friendly Stalwart settings;
- `hail-api` for the SPA, REST API, and future WebSocket;
- `hail-worker` for JMAP event subscriptions and hail routing jobs.

From `deploy/`, copy and edit the environment file:

```bash
cp .env.example .env
```

Set at least:

```env
HAIL_PUBLIC_URL=https://mail.example.com
STALWART_PUBLIC_URL=https://mail.example.com/
STALWART_RECOVERY_ADMIN=admin:REPLACE_WITH_LONG_RANDOM_PASSWORD
HAIL_SERVER_KEY=REPLACE_WITH_OPENSSL_RAND_HEX_32
HAIL_SETUP_BOOTSTRAP_TOKEN=REPLACE_WITH_OPENSSL_RAND_HEX_32
```

Optional Gmail/provider-import OAuth secrets are orthogonal to self-hosted
outbound mail. Set them only if this same deployment also lets users connect a
Gmail/provider account for import:

```env
HAIL_PROVIDER_IMPORT__GMAIL__OAUTH_CLIENT_ID=
HAIL_PROVIDER_IMPORT__GMAIL__OAUTH_CLIENT_SECRET=
```

If you terminate TLS inside Stalwart for SMTPS, IMAPS, or HTTPS/JMAP, configure
Stalwart certificates and ACME/mounted certificate settings in your private
Stalwart config or WebAdmin. If TLS terminates at a reverse proxy or
Cloudflare Tunnel for the hail web UI, keep `hail-api` on its internal HTTP port
and let the proxy provide public HTTPS.

Start the stack from `deploy/`:

```bash
podman compose up -d --build
```

or:

```bash
docker compose up -d --build
```

Use either Cloudflare Tunnel or a conventional reverse proxy for public HTTPS to
`hail-api`. The Cloudflare recipe is in
[cloudflare-tunnel.md](./cloudflare-tunnel.md). A conventional proxy recipe is in
[reverse-proxy.md](./reverse-proxy.md).

## Reputation warm-up

New mail server IPs and new domains rarely get full trust on day one. Major
receivers such as Gmail and Outlook ramp trust over days or weeks based on
volume, complaint rate, authentication alignment, bounce rate, and engagement.

Recommendations:

- Start with low volume: personal mail and a few test recipients, not bulk
  sends.
- Keep SPF, DKIM, and DMARC aligned for the visible From domain.
- Watch bounces and Stalwart outbound logs after every early send.
- Register the domain/IP in Google Postmaster Tools:
  <https://postmaster.google.com>
- Run pre-flight checks with mail-tester before sending broadly:
  <https://www.mail-tester.com/>
- Avoid forwarding test loops and mailing lists during warm-up; they can create
  confusing SPF/DKIM/DMARC results.

This runbook can make your DNS and Stalwart configuration correct. It cannot
make a brand-new IP instantly reputable.

## Troubleshooting

### `550 sender unauthenticated` or DMARC failure

Check alignment, not just record existence:

- SPF must authorize the server that connects to the recipient MX.
- DKIM must sign with a domain aligned with the message From domain, usually
  `example.com` or a subdomain you control.
- DMARC passes when SPF or DKIM passes **and** aligns with the visible From
  domain.
- Make sure Stalwart is actually signing outbound mail for the domain and
  selector whose TXT record you published.

Send to mail-tester and inspect the Authentication-Results header in the
received message.

### `5xx reverse DNS does not match` or generic PTR warnings

Ask the hoster to set PTR for the sending IP:

```text
198.51.100.10 PTR mail.example.com.
```

Then confirm `mail.example.com` resolves back to the same IP. Avoid generic PTR
names such as `vps-198-51-100-10.provider.example` for direct-to-MX sending.

### `TLS handshake failed`

Check that the certificate Stalwart or your proxy presents matches the hostname
used by peers:

- SMTP banner/HELO/EHLO hostname: `mail.example.com`.
- Certificate subject/SAN includes `mail.example.com` for SMTPS/submission or
  public Stalwart HTTPS/JMAP.
- `STALWART_PUBLIC_URL` uses the same public HTTPS hostname you expect clients
  and discovery to use.
- If TLS terminates in a reverse proxy, make sure SMTP TLS is not accidentally
  being sent to an HTTP-only listener.

### Outbound works locally but never reaches recipients

Check for a leftover local-smoke sink. `deploy/docker-compose.local.yml` sets
`HAIL_LOCAL_SINK=1` and forces Stalwart outbound through a local route for smoke
tests. Production must use `deploy/docker-compose.yml` without that variable or
a deliberate relay/smarthost route.

### Port 25 blocked

If the host cannot connect to recipient MX servers on TCP/25, direct outbound
mail will not work. Ask the provider to unblock port 25, move to a provider that
allows mail servers, or use a smarthost/provider SMTP route. The planned
`feature-outbound-via-provider-smtp` task tracks the "send via your existing
Gmail/provider SMTP" path that avoids direct outbound TCP/25 and does not
require operating your own mail reputation from scratch.

## Cross-references

- [cloudflare-tunnel.md](./cloudflare-tunnel.md): Cloudflare Tunnel for the web
  UI, Cloudflare Email Routing for inbound, and VPS/WireGuard MX recipes.
- [setup-runbook.md](./setup-runbook.md): first-run hail/Stalwart setup and the
  `HAIL_LOCAL_SINK=1` local smoke behavior.
- [provider-import-architecture.md](./provider-import-architecture.md):
  alternative path where Gmail/provider remains the public mail edge and hail
  imports into Stalwart.
- [provider-outbound-strategy.md](./provider-outbound-strategy.md): provider
  smarthost and Gmail API send architecture for provider-import deployments.
- `feature-outbound-via-provider-smtp` mu task: planned "send via your existing
  Gmail/provider SMTP" path that does not require direct outbound TCP/25,
  PTR-controlled IPs, or self-managed SPF/DKIM reputation.
