# UI shadcn refresh design

This document records the agreed direction for the hail UI refresh. It is the
source of truth for the `ui-shadcn-refresh` mu track and related wave tasks.

## Product direction

Hail should stop trying to be a close visual clone of HEY. Instead, it should
bring HEY-like mail concepts to a more traditional, compact, self-hosted mail
client layout.

The target is:

```text
HEY concepts + shadcn app language + compact traditional mail layout
```

This should distinguish hail rather than leave it as a less-polished copy of
HEY's large-box visual language.

## Design goals

- Adopt shadcn/Radix as the coherent component and layout language.
- Use a compact, traditional mail-client layout.
- Move primary navigation into a collapsible/hideable left sidebar.
- Keep a top toolbar for search, compose, account/theme/status actions.
- Preserve light and dark mode.
- Keep hail's product concepts: Imbox, Feed, Paper Trail, Screener, Set Aside,
  Reply Later, Bubble Up, labels, provider import, keyboard-first operation.
- Make labels and future dense navigation feel native, not bolted on.

## Non-goals

- Do not keep chasing HEY's spacious large-card visual clone.
- Do not hand-roll menus, dropdowns, popovers, dialogs, command palettes,
  checkboxes, buttons, badges, or sheets when shadcn/Radix provides a primitive.
- Do not migrate every screen in one unreviewable rewrite.
- Do not sacrifice keyboard accessibility.
- Do not break mail HTML rendering by applying generic prose/table borders to
  message bodies.

## Component principle: shadcn first

Default to shadcn/Radix primitives for every UI building block.

Use shadcn-style primitives for:

- `Button`
- `Input`
- `Textarea`
- `Card`
- `Badge`
- `Separator`
- `DropdownMenu`
- `Dialog`
- `AlertDialog`
- `Tooltip`
- `Popover`
- `Command`
- `Sheet`
- `ScrollArea`
- `Skeleton`
- `Alert`
- `Checkbox`
- `Select`
- `Switch`
- `Collapsible`
- `Sidebar`
- `Table` only when table semantics are real

Anti-patterns:

- custom absolute-position dropdowns/popups when Radix can do it;
- bespoke button variants outside shared shadcn `buttonVariants`;
- duplicated one-off pills where `Badge` fits;
- route-local card/input/dropdown styles that duplicate primitives;
- custom menu keyboard navigation where Radix already handles it;
- big custom state cards where `Alert`, `Skeleton`, or shadcn `Card` is enough.

If a task must hand-roll a control, it should add a note explaining why shadcn
was not suitable.

## Token strategy

Use a full shadcn token reset while preserving light/dark mode.

Adopt shadcn CSS variables such as:

```text
--background
--foreground
--card
--card-foreground
--popover
--popover-foreground
--primary
--primary-foreground
--secondary
--secondary-foreground
--muted
--muted-foreground
--accent
--accent-foreground
--destructive
--destructive-foreground
--border
--input
--ring
--radius
```

Also include sidebar-specific variables if using the shadcn sidebar pattern.

Rules:

- `.dark` must define dark-mode equivalents.
- The existing theme toggle remains, but it should switch the shadcn variable
  set rather than the current ad hoc token set.
- Existing hail-specific tokens can be temporarily mapped during migration, but
  the end state should be shadcn tokens.
- Use shadcn neutral palette as the base.
- Hail logo/copy may provide product personality, but layout/color should not
  depend on bespoke HEY-like styling.

## Density and layout language

Default density is compact, similar to shadcn demos and admin/app tooling.

This is intentionally the opposite of HEY's large box language.

Rules:

- Smaller paddings by default.
- Tighter row heights.
- More content above the fold.
- Subtle borders/separators instead of large background panels.
- Restrained shadows.
- Page headers should be compact and functional.
- Actions should live in toolbars, dropdown menus, command menus, or sidebars.
- Use `size="sm"` controls where appropriate.
- Use `Badge` for labels/status.
- Use `Card` sparingly, only for real grouping.
- Lists should feel like productive mail-client lists, not marketing cards.
- Reading/composer surfaces may preserve readable line length, but the shell and
  controls should still use the compact language.

## Target app structure

Desktop:

```text
+------------------------------------------------------------+
| top toolbar: search, compose, account/theme/status         |
+----------------------+-------------------------------------+
| left sidebar         | main content                        |
| - Imbox              | mail list / thread / settings       |
| - Feed               |                                     |
| - Paper Trail        |                                     |
| - Screener           |                                     |
| - Set Aside          |                                     |
| - Reply Later        |                                     |
| - Bubble Up          |                                     |
| - Labels tree        |                                     |
| - Admin/settings     |                                     |
+----------------------+-------------------------------------+
```

