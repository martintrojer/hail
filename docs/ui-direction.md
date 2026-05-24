# HEY-inspired UI direction for hail

This document captures the look, feel, and interaction model hail's SPA should
move toward. It is derived directly from operator-provided HEY screenshots in
`design-reference/hey/`. It is not a literal clone; it is the direction that
mu task `ui-design-direction-hey-inspired` is committing to before any
redesign work starts.

If you change something here, also update `ui-design-direction-hey-inspired`
in mu so the redesign branch references the current direction.

## 1. Overall posture

- Calm, paper-like canvas. The background is a warm off-white / cream, never
  a pure `#ffffff`. The whole app feels like a sheet of good stationery.
- Single-column, centered reading layout. Everything - lists, threads,
  compose - lives in one generous center column (~680-740px) with large
  quiet margins on both sides. There is **no permanent left sidebar** and
  **no three-pane Gmail layout**. The empty margins are part of the design;
  they create breathing room.
- Strong, friendly typography. Section titles are very large (~2.5rem+),
  body text is generous (~1.05-1.1rem), line height is relaxed (1.5-1.6).
  The product reads like a magazine, not a database.
- Almost no borders or boxes. Hierarchy comes entirely from font size,
  weight, whitespace, and subtle color. No card shadows, no rounded-corner
  card outlines, no colored section backgrounds. The only lines are faint
  1px hairline dividers between list rows.
- Restrained palette. Mostly warm black on cream. Two accent hues:
  a blue (HEY uses a medium blue for interactive elements, links, and
  primary buttons) and a warm yellow/gold for badges like "New". No
  gradients anywhere.
- Density: deliberately low. Imbox rows are tall with generous vertical
  padding (~20-28px per side). The feeling is of handling one piece of
  mail at a time, not scanning a feed.
- No visual chrome. No app-bar background color, no colored header bands,
  no icon-heavy toolbars. The UI gets out of the way of the content.

## 2. Layout (from 01-imbox, 02-main-menu-dropdown, 15-imbox-with-screener-notification)

The layout is radically simple compared to most mail clients:

- **No sidebar.** Navigation is hidden behind a dropdown menu triggered by
  clicking the HEY logo / app name in the top-left corner. The screen is
  almost entirely content.
- Top strip (not a traditional app bar):
  - Far left: app logo / wordmark ("HEY" / for us: "hail"). Clicking it
    opens the main menu dropdown.
  - Center-left: the current section title in very large bold type, e.g.
    "Imbox". This is the dominant visual element on the page.
  - Far right: a sparse cluster of small icons - search (magnifying glass),
    screener notification indicator, and the user's avatar circle. These
    are small and understated, not a toolbar.
- The main menu dropdown (visible in `02-main-menu-dropdown.png`):
  - Appears as a clean white/cream card with subtle shadow, anchored to the
    top-left logo area.
  - Items listed vertically, each with a small icon on the left and label
    on the right: Imbox, The Feed, Paper Trail, Set Aside, Reply Later,
    Previously Seen, All, Contacts, Compose.
  - Below the nav items: account section showing user name/email and a
    "Sign Out" link.
  - The dropdown has rounded corners and a very gentle drop shadow - the
    only place in the app where a shadow appears.
  - When the menu is open, the content behind dims slightly.
- Below the top strip, the center column begins immediately with content.
  There is no sub-header, no breadcrumb, no tab bar.
- The Imbox screener notification (visible in `15-imbox-with-screener-notification.png`):
  - Appears as a full-width banner at the very top of the Imbox content area,
    above the first mail row.
  - Warm cream/yellow-tinted background, slightly distinct from the page.
  - Left side: small icon + text like "1 new person wants to email you".
  - Right side: a link/button to "See who's waiting" or "Screen".
  - The banner is dismissible and disappears after all senders are triaged.
  - Tone is conversational, not urgent. No alarm colors.
- Thread reading replaces the list in-place. Clicking a row transitions to
  the thread view using the same center column. A small back link or breadcrumb
  appears at the top to return to the list. There is no split/side-by-side
  reading pane.

## 3. Typography

From close inspection of all screenshots:

- HEY uses a single sans-serif family throughout (likely a custom cut of a
  geometric humanist sans). There is **no serif**. The warmth comes from
  weight contrast and generous sizing, not from a serif face.
- For hail, use one high-quality sans-serif:
  - Primary: `"Inter", system-ui, sans-serif` - widely available, has
    good weight range, feels modern and warm at large sizes.
  - If Inter feels too generic, consider `"Plus Jakarta Sans"` which has
    a friendlier geometric personality at display sizes.
