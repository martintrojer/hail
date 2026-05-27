# Labels design

This document records the agreed design for hail labels. It is operator/product
visible design, not an implementation checklist. Live implementation tracking
lives in mu under `labels-feature` and its blockers.

## Goals

- Add HEY-style labels to hail.
- Let users create, rename, delete, and assign labels to mail.
- Support label-specific mail views.
- Support label filtering in search.
- Import existing user-created Gmail labels into hail in provider-import mode.

## Non-goals

- Do not sync hail label changes back to Gmail or any provider.
- Do not request Gmail write/modify scopes for labels.
- Do not implement true hierarchical label semantics in v1.
- Do not import Gmail system/category labels as hail labels.

## Core semantics

Labels are **local, thread-level tags** in hail.

A label applies to a whole thread. If any imported message in a thread has a
user-created Gmail label, the local hail thread gets the corresponding hail
label.

Hail-owned labels and Gmail labels are intentionally one-way:

```text
Gmail/provider user labels --> hail labels
hail labels                -/-> Gmail/provider
```

Hail never writes label changes back to Gmail. Creating, renaming, deleting, or
assigning a label in hail only changes hail state.

Provider import is deliberately simple and non-reconciling:

- Gmail labels are imported when observed on messages.
- New Gmail labels are imported when later sync sees them.
- If a local hail label is deleted but Gmail still has that label on later
  imported mail, hail recreates the local label.
- If a Gmail label is removed from a message, hail does not remove the local
  thread label as a reconciliation step.
- If a Gmail label is deleted in Gmail, hail does not delete the local label.

## Gmail label scope

Import only user-created Gmail labels.

Skip Gmail system/category/state labels, including but not limited to:

- `INBOX`
- `SENT`
- `TRASH`
- `SPAM`
- `DRAFT`
- `UNREAD`
- `STARRED`
- `IMPORTANT`
- any `CATEGORY_*` label

The importer should use Gmail label metadata rather than guessing solely from
message `labelIds` whenever possible. Gmail `type=user` labels are importable;
`type=system` labels are not.

## Nested labels

Gmail nested labels are represented as names containing `/`, for example:

```text
Work
Work/Receipts
Family/Kids/School
```

Hail stores each full path as one flat label. There is no implied parent/child
membership.

Rules:

- `Work/Receipts` does not imply `Work`.
- If Gmail sends only `Work/Receipts`, hail creates only `Work/Receipts`.
- If Gmail sends both `Work` and `Work/Receipts`, hail creates two labels.
- A local user may rename/delete either independently.

UI may display labels as a tree by splitting names on `/`, but API/DB semantics
remain flat concrete labels.

## Label display

Label chips on thread rows and thread pages should be compact:

- visible chip text: leaf segment only, e.g. `Receipts`
- tooltip/title: full path, e.g. `Work/Receipts`

Label management and navigation should show the indented tree so full context is
visible.

Label view headers should show the full label path or a breadcrumb-style path,
e.g. `Work / Receipts`.

## Data model

Expected sidecar tables:

```text
labels
  id
  user_id
  name                 -- full path, e.g. Work/Receipts
  normalized_name      -- uniqueness key derived from full path
  source               -- manual | gmail
  provider_kind        -- nullable, e.g. gmail
  provider_label_id    -- nullable
  color                -- nullable
  created_at
  updated_at

thread_labels
  user_id
  thread_id
  label_id
  created_at
```

Recommended constraints:

- unique `(user_id, normalized_name)`
- unique `(user_id, provider_kind, provider_label_id)` where provider metadata
  is present
- unique `(user_id, thread_id, label_id)`
- deleting a label deletes all `thread_labels` assignments for that label

Normalization should be deterministic and conservative:

- trim whitespace around the full label name and around path segments
- collapse repeated internal whitespace if useful for UX
- compare case-insensitively for duplicate detection
- preserve the display `name` as entered/imported, except for trimming
- reject empty names and empty path segments such as `Work//Receipts`

## Merge/upsert rules

When importing a Gmail label:

1. If `(user_id, provider_kind='gmail', provider_label_id)` exists, update that
   label's name/normalized name if needed.
2. Otherwise, if `(user_id, normalized_name)` exists, reuse that local label and
   attach provider metadata if the provider metadata slot is empty.
3. Otherwise, create a new label with `source='gmail'`.