Mobile:

- Sidebar becomes a shadcn `Sheet`.
- Top toolbar keeps menu/search/compose entry points.
- Primary actions remain reachable without the centered-logo dropdown.

## Navigation

Move the current centered logo dropdown menu into the left sidebar.

Sidebar includes:

- Imbox
- Feed
- Paper Trail
- Screener
- Set Aside
- Reply Later
- Bubble Up
- Drafts
- Spam
- Trash
- Archive
- All Files
- Workflows
- Provider Accounts
- Admin where permitted
- Labels expandable tree

Sidebar behavior:

- collapsible/hideable on desktop;
- sheet/drawer on mobile;
- keyboard accessible;
- visually compact;
- label tree should use `Collapsible`/sidebar primitives.

## Route-specific direction

### Mail list

- Compact rows.
- Checkbox/select column for multiselect.
- Sender/subject/preview in dense layout.
- Badges for labels/status.
- Batch action toolbar with shadcn buttons/dropdowns.
- Preserve keyboard navigation and existing batch actions.

### Thread view

- Compact thread header and toolbar.
- Message action menu uses shadcn `DropdownMenu`.
- Bubble Up action uses shadcn dropdown/popover primitives.
- Remote-image notice uses `Alert` or similar primitive.
- Notes use shadcn card/alert-style primitives.
- Mail HTML body remains isolated; do not apply generic shadcn table/prose
  styling inside message HTML that damages real email layout.

### Labels

Labels should be the first new-native shadcn feature:

- sidebar label tree with `Collapsible`;
- `/labels` management page;
- `/labels/:id` mail view;
- label picker via `Command`;
- batch label assignment using the same picker;
- chips via `Badge`.

See `docs/labels-design.md` for label semantics.

### Search

- Use compact form controls.
- Label filter uses the label tree/select pattern.
- Query, mailbox, and label filters compose with AND semantics.

### Screener

- Compact sender list and detail/preview areas.
- Routing dropdowns use shadcn primitives.
- Avoid large custom panels per sender unless the panel groups real content.

### Provider accounts/admin/settings

- Use shadcn `Card`, `Table`, `Badge`, `Alert`, `Progress`, `Button`, and form
  primitives.
- Prefer dense operational layouts over large marketing-style cards.

### Composer

- Compact form layout.
- Use shadcn inputs/textareas/buttons/selects/dialogs.
- Toolbar/actions should be consistent with the rest of the app.

## Migration waves

The refresh should be rolled out in gated waves with human smoke between waves.
Do not move to the next visible wave until the human smoke task for the current
wave is complete or explicitly deferred by the operator.

### Wave 0: shadcn foundation

Install/configure shadcn, Radix primitives, `cn`, token reset, light/dark mode,
and contribution rules.

Human gate: `human-smoke-ui-foundation`.

### Wave 1: shell/sidebar/topbar

Replace current app shell with compact sidebar + top toolbar.

Human gate: `human-smoke-ui-shell`.

### Wave 2: mail list vertical slice

Migrate Imbox/list rows and batch action bar to compact shadcn language, then
propagate to other mail list views.

Human gate: `human-smoke-ui-mail-list`.

### Wave 3: thread view vertical slice

Migrate thread toolbar/actions/notes/remote image notice to shadcn while
preserving isolated email HTML rendering.

Human gate: `human-smoke-ui-thread`.

### Wave 4: labels UI

Implement labels using the new shadcn layout language.

Human gate: `human-smoke-ui-labels`.

### Wave 5: remaining route consolidation

Migrate Screener, Composer, Provider Accounts, Admin, Workflows, All Files,
Scheduled Sends, Pile views, and common state/loading/error components.

Human gate: `human-smoke-ui-full-app`.

### Wave 6: review gates

Code and test review for UI refresh. Findings become mu tasks.

## Human smoke expectations

Each wave's human smoke task should ask the operator to try the app in the
local stack and report notes. The agent responsible must triage every note into
one of:

- a new mu task;
- a deferral with evidence;
- an explicit rejection/no-action note with reason.

This follows the human-smoke rule in `AGENTS.md`.

## Verification expectations

Every wave should keep these passing as applicable:

```bash
RUSTFLAGS="-D warnings" cargo build --workspace
cd webapp && npm run build && npm run lint && npm test -- --run
```

Targeted route/component tests should be added per wave. For visual changes,
human smoke is required before proceeding to the next wave.
