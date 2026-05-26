# Provider-backed hail modes

This note records deployment/use-case alternatives discussed for operators who do
not want to run a public mail server. These are **not the current v1 runtime
architecture**. Today hail is Stalwart-first: Stalwart owns messages, threads,
mailboxes, blobs, auth, SMTP/IMAP/JMAP, and hail adds the HEY-style product layer
on top. See [architecture.md](./architecture.md).

The modes below are useful product directions because many personal operators
already have a reliable mailbox at Gmail, Fastmail, Proton, Microsoft 365, or
another provider, and only want hail's workflow/UI locally behind Cloudflare
Tunnel.

## Why consider provider-backed modes?

Self-hosted SMTP is operationally expensive even when the software is simple:

- residential networks and CGNAT commonly block inbound TCP/25;
- outbound deliverability depends on IP reputation, SPF, DKIM, DMARC, reverse
  DNS, and provider policy;
- a VPS gateway adds cost and operational surface;
- Cloudflare Tunnel cannot act as a public MX for arbitrary remote MTAs;
- Cloudflare Email Routing can receive mail, but it is a forwarding/import path,
  not transparent SMTP into Stalwart.

For a user who already has a working Gmail mailbox, the cheapest and simplest
shape may be to keep Gmail as the mail provider and use hail as an alternative
local client/workflow layer.

## Mode P1: Provider-backed client/cache

Use case: an operator has an existing Gmail/provider mailbox and wants the
HEY-style hail experience without operating inbound SMTP, Stalwart, a VPS, or
Cloudflare Email Routing.

```text
Existing Gmail/provider mailbox
  ↔ provider API or IMAP/SMTP
  ↔ hail-worker on home server
  ↔ hail local cache/sidecar state
  ↔ hail-api + SPA over Cloudflare Tunnel
```

In this mode the provider remains the source of truth for delivery, spam,
labels/folders, sent mail, and account authentication. hail stores only the state
needed to make the UI fast and product-specific:

- sync cursors;
- cached message/thread summaries and bodies as needed;
- Screener decisions and sender classifications;
- Feed/Paper Trail/Imbox mapping;
- notes, clips, set-aside, reply-later, and other hail-only state;
- local search/cache data if implemented.

Recommended implementation for Gmail is the Gmail API rather than IMAP/SMTP:

- `users.history.list` for incremental sync;
- `users.messages.list/get/modify/send` for message operations;
- `users.drafts.*` for drafts;
- `users.labels.*` for label mapping;
- OAuth refresh tokens stored encrypted at rest.

IMAP/SMTP can be a fallback for providers without a good API, but it has weaker
incremental sync, label, and threading primitives.

### Pros

- No VPS, public IP, public SMTP, or inbound ports.
- Works behind CGNAT with only outbound HTTPS.
- Cheapest operational path for existing Gmail users.
- Gmail/provider handles spam filtering and deliverability.
- Cloudflare Tunnel is only needed for the hail web UI.

### Cons

- Not a self-hosted mail server; the provider still receives and stores mail.
- Provider API limits, OAuth policies, and product changes become dependencies.
- Hail must map provider labels/archive/delete semantics into its own workflow.
- Offline/local archive behavior depends on how much data hail caches.

## Mode P2: Provider importer into Stalwart

Use case: keep the current Stalwart-first architecture while avoiding public SMTP.
A home importer reads from Gmail/provider and imports messages into local
Stalwart. hail continues to talk only to Stalwart/JMAP.

```text
Existing Gmail/provider mailbox
  → provider API or IMAP fetcher
  → Stalwart Email/import or equivalent local delivery
  → hail-worker routes via JMAP
  → hail UI

Outbound:
hail/Stalwart
  → provider SMTP/API or smarthost
  → recipient
```

This is the best incremental bridge if we want to support no-public-server users
without rewriting hail's core storage model.

### Pros

- Preserves current hail architecture: Stalwart remains the mail store.
- Gives the operator a local standards-based archive.
- Future migration to direct SMTP, Cloudflare Worker import, or VPS MX gateway is
  possible without changing hail UI semantics.
