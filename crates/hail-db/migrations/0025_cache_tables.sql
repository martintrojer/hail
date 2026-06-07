-- Cache-side mail metadata tables from docs/hail-architecture.md §Schema additions.
-- TODO(primitive-rename-provider-accounts): these foreign keys intentionally
-- reference provider_accounts(id) until the mail_accounts rename migration
-- reconciles cache-table FKs with the final account table name.

-- One row per known message across all accounts. Metadata-only: cheap and
-- always populated regardless of cache mode (except cache.mode='off').
CREATE TABLE messages (
  id              INTEGER PRIMARY KEY,
  account_id      INTEGER NOT NULL REFERENCES provider_accounts(id)
                                   ON DELETE CASCADE,
  backend_msg_id  TEXT    NOT NULL,
  thread_id       TEXT    NOT NULL,
  internal_date   INTEGER NOT NULL,
  from_addr       TEXT    NOT NULL,
  subject         TEXT    NOT NULL,
  preview         TEXT    NOT NULL,
  size_bytes      INTEGER NOT NULL,
  body_blob_id    TEXT,
  body_text       TEXT,
  inserted_at     TEXT    NOT NULL,
  accessed_at     TEXT    NOT NULL,
  pinned          INTEGER NOT NULL DEFAULT 0,
  UNIQUE (account_id, backend_msg_id)
);
CREATE INDEX idx_messages_thread     ON messages(account_id, thread_id);
CREATE INDEX idx_messages_received   ON messages(account_id, internal_date DESC);
CREATE INDEX idx_messages_from       ON messages(account_id, from_addr);
CREATE INDEX idx_messages_lru        ON messages(account_id, pinned, accessed_at)
                                       WHERE body_blob_id IS NOT NULL;

CREATE TABLE message_keywords (
  message_id  INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  keyword     TEXT    NOT NULL,
  PRIMARY KEY (message_id, keyword)
);
CREATE INDEX idx_message_keywords_keyword ON message_keywords(keyword);

CREATE TABLE attachments (
  id           INTEGER PRIMARY KEY,
  message_id   INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  filename     TEXT    NOT NULL,
  mime_type    TEXT    NOT NULL,
  size_bytes   INTEGER NOT NULL,
  blob_id      TEXT,
  inline       INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_attachments_message ON attachments(message_id);

CREATE VIRTUAL TABLE messages_fts USING fts5(
  from_addr, subject, body_text,
  content='messages', content_rowid='id'
);
CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
  INSERT INTO messages_fts(rowid, from_addr, subject, body_text)
  VALUES (new.id, new.from_addr, new.subject, new.body_text);
END;
CREATE TRIGGER messages_ad AFTER DELETE ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, from_addr, subject, body_text)
  VALUES ('delete', old.id, old.from_addr, old.subject, old.body_text);
END;
CREATE TRIGGER messages_au AFTER UPDATE ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, from_addr, subject, body_text)
  VALUES ('delete', old.id, old.from_addr, old.subject, old.body_text);
  INSERT INTO messages_fts(rowid, from_addr, subject, body_text)
  VALUES (new.id, new.from_addr, new.subject, new.body_text);
END;

CREATE TABLE cache_policy (
  account_id        INTEGER PRIMARY KEY REFERENCES provider_accounts(id),
  mode              TEXT    NOT NULL CHECK (mode IN ('off','bounded','full')),
  keep_days         INTEGER,
  keep_max_msgs     INTEGER,
  keep_max_bytes    INTEGER,
  backfill          TEXT    NOT NULL CHECK (backfill IN ('off','incremental')),
  updated_at        TEXT    NOT NULL
);

CREATE TABLE outbound_changes (
  id              INTEGER PRIMARY KEY,
  account_id      INTEGER NOT NULL REFERENCES provider_accounts(id)
                                   ON DELETE CASCADE,
  backend_msg_id  TEXT    NOT NULL,
  change_type     TEXT    NOT NULL CHECK (change_type IN (
                    'read','unread','keyword_add','keyword_remove',
                    'role_move','trash','untrash','permanent_delete',
                    'send')),
  payload_json    TEXT    NOT NULL CHECK (json_valid(payload_json)),
  created_at      TEXT    NOT NULL,
  applied_at      TEXT,
  attempt_count   INTEGER NOT NULL DEFAULT 0,
  last_error      TEXT
);
CREATE INDEX idx_outbound_pending ON outbound_changes(account_id, applied_at)
  WHERE applied_at IS NULL;
