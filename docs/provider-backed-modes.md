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

Use case: hail removes Stalwart from this deployment mode and owns mail-shaped
storage itself. Gmail, Cloudflare Email Routing, or another provider becomes a
delivery/sync edge instead of the local mailbox database.

We identified two versions of this idea. They look similar on a diagram, but the
source-of-truth boundary is different.

### Mode P4a: Hail-native provider replica

The provider remains authoritative for the mailbox, but hail stores full local
copies of messages, bodies, and attachments in a hail-owned store for speed,
privacy-at-rest, local search, and HEY-style product behavior.

```text
Gmail/provider mailbox  ← source of truth for delivery/sent/archive/delete
  ↔ provider API sync
  ↔ hail-owned message + blob replica
  ↔ hail UI/API
```

This is the "local replicated Gmail archive" version. Hail can render and search
from its own store, but destructive actions and sent/draft state still reconcile
back to the provider.

#### Pros

- No Stalwart required.
- Fast local UI and search once synced.
- Local copy gives some independence from provider outages.
- Easier than becoming a complete mail server because provider still handles
  delivery, spam, account security, and outbound reputation.

#### Cons

- Sync correctness becomes central: deletions, archive, labels, sent mail,
  drafts, and provider-side changes must reconcile cleanly.
- The provider can still be the legal/practical source of truth and may retain
  all mail.
- Hail must own MIME parsing, blobs, search, dedupe, and threading for the local
  replica.
- Conflict semantics must be explicit: if Gmail and hail disagree, who wins?

### Mode P4b: Hail-native authoritative store

Hail becomes the authoritative local mailbox/archive. Providers, Cloudflare
Workers, Gmail importers, or future SMTP components are just ingress/egress
adapters.

```text
Provider / Cloudflare / importer / future SMTP edge
  → hail-owned authoritative message + blob store
  → hail UI/API
```

This is a larger product: hail is no longer just a client or Stalwart product
layer. It is the mail store. It must provide durable storage, import/export,
search, threading, attachment handling, sent/draft state, and clear interop
boundaries.

#### Pros

- Maximum product control.
- No dependency on Stalwart's or Gmail's data model for HEY-style workflows.
- Cleanest long-term foundation if hail becomes a standalone personal mail
  workspace rather than a Stalwart UI.

#### Cons

- Reimplements a large amount of mail-server/mail-store functionality.
- Harder to interoperate with standard mail clients unless hail also exposes
  JMAP/IMAP/export surfaces.
- Export, backup, search, attachments, threading, MIME edge cases, duplicate
  detection, and delivery/import semantics become hail responsibilities.
- This should not be treated as a small extension of v1.

### P4 shared responsibilities

Both P4 variants require hail to own more mail-shaped infrastructure than the
current architecture:

- MIME parsing and normalized body extraction;
- raw RFC822 retention policy;
- attachment/blob storage;
- threading and duplicate detection;
- search indexing;
- import/export formats;
- sent/draft/reply state;
- backup/restore of message data, not just hail sidecar state.

## Recommendation

For the current project, keep **Stalwart-first v1** as the mainline.

For v1.2 provider import, hail chooses **Mode P2: provider importer into
Stalwart**. Gmail/provider remains the public mailbox edge, but Stalwart remains
the local source of truth for the hail UI after import. The detailed source of
truth, OAuth, dedupe, scheduler, retry, and Gmail-to-Stalwart flow rules live in
[provider-import-architecture.md](./provider-import-architecture.md).

If we add additional no-public-server support, the lowest-risk order is:

1. **Mode P2: provider importer into Stalwart** — selected for v1.2 provider
   import; preserves the current architecture and gives existing Gmail users a
   practical path.
2. **Mode P3: Cloudflare Email Routing import bridge** — useful for custom-domain
   users without Gmail-as-inbox, once a secure import queue exists.
3. **Mode P1: provider-backed client/cache** — attractive for Gmail users, but it
   requires defining which provider state is authoritative.
4. **Mode P4a: hail-native provider replica** — no Stalwart and fast local
   storage, but the provider remains authoritative and sync conflicts become the
   core problem.
5. **Mode P4b: hail-native authoritative store** — powerful but a separate
   product line or major-version architecture change.

For a single operator who already lives in Gmail and only wants the HEY-style UI,
Mode P1 may be the simplest user experience. For hail's current codebase, Mode
P2 is the more incremental implementation path.

## Open implementation questions

For P2 provider import, these are answered in
[provider-import-architecture.md](./provider-import-architecture.md): Stalwart is
required and remains the local source of truth; Gmail import is one-way in v1.2;
OAuth refresh tokens are encrypted in `hail.db`; raw RFC822 is imported into
Stalwart rather than stored as a hail-native archive. Open questions for other
modes and future versions remain:

- Can Stalwart become optional in a later P1/P4 deployment?
- For non-P2 modes, are Gmail labels mapped to hail views, or are hail views sidecar-only?
- For future bidirectional sync, does deleting/archive in hail mutate Gmail, or only local hail state?
- For non-Stalwart modes, how much raw RFC822/body/attachment data is cached locally?
- How are OAuth refresh tokens encrypted, rotated, and revoked?
- How do multi-user deployments map provider accounts to hail users?
- What is the minimum import API needed for both Gmail importer and Cloudflare
  Worker bridge?
- What does export look like if hail stores provider-backed state outside Stalwart?
- For P4a, which provider actions are mirrored back and which are local-only?
- For P4b, does hail expose JMAP/IMAP/export so users are not trapped in a
  hail-only store?