When the user manually creates a label with the same normalized name as a later
Gmail import, the Gmail import should reuse the existing label rather than
creating a duplicate.

If a local label was deleted, the rows are gone; a later Gmail import can create
it again.

## API shape

Label management:

```text
GET    /api/labels
POST   /api/labels
PATCH  /api/labels/{id}
DELETE /api/labels/{id}
```

Thread assignment:

```text
POST   /api/threads/{thread_id}/labels/{label_id}
DELETE /api/threads/{thread_id}/labels/{label_id}
POST   /api/threads/{thread_id}/labels      -- inline create/upsert by label_name
```

Batch assignment:

```text
POST /api/threads/labels
```

Payload should accept either `label_id` or `label_name` plus selected
`thread_ids`. Inline create uses `label_name`.

Label mail view:

```text
GET /api/labels/{id}/threads?cursor=&limit=
```

Search filter:

```text
GET /api/views/search?q=...&mailbox=...&label_id=...
```

`label_id` omitted or set to an agreed all value means no label filter.

Search filters combine with **AND**:

```text
query text match
AND optional mailbox filter
AND optional label filter
```

## API response shape

Labels should include path data for easy tree rendering:

```json
{
  "id": 12,
  "name": "Work/Receipts",
  "path_segments": ["Work", "Receipts"],
  "source": "gmail",
  "color": null,
  "thread_count": 8
}
```

Thread/list rows should include labels, for example:

```json
{
  "thread_id": "t1",
  "subject": "Invoice",
  "labels": [
    {
      "id": 12,
      "name": "Work/Receipts",
      "leaf_name": "Receipts",
      "path_segments": ["Work", "Receipts"]
    }
  ]
}
```

## UI shape

Main navigation:

- Add an expandable **Labels** section.
- Display labels as an indented tree based on `/` path segments.
- Clicking a concrete label opens `/labels/:label_id`.

Label management page `/labels`:

- Show labels as an indented tree.
- Create labels.
- Rename labels.
- Delete labels.
- Deleting a label removes all thread assignments.

Label view `/labels/:id`:

- Shows all threads assigned to that label.
- Uses normal mail list row behavior.
- Header shows full path/breadcrumb.

Thread/list row chips:

- Show leaf segment only.
- Full path in `title`/tooltip.

Thread action popup:

- Add/remove labels.
- Searchable label picker.
- Inline create when no exact normalized-name match exists.
- Typing `/` creates nested path labels, e.g. `Work/Receipts`.

Multiselect:

- Add batch action: `Label`.
- Opens same searchable picker.
- Allows inline create.
- Applies selected label to all selected threads.

Search page:

- Add label filter dropdown/tree.
- Default to `All`.
- Combine with query and mailbox filter using AND semantics.

## Gmail import behavior

The Gmail client needs label metadata support. Import should know which Gmail
label ids are user labels and what names they map to.

Historical import:

- Load/cache Gmail label metadata for the provider account.
- For each fetched Gmail message, inspect `labelIds`.
- Filter to user-created labels.
- Upsert each hail label.
- Assign each label to the local Stalwart thread after the message has a local
  thread id.

Incremental import:

- Apply the same label import behavior to newly imported Gmail messages.
- New Gmail labels encountered later are created when seen.

No Gmail write scope is needed.

## Testing requirements

DB/API tests should cover:

- create/list/rename/delete labels
- duplicate normalized-name rejection/upsert behavior
- nested label path validation and path segment output
- delete removes assignments
- thread assignment/remove
- inline create assignment
- batch assignment
- label view filtering
- search label filter AND mailbox/query

Gmail import tests should cover:

- user-created labels import
- system/category labels ignored
- nested Gmail labels import as flat paths
- deleted local label reappears on later import
- manual label with same normalized name merges with Gmail import
- thread-level rollup when any message in thread has a label
- no Gmail modify/write scope or API call is required

SPA tests should cover:

- label management tree create/rename/delete
- nav label tree
- label view
- thread/list chips leaf display and full-path tooltip
- thread label picker add/remove/inline create
- multiselect label assignment
- search label filter default All and AND behavior

Human smoke should cover:

- create a local nested label
- assign from thread popup
- assign selected threads via multiselect
- view `/labels/:id`
- filter search by label and mailbox together
- import a Gmail user label and verify it appears locally
- delete a Gmail-imported local label and observe it can reappear when imported again
