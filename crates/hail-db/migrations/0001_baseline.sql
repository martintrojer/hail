-- hail v1 baseline schema (design.md §6.2).
-- The `_sqlx_migrations` table is created and managed by `sqlx::migrate!()`.

-- Users mapped 1:1 to Stalwart accounts.
CREATE TABLE users (
  id              INTEGER PRIMARY KEY,
  email           TEXT NOT NULL UNIQUE,
  jmap_account_id TEXT NOT NULL,
  display_name    TEXT,
  is_admin        INTEGER NOT NULL DEFAULT 0,
  created_at      TEXT NOT NULL
);

-- Encrypted JMAP token; one row per active login session.
CREATE TABLE sessions (
  id              TEXT PRIMARY KEY,          -- opaque cookie value (256-bit)
  user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  jmap_token_enc  BLOB NOT NULL,             -- AES-GCM, server key
  user_agent      TEXT,
  expires_at      TEXT NOT NULL,
  created_at      TEXT NOT NULL,
  last_used_at    TEXT NOT NULL
);
CREATE INDEX idx_sessions_user ON sessions(user_id);

-- Screener decisions, one row per (user, sender).
CREATE TABLE screener_rules (
  user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  sender_address  TEXT NOT NULL,             -- normalized lowercase
  decision        TEXT NOT NULL CHECK (decision IN ('allow','deny','pending')),
  classify_as     TEXT     CHECK (classify_as IN ('imbox','feed','papertrail')),
  decided_at      TEXT,
  first_seen_at   TEXT NOT NULL,
  PRIMARY KEY (user_id, sender_address)
);

-- Per-contact private notes (markdown).
CREATE TABLE contact_notes (
  user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  address         TEXT NOT NULL,             -- normalized lowercase
  markdown        TEXT NOT NULL,
  updated_at      TEXT NOT NULL,
  PRIMARY KEY (user_id, address)
);

-- Stack ordering for Reply Later and Set Aside.
CREATE TABLE stack_positions (
  user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  stack           TEXT NOT NULL CHECK (stack IN ('reply_later','set_aside')),
  thread_id       TEXT NOT NULL,             -- JMAP thread id
  position        INTEGER NOT NULL,
  added_at        TEXT NOT NULL,
  PRIMARY KEY (user_id, stack, thread_id)
);
CREATE INDEX idx_stack_order ON stack_positions(user_id, stack, position);

-- Scheduled "bubble up" — re-mark a thread unread at surface_at.
CREATE TABLE bubble_ups (
  id              INTEGER PRIMARY KEY,
  user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  thread_id       TEXT NOT NULL,
  surface_at      TEXT NOT NULL,
  fired_at        TEXT,
  created_at      TEXT NOT NULL
);
CREATE INDEX idx_bubble_ups_pending ON bubble_ups(surface_at) WHERE fired_at IS NULL;

-- Scheduled outbound mail.
CREATE TABLE scheduled_sends (
  id              INTEGER PRIMARY KEY,
  user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  draft_email_id  TEXT NOT NULL,             -- JMAP email id of the draft
  send_at         TEXT NOT NULL,
  status          TEXT NOT NULL CHECK (status IN ('pending','sent','cancelled','failed')),
  sent_at         TEXT,
  error           TEXT,
  created_at      TEXT NOT NULL
);
CREATE INDEX idx_scheduled_sends_due ON scheduled_sends(send_at) WHERE status = 'pending';

-- Per-user preferences blob (signature, default classifications, theme, etc.)
CREATE TABLE user_prefs (
  user_id         INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  prefs_json      TEXT NOT NULL DEFAULT '{}'
);

-- Worker resume marker — JMAP state cursor per (user, type).
CREATE TABLE jmap_state (
  user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  type_state      TEXT NOT NULL,             -- 'Email','Thread','Mailbox','EmailSubmission'
  state           TEXT NOT NULL,             -- opaque JMAP state token
  updated_at      TEXT NOT NULL,
  PRIMARY KEY (user_id, type_state)
);

-- Audit log (admin actions, screener decisions, sends). Append-only.
CREATE TABLE audit_log (
  id              INTEGER PRIMARY KEY,
  user_id         INTEGER REFERENCES users(id) ON DELETE SET NULL,
  action          TEXT NOT NULL,
  payload_json    TEXT,
  created_at      TEXT NOT NULL
);
