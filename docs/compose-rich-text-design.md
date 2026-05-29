# Compose rich-text design

> Status: agreed Option A design for the rich-text composer.
> This document records the boundary between the React editor, the hail API, and
> outbound MIME generation. The broader system map remains in
> [`architecture.md`](./architecture.md), and product roadmap context remains in
> [`design.md`](./design.md).

## 1. Decision

hail will store and send composer content as sanitized HTML.

The previous markdown-centered composer model is replaced going forward:
`ComposeRequest`, `DraftRequest`, and `DraftResponse` carry `body_html` instead
of `body_markdown`. The browser editor treats TipTap HTML as the authoritative
compose document; the server sanitizes that HTML before storing, previewing, or
sending it.

This is Option A from the compose-rich-text planning notes: make HTML the wire
format and storage format for drafts, instead of treating the rich-text editor as
a markdown adapter.

## 2. Frontend editor

The SPA composer uses TipTap/ProseMirror with:

- `StarterKit` for paragraphs, bold, italic, strike, headings, lists,
  blockquote, code, code block, and horizontal rule;
- `Underline` for underlined text;
- `Link` for links;
- `Placeholder` for the empty composer affordance.

The toolbar is built from existing shadcn/ui primitives:

- `Toggle` for active formatting states such as bold, italic, underline, strike,
  blockquote, and list modes;
- `Button` for actions such as send, discard, insert link, and remove link;
- `Separator` for visual grouping;
- `DropdownMenu` for grouped controls such as heading level, more formatting,
  or Send Later options.

Behavioral requirements:

- TipTap's current document HTML is the source of truth for compose and draft
  auto-save payloads. The SPA must not render markdown and must not double-render
  TipTap output through a markdown pipeline.
- Existing composer keyboard shortcuts keep working, including Cmd/Ctrl+Enter to
  send and Esc to close/dismiss the composer.
- Paste handling accepts plain text and browser-sanitized rich HTML, then relies
  on the server sanitizer for the durable allow-list.
- Link editing normalizes through the editor UI, but server-side sanitization is
  authoritative.

## 3. Wire format and draft migration

New API payloads use `body_html`:

```json
{
  "to": ["friend@example.com"],
  "cc": [],
  "bcc": [],
  "subject": "Hello",
  "body_html": "<p>Hello <strong>there</strong>.</p>",
  "attachments": [],
  "send_at": null,
  "in_reply_to": null
}
```

Applies to:

- `ComposeRequest` for immediate send and scheduled send;
- `DraftRequest` for create/update draft auto-save;
- `DraftResponse` for loading a saved draft into the editor.

Legacy compatibility:

- Existing drafts that still contain `body_markdown` must load.
- On first edit/save of a legacy draft, the server converts the markdown to HTML
  with the existing server-side `render_markdown` path, stores the resulting
  `body_html`, and continues with the new HTML flow.
- After migration, `body_html` is authoritative. New clients should not send
  `body_markdown`.

OpenAPI-generated TypeScript types remain the frontend contract; after the API
change, the webapp consumes generated `body_html` fields from
`webapp/src/api/types.ts`.

## 4. Server handling and sanitization

The server owns outbound HTML safety. It accepts editor HTML as user input and
runs it through `sanitize_outgoing_html()` before storing a draft, rendering a
preview, quoting in a reply, or sending.

The sanitizer is an allow-list, conceptually similar to the inbound mail HTML
sanitizer, with outbound-specific rules:

- remove scripts, iframes, objects, embeds, forms, event handler attributes, and
  JavaScript/data URL execution vectors;
- allow ordinary formatting elements produced by the editor, such as paragraphs,
  headings, lists, blockquotes, inline emphasis, code, preformatted code, links,
  and horizontal rules;
- normalize link `href` values to safe protocols only;
- normalize outbound links with safe `rel` and `target` values when they open in
  a new context;
- remove unsupported attributes and styles unless deliberately allowed;
- preserve current remote-image privacy policy. In particular, an outbound draft
  preview must not load remote images.

`sanitize_outgoing_html()` is the durable server boundary. Client-side editor
constraints improve UX but are not a security control.

## 5. MIME generation

On send, `hail-api` remains the sole producer of outbound RFC 5322/MIME content.
The client never supplies raw headers or MIME parts.

Send behavior:

1. receive `body_html` from the request or migrated draft;
2. sanitize with `sanitize_outgoing_html()`;
3. build the HTML body part from sanitized HTML;
4. derive the `text/plain` alternative body from sanitized HTML;
5. attach uploaded blobs through the existing attachment path;
6. submit through the existing JMAP/Stalwart outbound flow.

The plain-text alternative is derived on send so there is one compose source of
truth and no client-side risk of HTML/text divergence.

## 6. Reply quoting

Replies quote previous mail as HTML, not as markdown.

When building a reply draft, the server takes sanitized previous-message HTML and
wraps it in a `<blockquote>` for the editor. Replies must not use the old
markdown convention of prefixing lines with `>`.

The resulting draft body is still passed through the same outgoing sanitizer
before save or send, so quote HTML and newly typed content share one safety
boundary.

## 7. Privacy rules

The rich-text composer does not weaken existing mail privacy behavior:

- inbound message rendering keeps the current remote-image and tracking-pixel
  policy;
- outbound draft preview must not load remote images;
- quoted prior message HTML must already be sanitized and must not reintroduce
  remote trackers;
- the editor may display local blob attachments or inline local previews only
  when they are represented by hail-controlled blob references, not arbitrary
  remote URLs.

Remote-image behavior should match the architecture-level rule that the server
sanitizes rendered mail before the SPA displays it.

## 8. Testing plan

API/server coverage:

- `ComposeRequest` accepts `body_html` and round-trips sanitized HTML through send
  and scheduled-send paths;
- `DraftRequest` and `DraftResponse` round-trip `body_html` through draft
  create/update/load;
- legacy `body_markdown` drafts migrate through server-side `render_markdown` on
  first edit/save;
- sanitizer cases cover script/iframe/event removal, unsafe URL removal, link
  `rel`/`target` normalization, unsupported attribute stripping, and remote image
  handling for preview;
- send derives a sane `text/plain` alternative from sanitized HTML.

SPA coverage:

- TipTap toolbar toggles for bold, italic, underline, strike, headings, lists,
  blockquote, code/code block, and horizontal rule;
- paste behavior for plain text and HTML;
- link insertion, editing, removal, and reload from a saved draft;
- reply quote rendering as `<blockquote>` HTML;
- draft auto-save and reload round-trip with `body_html`;
- Cmd/Ctrl+Enter send and Esc close shortcuts still work.

## 9. Non-goals

- No markdown-to-rich-text editing mode for new drafts.
- No raw MIME editing in the browser.
- No relaxation of inbound or outbound HTML sanitization to preserve arbitrary
  pasted markup.
- No change to the architecture rule that the SPA talks only to `hail-api` and
  never to Stalwart/JMAP directly.