- JMAP search/thread/blob behavior remains Stalwart's job.

### Cons

- Still runs Stalwart locally.
- Importer must be idempotent and map provider state to Stalwart cleanly.
- Provider remains the public mailbox and may retain a copy.
- Sent/draft/archive/delete reconciliation can be subtle.

## Mode P3: Cloudflare Email Routing import bridge

Use case: the operator owns a domain on Cloudflare but has no VPS/public server.
Cloudflare receives mail for the domain and a Worker imports raw messages into
hail/Stalwart through an HTTPS endpoint exposed by Cloudflare Tunnel.

```text
Remote sender
  → Cloudflare Email Routing MX
  → Cloudflare Worker email handler
  → HMAC/Access-authenticated HTTPS import endpoint over Tunnel
  → hail import queue
  → Stalwart or hail-native store
```

This can work, but it should be described honestly as an import bridge. It is not
`SMTP over Tunnel` and Stalwart does not receive the original SMTP session.

A robust bridge needs:

- raw RFC822 capture from the Worker;
- request signing or Cloudflare Access service-token authentication;
- replay protection;
- size limits and attachment policy;
- durable queueing/retry when the home server is down;
- duplicate detection by provider id and RFC822 `Message-ID`;
- operator-visible import logs;
- clear failure behavior when import permanently fails.

### Pros

- No VPS and no public home IP.
- Works with a custom domain on Cloudflare.
- Avoids Gmail as an intermediate mailbox.
- Could become a smooth setup if hail ships a first-class import endpoint.

### Cons

- More custom glue than it first appears.
- Cloudflare receives inbound mail and Worker/runtime limits apply.
- Stalwart loses SMTP-time checks and original SMTP session semantics.
- A safe import endpoint is product work, not just documentation.

## Mode P4: Hail-native mail store

Use case: hail becomes the primary local mail database/client, while Gmail,
Cloudflare Email Routing, or another provider is just a delivery/sync edge.
Stalwart may be optional or removed from this mode.

```text
Provider / Cloudflare / importer
  → hail-owned message + blob store
  → hail UI/API
```

This is the largest architectural change. Hail would own MIME parsing, message
storage, attachment storage, threading, search indexing, duplicate detection,
import/export, sent/draft state, and interoperability decisions.

### Pros

- Maximum product control.
- No dependency on Stalwart's data model for HEY-style workflows.
- Potentially simplest deployment for users who only want a local personal mail
  client/cache.

### Cons

- Reimplements a large amount of mail-server/mail-store functionality.
- Harder to interoperate with standard mail clients.
- Export, backup, search, attachments, threading, and MIME edge cases become hail
  responsibilities.
- This should not be treated as a small extension of v1.

## Recommendation

For the current project, keep **Stalwart-first v1** as the mainline.

If we add no-public-server support, the lowest-risk order is:

1. **Mode P2: provider importer into Stalwart** — preserves the current
   architecture and gives existing Gmail users a practical path.
2. **Mode P3: Cloudflare Email Routing import bridge** — useful for custom-domain
   users without Gmail-as-inbox, once a secure import queue exists.
3. **Mode P1: provider-backed client/cache** — attractive for Gmail users, but it
   requires defining which provider state is authoritative.
4. **Mode P4: hail-native mail store** — powerful but a separate product line or
   major-version architecture change.

For a single operator who already lives in Gmail and only wants the HEY-style UI,
Mode P1 may be the simplest user experience. For hail's current codebase, Mode
P2 is the more incremental implementation path.

## Open implementation questions

- Is Stalwart required in provider-backed deployments, or can it be optional?
- Are Gmail labels mapped to hail views, or are hail views sidecar-only?
- Does deleting/archive in hail mutate Gmail, or only local hail state?
- How much raw RFC822/body/attachment data is cached locally?
- How are OAuth refresh tokens encrypted, rotated, and revoked?
- How do multi-user deployments map provider accounts to hail users?
- What is the minimum import API needed for both Gmail importer and Cloudflare
  Worker bridge?
- What does export look like if hail stores provider-backed state locally?