- Scale (approximate, calibrated from screenshots):
  - Page section title ("Imbox", "The Feed"): **2rem-2.5rem**, **bold**
    (font-weight 700). Not semibold - genuinely bold.
  - Sender name in mail list: **1.0-1.05rem**, **semibold** (600).
  - Subject line in mail list: **0.95-1.0rem**, **regular** (400),
    ink-secondary color.
  - Preview snippet / timestamp: **0.85-0.9rem**, regular, muted color.
  - Body text in thread reading: **1.0-1.1rem**, regular, line-height
    1.55-1.65.
  - UI chrome (menu items, buttons, labels): **0.875rem**, medium (500).
  - Badges / pills ("New"): **0.7-0.75rem**, semibold, uppercase with
    wide tracking.
- Letter-spacing:
  - Titles: default or very slightly tighter (-0.01em).
  - Badges/pills: wide (+0.05-0.08em), all-caps.
  - Everything else: default.
- Line height:
  - Titles: 1.2-1.3 (tight, since they're large).
  - Body/reading: 1.55-1.65 (generous, easy to read).
  - List rows: 1.3-1.4.
- No italic text anywhere in the chrome. Italic only appears in user
  content (email body).

## 4. Color tokens

Light theme first; dark theme is a follow-up task.

Derived from pixel-sampling the screenshots:

- **bg-page**: `#faf8f5` - the main app background. Warm off-white, not
  yellow-tinted, not pure white. Slightly cooler than old-paper cream.
- **bg-surface**: `#ffffff` - used for the menu dropdown card, compose
  area, and modal-like surfaces. Pure white stands out against the
  warm page background.
- **bg-banner**: `#fef9ee` - the screener notification banner and
  similar inline callouts. A slightly warmer/yellower tint than bg-page.
- **bg-hover**: `#f3f0eb` - list row hover state. A subtle darkening of
  bg-page, like a shadow of your hand on paper.
- **bg-selected**: `#eae6df` - the currently-selected/active list row.
  Noticeably darker than hover but still warm.
- **ink-primary**: `#1a1a1a` - near-black for sender names, titles,
  body text. Not pure `#000`.
- **ink-secondary**: `#6b6560` - warm medium grey for subjects, snippets,
  timestamps, secondary labels.
- **ink-tertiary**: `#9b9590` - lighter warm grey for placeholder text,
  disabled states, very quiet metadata.
- **accent-blue**: `#4a77e5` - the primary interactive color. Used for:
  links, the "Compose" button fill, active nav item indicator, the
  focused mini-composer's send button, and the screener "Yes" action.
  In HEY this is a confident medium blue, not electric/neon.
- **accent-blue-hover**: `#3a63cc` - slightly darker for hover states on
  blue interactive elements.
- **accent-yellow**: `#f0c030` - warm gold/yellow for "New" / unread
  badges. Used sparingly as a pill background with ink-primary text.
- **accent-red**: `#d64545` - used only for destructive actions (trash,
  block sender) in confirmations. Never used for badges or attention.
- **border-hairline**: `#e8e3db` - the single 1px divider between list
  rows. Very faint, warm-tinted. Should nearly disappear on the page.
- **border-menu**: `#e0dbd4` - slightly more visible border for the
  dropdown menu card and popup cards.

Key differences from current hail palette:
- Replace all `slate-*` (cool blue-grey) with these warm neutrals.
- Replace `sky-*` accent with `accent-blue`.
- No dark mode backgrounds. The app is light, always.
- Shadows are used only on the dropdown menu and per-message popup cards,
  and are very subtle (`shadow-md` equivalent with warm tint).

## 5. Imbox list pattern

From `01-imbox.png`, `13-power-through-imbox.png`,
`15-imbox-with-screener-notification.png`:

The Imbox is a simple vertical list of threads in the center column.

- **Row anatomy** (each row = one thread):
  - Top line: **sender name** (semibold, ink-primary, ~1.0rem) on the left.
    Relative timestamp on the far right (ink-tertiary, ~0.85rem).
  - Second line: **subject** (regular weight, ink-secondary, ~0.95rem).
  - Optional third line: **snippet** / preview of the latest message body
    (regular, ink-tertiary, ~0.85rem, single line, truncated with ellipsis).
  - Optional: a small **"New"** pill/badge (accent-yellow background,
    ink-primary text, rounded-full, ~0.7rem uppercase) next to the sender
    name or timestamp when the thread has unread messages.

- **Spacing**:
  - Vertical padding inside each row: ~16-20px top and bottom.
  - A 1px `border-hairline` divider between rows. No left/right borders.
  - No horizontal padding beyond the column's own margins.

- **Interaction states**:
  - Hover: bg-hover (subtle warm darkening). Instant, no transition.
  - Selected/focused (keyboard): bg-selected with a 3px accent-blue left
    border/rail to indicate focus. The left rail is the only strong color
    in the list.
  - Click: navigates to the thread view (replaces list in-place).

- **No checkboxes** by default. The list is purely for reading and triage.
  Bulk operations, if added later, appear via a mode toggle in the header.

- **Unread vs read**: unread rows have the sender name in bold (700) and
  possibly the "New" pill. Read rows use semibold (600) sender and no pill.
  No background color difference between read/unread rows.

- **Power through** (`13-power-through-imbox.png`): a focused triage mode
  where you see one thread at a time with action buttons (Keep in Imbox /
  Move to Feed / Move to Paper Trail / Set Aside). This replaces the list
  temporarily. Implement as a separate view mode triggered by a button in
  the Imbox header area.

## 6. Thread reading view

From `03-reading-email.png`, `05-thread-message-popup.png`,
`06-thread-add-note.png`, `07-focus-reply-mini-composer.png`,
`14-email-with-notes.png`:

The thread view replaces the list in the center column. Same width, same
margins. A back link at the top returns to the list.

- **Header area**:
  - Subject line in large bold text (~1.5-1.75rem, ink-primary).
  - Below: sender name and email, timestamp, and a small "..." overflow
    button for thread-level actions.

- **Message stack**: messages are vertically stacked, oldest at top.
  Each message block:
  - **Sender line**: avatar circle (small, ~28-32px) on the left, sender
    name (semibold) and relative timestamp (ink-tertiary) on the right.
  - **Per-message action button**: a tiny circle icon ("···" or a
    message-circle icon, visible in `05-thread-message-popup.png`) floated
    to the right of the sender line. Clicking it opens the per-message
    popup.
  - **Body**: rendered HTML/text below the sender line, using body
    typography (1.0-1.1rem, line-height 1.6, ink-primary). Images are
    blocked/proxied by default.
  - Between messages: a hairline divider, or just generous whitespace
    (~24-32px).

- **Per-message popup** (`05-thread-message-popup.png`):
  - A floating card (bg-surface, border-menu, subtle shadow) anchored
    near the action button.
  - Menu items, each a text row with a small icon:
    - Reply / Reply All
    - Forward
    - Set Aside
    - Reply Later
    - Bubble Up (opens time submenu)
    - Move to → Imbox / Feed / Paper Trail
    - Add a Note
    - Mark as spam
    - Trash
  - Items are ~0.875rem, medium weight, generous vertical padding (~10px).
  - No nested submenus except for Bubble Up's time picker.

- **Inline notes** (`06-thread-add-note.png`, `14-email-with-notes.png`):
  - Notes appear inline in the message stack, visually distinct:
    - A 3-4px accent-yellow or accent-blue left border/rail.
    - Slightly tinted background (bg-banner or similar).
    - A small "Note" label or icon at the top.
    - The note text in body typography.
    - Author name + timestamp below in ink-tertiary.
  - Adding a note: clicking "Add a Note" in the popup opens an inline
    text area at that point in the thread. Simple text input, no rich
    formatting. A "Save" button and "Cancel" link.

- **Focused mini composer** (`07-focus-reply-mini-composer.png`):
  - Appears at the bottom of the thread, inside the same column.
  - A simple text area with the sender's name as context ("Reply to
    Jane..." placeholder).
  - Below the text area: a row with "Send" button (accent-blue fill,
    white text) on the left and secondary actions (attach, send later)
    as small quiet icons/links on the right.
  - No modal, no popup, no separate route. The composer is part of the
    thread reading flow.
  - The mini composer can expand vertically as you type. It does not
    have a fixed height.

## 7. Compose

From `04-compose-email.png`:

Compose opens as a full center-column page, not a modal or popup.

- **Layout**: same center column as everything else. The compose form
  fills the column with generous whitespace.
- **Fields**, top to bottom:
  - **To**: a clean text input with contact autocompletion. No box/border
    decoration - just a label ("To") in ink-tertiary and an underline or
    very faint bottom border.
  - **Subject**: same minimal style as To.
  - **Cc / Bcc**: hidden by default. A small "Cc / Bcc" link next to the
    To field reveals them.
  - **Body**: a large, borderless text area that takes up the remaining
    column height. Placeholder text in ink-tertiary ("Write your email...").
    The text area uses body typography (1.0-1.1rem, line-height 1.6).
- **Toolbar**: minimal, appears below or inside the body area:
  - Formatting controls (bold, italic, link) shown as small quiet icons,
    possibly hidden behind a formatting toggle.
  - Attachment (paperclip icon) as a single small icon.
  - No heavy toolbar ribbon.
- **Actions** at the bottom:
  - **Send** button: accent-blue fill, white text, ~0.875rem semibold.
    This is the primary action.
  - **Send Later**: a small quiet text link or secondary button next to
    Send. Clicking opens a time picker (preset options + custom).
  - **Discard**: a quiet text link in ink-tertiary, far from Send to
    avoid accidents.
- **Autosave**: happens silently. A tiny muted line of text ("Draft saved")
  appears briefly below the body area, then fades. No toast, no flash.
- **Back/cancel**: a back arrow or "Cancel" link at the top, same as
  thread view's back link.

## 8. The Feed

From `08-the-feed.png`:

The Feed is for newsletters and automated/marketing mail that you want to
read but not in your Imbox.

- Same center column, same warm background.
- Section title: "The Feed" in the same large bold style.
- Below the title: a brief blurb in ink-secondary ("Newsletter and
  notification email lives here" or similar).
- **Row style**: slightly richer than Imbox rows. Each row shows:
  - Sender brand/name (semibold, larger than Imbox rows).
  - Subject line.
  - A 2-3 line preview snippet (more generous than Imbox's single line).
  - Timestamp on the right.
  - Optionally: a small sender logo/icon if available.
- Rows are taller than Imbox rows to accommodate the longer preview.
- Interaction: clicking opens the thread in the same center column.
- The Feed does not have a "power through" mode — it's for browsing at
  leisure.

## 8b. The Screener

From `15-imbox-with-screener-notification.png` and `16-screener-page.png`:

The Screener is how you decide whether new senders get access to your
Imbox. It's a gatekeeper, not a spam filter.

- **Screener notification** (on the Imbox page):
  - A banner at the top of the Imbox content, above the first mail row.
  - Warm bg-banner background, slightly tinted compared to bg-page.
  - Left: a small icon (bell or person+question) and conversational text:
    "1 new person wants to email you" / "3 new senders are waiting".
  - Right: a link/button "Screen them" that navigates to the Screener
    page.
  - The banner uses body typography. Not loud, not alarmed.
  - The banner disappears when no senders are pending.

- **Screener page** (`16-screener-page.png`):
  - Section title: "The Screener" in the standard large bold style.
  - Below the title: a brief blurb in ink-secondary explaining the concept
    ("New senders end up here. Decide if they get in.").
  - **Sender cards**: each pending sender is a distinct block/card:
    - Sender name in semibold + email address in ink-secondary below it.
    - The subject line of the email they sent, as context.
    - A 1-2 line preview/snippet of the message body.
    - **Two primary actions**, prominently displayed:
      - "**Yes**" / "Let them in" — accent-blue button. Approving routes
        the sender's current and future mail into Imbox (or Feed/Paper
        Trail based on a secondary choice).
      - "**No**" / "Block" — outlined/muted button in ink-secondary.
        Never red. Blocking is a quiet action, not an angry one.
    - **Secondary routing**: after clicking "Yes", a subtle follow-up
      allows choosing where the sender's mail goes: Imbox, The Feed, or
      Paper Trail. Default is Imbox.
  - Cards are separated by generous whitespace (~20-24px), no borders
    between them.
  - Each card has a very subtle bg-surface or bg-banner background to
    distinguish it from the page, with rounded corners (~8px).
  - **Empty state**: when all senders are screened, show a calm centered
    message: "All clear. No one new is waiting." in ink-secondary.
    No illustrations.

- **Screener indicator in the top strip**: the screener icon in the
  top-right cluster shows a small count badge (accent-yellow or
  accent-blue circle with white number) when senders are pending.
  When none are pending, the icon is ink-tertiary and has no badge.

- **Screener decisions are immediate**: approving or blocking takes effect
  right away with a brief undo toast. No multi-step confirmation modal.
  Future mail from that sender is routed automatically.

## 9. Paper Trail

From `09-paper-trail.png`:

Paper Trail is for transactional/receipt mail — order confirmations,
shipping notifications, password resets, etc.

- Same center column, same warm background, same title pattern.
- **Row style**: denser than Imbox. Each row is shorter:
  - Sender/brand on the left (semibold, ~0.95rem).
  - Subject on the same line or wrapping below (regular, ink-secondary).
  - Timestamp on the far right (ink-tertiary, ~0.8rem).
  - No preview snippet — the subject is usually enough context for
    receipts.
- Rows are ~40-48px tall (vs ~64-80px for Imbox).
- Hairline dividers between rows.
- No badges or pills. Paper Trail content is inherently read-and-forget.
- Clicking a row opens the thread in the center column as usual.

## 10. Set Aside

From `10-set-aside.png`:

Set Aside is a personal holding area for threads you want to come back to.

- Same center column. Title: "Set Aside".
- Row style matches Imbox (sender, subject, snippet, timestamp).
- Each row may show a small "set aside" icon or the date it was set aside.
- A row-level action to "move back to Imbox" or "release" is available
  via the per-message popup or a small inline icon on hover.
- **Empty state**: centered text, "Nothing set aside. When you set a
  thread aside, it’ll wait here." in ink-secondary. Calm and encouraging.

## 11. Reply Later / Bubble Up

From `11-bubble-up-submenu.png` and `12-bubble-up-page.png`:

- **Reply Later**: a section listing threads you’ve marked to reply to.
  Same row style as Imbox. Each row may show the date/time it was
  deferred. Clicking opens the thread with the mini composer ready.

- **Bubble Up**:
  - Accessed from the per-message popup. Choosing "Bubble Up" opens a
    time submenu with preset options:
    - "Later today"
    - "Tomorrow morning"
    - "This weekend"
    - "Next week"
    - "Pick a date…" (opens a date picker)
  - The submenu is a small floating card (bg-surface, border-menu,
    subtle shadow), similar in style to the per-message popup.
  - Each option is a text row (~0.875rem, medium weight), with generous
    padding.

- **Bubble Up page** (`12-bubble-up-page.png`):
  - Section title: "Bubble Up".
  - Lists threads scheduled to bubble up, with the bubble time visible
    as a secondary line in ink-secondary ("Bubbles up tomorrow at 9am").
  - Row style matches Imbox but with the bubble time replacing the
    received timestamp.
  - Each row has a "cancel bubble" action on hover or in the popup.

- **The Pile** (optional future feature):
  - In HEY, the Pile is a small persistent panel anchored bottom-right.
  - For hail v1, implement piles as regular section pages (Set Aside,
    Reply Later) rather than a floating panel. The panel treatment
    can be a follow-up.

## 11b. Notification banners and toasts

- **Screener banner** (described in §8b): warm bg-banner row at the top
  of Imbox content. Same column width as the list.
- **Undo toast**: appears as a slim bar at the bottom-center of the
  viewport (not inside the column). Background: ink-primary or a dark
  warm tone. Text: white, ~0.875rem. An "Undo" link in accent-blue or
  white-underlined. Auto-dismisses after ~5 seconds.
- **Send-later confirmation**: same toast style as undo. "Scheduled for
  tomorrow at 9am. Undo."
- At most one toast visible at a time. No stacking.
- Banners (in-column, like screener) are distinct from toasts (viewport-
  anchored). Don’t mix the two patterns.

## 12. Empty/transitional states

- **Empty Imbox**: centered in the column, vertically middle-ish:
  - Large text (~1.25rem, semibold, ink-primary): "You’re all caught up."
  - A small line below in ink-secondary: "New mail will appear here."
  - No illustrations, no confetti, no emoji.
- **Empty Feed**: "Nothing in The Feed yet."
- **Empty Paper Trail**: "No receipts yet."
- **Empty Set Aside**: "Nothing set aside."
- **Empty Screener**: "All clear. No one new is waiting."
- **Loading**: very quiet. Show the section title and a single line of
  muted text ("Loading…") in ink-tertiary. No skeleton bars, no
  spinners. If loading takes more than ~1s, show the text. Under 1s,
  show nothing (flash-free).
- **Error states**: same calm style. "Something went wrong. Try again."
  with a "Retry" link in accent-blue. No red backgrounds or alarm icons.

## 13. Motion and interaction

- Almost no motion. Page transitions are instant cuts, not slides or fades.
- Hover state changes are instant (no `transition` on background color).
- The only acceptable animations:
  - Undo toast: fade in on appear, fade out on dismiss.
  - Dropdown menu: instant appear, or a very fast (~100ms) fade/scale.
  - Loading text: appears after a 1s delay, no animation.
- No bouncy animations, no slide-in panels, no parallax.
- Focus rings: use a 2px accent-blue outline for keyboard focus, with
  `outline-offset: 2px`. Visible only on `:focus-visible`, not on click.

## 14. Iconography

- Use **Lucide** icons throughout. They are consistent, have the right
  weight (1.5px stroke at 24px), and match the quiet aesthetic.
- Icon size in the top strip: 20px.
- Icon size in menus and inline actions: 16-18px.
- Icon color: ink-secondary by default, ink-primary on hover.
- Key icons needed:
  - Search: `Search` (magnifying glass)
  - Screener: `UserPlus` or `ShieldQuestion`
  - Menu: the app logo/wordmark, not a hamburger icon
  - Per-message actions: `MoreHorizontal` (three dots)
  - Reply: `Reply`
  - Forward: `Forward`
  - Attach: `Paperclip`
  - Compose: `PenSquare` or `Plus`
  - Trash: `Trash2`
  - Set Aside: `Bookmark`
  - Reply Later: `Clock`
  - Bubble Up: `ArrowUpCircle`
  - Note: `StickyNote`
  - Back: `ArrowLeft`

## 15. Voice

- Use HEY-style human language for chrome where possible:
  - "Imbox" (intentional misspelling).
  - "The Feed".
  - "Paper Trail".
  - "Set Aside".
  - "Reply Later".
  - "Bubble Up".
  - "The Screener".
  - "Power through" (for the focused triage mode).
- Error messages, empty states, and labels should sound like a person
  talking, not a system notification. Prefer "No one new is waiting"
  over "0 pending items".
- Button labels are verbs or short phrases: "Let them in", "Screen them",
  "Send", "Save note", "Undo". Not "Submit", "OK", "Confirm".

## 16. Spacing system

Use a consistent 4px grid:
- **4px**: minimum micro-spacing (icon-to-text gap).
- **8px**: padding inside pills/badges.
- **12px**: gap between icon and label in menu items.
- **16px**: row horizontal padding, gap between columns of text.
- **20px**: vertical padding inside list rows.
- **24px**: gap between sections, padding around the center column.
- **32px**: gap above/below the section title.
- **48px+**: vertical margin between major page sections.

The center column max-width should be **720px**, centered with
`margin: 0 auto`. On small screens (<768px) the column fills the
viewport with 16px horizontal padding.

## 17. Responsive behavior

- Single-column layout at all breakpoints. The center column simply
  narrows on smaller screens.
- Top strip remains: logo left, section title center-left, icons right.
  On very small screens the title may need to be smaller (~1.75rem).
- The dropdown menu becomes full-width on mobile (<640px).
- Thread reading and compose use the same single column.
- No breakpoint where a sidebar appears. The layout is always sidebar-free.

## 18. Non-goals

- Do not try to clone HEY pixel for pixel. We take the direction, not
  the exact implementation.
- No dark mode in this first redesign; track as a follow-up task.
- No custom icon font. Use Lucide React package.
- No animation library. CSS transitions only where noted.
- No i18n in v1; English only.
- No per-message emoji reactions or social features.
- No real-time typing indicators or presence.

## 19. Scope of the first redesign pass

The follow-up implementation task
`ui-redesign-hey-inspired-core` should touch:

- `webapp/src/layout/AppShell.tsx`
- `webapp/src/layout/Pile.tsx`
- `webapp/src/routes/MailViewPage.tsx`
- `webapp/src/routes/ThreadPage.tsx`
- `webapp/src/routes/ScreenerPage.tsx`
- the Imbox screener-notification banner (currently in `MailViewPage.tsx`
  or a shared banner component)
- `webapp/src/routes/ComposerPage.tsx`
- `webapp/src/index.css` / Tailwind theme tokens

It should NOT:

- change the API surface;
- change feature behavior unrelated to layout/look.

## 20. Verification

A redesign pass is "done" when:

- `npm run lint` and `npm test -- --run` pass.
- the operator (`human-smoke-ui-redesign`) confirms the redesign feels close
  enough to the HEY-inspired direction documented here.
- updated screenshots are added under `design-reference/hail/` for diffing
  in future redesigns.
