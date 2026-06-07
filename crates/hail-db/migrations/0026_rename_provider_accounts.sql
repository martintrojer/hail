-- no-transaction
-- Rename provider_accounts to mail_accounts after cache tables landed in 0025.
-- Rebuild every table with foreign keys to provider_accounts so no dangling FK
-- remains after provider_accounts is dropped.

PRAGMA foreign_keys = OFF;

CREATE TABLE mail_accounts (
  id                              INTEGER PRIMARY KEY,
  user_id                         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  jmap_account_id                 TEXT NOT NULL,
  backend_kind                    TEXT NOT NULL CHECK (backend_kind IN ('gmail','jmap')),
  provider_kind                   TEXT NOT NULL CHECK (provider_kind IN ('gmail')),
  provider_account_id             TEXT NOT NULL,
  provider_email                  TEXT NOT NULL,
  display_email                   TEXT,
  granted_scopes_json             TEXT NOT NULL DEFAULT '[]',
  consented_at                    TEXT,
  refresh_token_enc               BLOB,
  refresh_token_ref               TEXT,
  refresh_token_key_id            TEXT,
  cached_access_token_expires_at  TEXT,
  access_token_refreshed_at       TEXT,
  last_profile_history_id         TEXT,
  profile_synced_at               TEXT,
  sync_status                     TEXT NOT NULL CHECK (
    sync_status IN ('disabled','initial_sync','active','error','needs_reauth','paused','revoked','disconnected')
  ),
  backfill_cursor_json            TEXT,
  last_sync_attempted_at          TEXT,
  last_sync_succeeded_at          TEXT,
  next_sync_after                 TEXT,
  sync_backoff_secs               INTEGER,
  last_error_class                TEXT,
  last_error_message              TEXT,
  disconnected_at                 TEXT,
  revoked_at                      TEXT,
  created_at                      TEXT NOT NULL,
  updated_at                      TEXT NOT NULL,
  initial_sync_completed_at       TEXT,
  bidirectional_sync_enabled      INTEGER NOT NULL DEFAULT 0 CHECK (bidirectional_sync_enabled IN (0, 1)),
  CHECK (
    refresh_token_enc IS NOT NULL
    OR refresh_token_ref IS NOT NULL
    OR sync_status IN ('disabled','revoked','disconnected')
  ),
  UNIQUE (user_id, backend_kind, provider_account_id)
);

INSERT INTO mail_accounts (
  id, user_id, jmap_account_id, backend_kind, provider_kind,
  provider_account_id, provider_email, display_email, granted_scopes_json,
  consented_at, refresh_token_enc, refresh_token_ref, refresh_token_key_id,
  cached_access_token_expires_at, access_token_refreshed_at,
  last_profile_history_id, profile_synced_at, sync_status,
  backfill_cursor_json, last_sync_attempted_at, last_sync_succeeded_at,
  next_sync_after, sync_backoff_secs, last_error_class, last_error_message,
  disconnected_at, revoked_at, created_at, updated_at,
  initial_sync_completed_at, bidirectional_sync_enabled
)
SELECT
  id, user_id, jmap_account_id, 'gmail', provider_kind,
  provider_account_id, provider_email, display_email, granted_scopes_json,
  consented_at, refresh_token_enc, refresh_token_ref, refresh_token_key_id,
  cached_access_token_expires_at, access_token_refreshed_at,
  last_profile_history_id, profile_synced_at, sync_status,
  backfill_cursor_json, last_sync_attempted_at, last_sync_succeeded_at,
  next_sync_after, sync_backoff_secs, last_error_class, last_error_message,
  disconnected_at, revoked_at, created_at, updated_at,
  initial_sync_completed_at, bidirectional_sync_enabled
FROM provider_accounts;

CREATE INDEX idx_mail_accounts_user ON mail_accounts(user_id);
CREATE INDEX idx_mail_accounts_status ON mail_accounts(sync_status);
CREATE INDEX idx_mail_accounts_next_sync ON mail_accounts(backend_kind, sync_status, next_sync_after);
CREATE INDEX idx_mail_accounts_provider_email ON mail_accounts(backend_kind, provider_email);
CREATE INDEX idx_mail_accounts_initial_sync_completed
  ON mail_accounts(backend_kind, initial_sync_completed_at);

CREATE TRIGGER mail_accounts_refresh_token_storage_insert
BEFORE INSERT ON mail_accounts
FOR EACH ROW
WHEN NEW.sync_status IN ('initial_sync', 'active', 'error')
  AND NOT (
    NEW.refresh_token_ref IS NULL
    AND NEW.refresh_token_enc IS NOT NULL
    AND length(NEW.refresh_token_enc) >= 29
  )
BEGIN
  SELECT RAISE(ABORT, 'active mail_accounts require encrypted refresh_token_enc and no refresh_token_ref');
END;

CREATE TRIGGER mail_accounts_refresh_token_storage_update
BEFORE UPDATE OF sync_status, refresh_token_enc, refresh_token_ref ON mail_accounts
FOR EACH ROW
WHEN NEW.sync_status IN ('initial_sync', 'active', 'error')
  AND NOT (
    NEW.refresh_token_ref IS NULL
    AND NEW.refresh_token_enc IS NOT NULL
    AND length(NEW.refresh_token_enc) >= 29
  )
BEGIN
  SELECT RAISE(ABORT, 'active mail_accounts require encrypted refresh_token_enc and no refresh_token_ref');
END;

CREATE TRIGGER mail_accounts_json_state_insert
BEFORE INSERT ON mail_accounts
FOR EACH ROW
WHEN NOT json_valid(NEW.granted_scopes_json)
  OR (NEW.backfill_cursor_json IS NOT NULL AND NOT json_valid(NEW.backfill_cursor_json))
BEGIN
  SELECT RAISE(ABORT, 'mail_accounts JSON state fields must be valid JSON');
END;

CREATE TRIGGER mail_accounts_json_state_update
BEFORE UPDATE OF granted_scopes_json, backfill_cursor_json ON mail_accounts
FOR EACH ROW
WHEN NOT json_valid(NEW.granted_scopes_json)
  OR (NEW.backfill_cursor_json IS NOT NULL AND NOT json_valid(NEW.backfill_cursor_json))
BEGIN
  SELECT RAISE(ABORT, 'mail_accounts JSON state fields must be valid JSON');
END;

CREATE TABLE provider_message_mappings_new (
  id                    INTEGER PRIMARY KEY,
  provider_account_id   INTEGER NOT NULL REFERENCES mail_accounts(id) ON DELETE CASCADE,
  provider_message_id   TEXT NOT NULL,
  provider_thread_id    TEXT,
  provider_history_id   TEXT,
  rfc822_message_id     TEXT,
  content_sha256        BLOB CHECK (
    content_sha256 IS NULL OR (typeof(content_sha256) = 'blob' AND length(content_sha256) = 32)
  ),
  jmap_email_id         TEXT,
  jmap_thread_id        TEXT,
  jmap_mailbox_ids_json TEXT,
  import_status         TEXT NOT NULL CHECK (
    import_status IN ('pending','imported','duplicate','skipped','failed')
  ),
  imported_at           TEXT,
  last_seen_at          TEXT,
  error_class           TEXT,
  error_message         TEXT,
  created_at            TEXT NOT NULL,
  updated_at            TEXT NOT NULL,
  UNIQUE (provider_account_id, provider_message_id)
);
INSERT INTO provider_message_mappings_new
SELECT * FROM provider_message_mappings;
DROP TABLE provider_message_mappings;
ALTER TABLE provider_message_mappings_new RENAME TO provider_message_mappings;
CREATE INDEX idx_provider_message_mappings_thread
  ON provider_message_mappings(provider_account_id, provider_thread_id);
CREATE INDEX idx_provider_message_mappings_rfc822
  ON provider_message_mappings(provider_account_id, rfc822_message_id)
  WHERE rfc822_message_id IS NOT NULL;
CREATE INDEX idx_provider_message_mappings_jmap_email
  ON provider_message_mappings(provider_account_id, jmap_email_id)
  WHERE jmap_email_id IS NOT NULL;
CREATE INDEX idx_provider_message_mappings_status
  ON provider_message_mappings(provider_account_id, import_status);
CREATE INDEX idx_provider_message_mappings_content_sha256
  ON provider_message_mappings(provider_account_id, content_sha256)
  WHERE content_sha256 IS NOT NULL;
CREATE TRIGGER provider_message_mappings_json_state_insert
BEFORE INSERT ON provider_message_mappings
FOR EACH ROW
WHEN NEW.jmap_mailbox_ids_json IS NOT NULL
  AND NOT json_valid(NEW.jmap_mailbox_ids_json)
BEGIN
  SELECT RAISE(ABORT, 'provider_message_mappings.jmap_mailbox_ids_json must be valid JSON');
END;
CREATE TRIGGER provider_message_mappings_json_state_update
BEFORE UPDATE OF jmap_mailbox_ids_json ON provider_message_mappings
FOR EACH ROW
WHEN NEW.jmap_mailbox_ids_json IS NOT NULL
  AND NOT json_valid(NEW.jmap_mailbox_ids_json)
BEGIN
  SELECT RAISE(ABORT, 'provider_message_mappings.jmap_mailbox_ids_json must be valid JSON');
END;

CREATE TABLE provider_sync_events_new (
  id                    INTEGER PRIMARY KEY,
  provider_account_id   INTEGER NOT NULL REFERENCES mail_accounts(id) ON DELETE CASCADE,
  user_id               INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  operation_kind        TEXT NOT NULL CHECK (
    operation_kind IN ('oauth','sync','message_import','message_skip','outbound_send','retry','failure','token','disconnect')
  ),
  event_type            TEXT NOT NULL CHECK (
    event_type IN ('oauth_connected','sync_started','sync_completed','sync_paused','initial_sync_aborted','sync_failed','message_imported','message_skipped','message_retry_scheduled','message_failed','sent_via_provider','token_revoked','disconnected')
  ),
  provider_message_id   TEXT,
  result_status         TEXT NOT NULL CHECK (
    result_status IN ('started','succeeded','skipped','retrying','failed','info')
  ),
  safe_error_code       TEXT,
  safe_error_class      TEXT,
  safe_error_message    TEXT,
  metadata_json         TEXT,
  created_at            TEXT NOT NULL
);
INSERT INTO provider_sync_events_new
SELECT * FROM provider_sync_events;
DROP TABLE provider_sync_events;
ALTER TABLE provider_sync_events_new RENAME TO provider_sync_events;
CREATE INDEX idx_provider_sync_events_account_time
  ON provider_sync_events(provider_account_id, created_at);
CREATE INDEX idx_provider_sync_events_type
  ON provider_sync_events(event_type);
CREATE INDEX idx_provider_sync_events_user_account_time
  ON provider_sync_events(user_id, provider_account_id, created_at);
CREATE INDEX idx_provider_sync_events_account_result
  ON provider_sync_events(provider_account_id, result_status, created_at);
CREATE TRIGGER provider_sync_events_account_user_insert
BEFORE INSERT ON provider_sync_events
FOR EACH ROW
WHEN (SELECT user_id FROM mail_accounts WHERE id = NEW.provider_account_id) IS NOT NEW.user_id
BEGIN
  SELECT RAISE(ABORT, 'provider_sync_events user_id must match mail_accounts.user_id');
END;
CREATE TRIGGER provider_sync_events_account_user_update
BEFORE UPDATE OF provider_account_id, user_id ON provider_sync_events
FOR EACH ROW
WHEN (SELECT user_id FROM mail_accounts WHERE id = NEW.provider_account_id) IS NOT NEW.user_id
BEGIN
  SELECT RAISE(ABORT, 'provider_sync_events user_id must match mail_accounts.user_id');
END;
CREATE TRIGGER provider_sync_events_json_state_insert
BEFORE INSERT ON provider_sync_events
FOR EACH ROW
WHEN NEW.metadata_json IS NOT NULL
  AND NOT json_valid(NEW.metadata_json)
BEGIN
  SELECT RAISE(ABORT, 'provider_sync_events.metadata_json must be valid JSON');
END;
CREATE TRIGGER provider_sync_events_json_state_update
BEFORE UPDATE OF metadata_json ON provider_sync_events
FOR EACH ROW
WHEN NEW.metadata_json IS NOT NULL
  AND NOT json_valid(NEW.metadata_json)
BEGIN
  SELECT RAISE(ABORT, 'provider_sync_events.metadata_json must be valid JSON');
END;

CREATE TABLE provider_outbound_changes_new (
  id                    INTEGER PRIMARY KEY,
  provider_account_id   INTEGER NOT NULL REFERENCES mail_accounts(id) ON DELETE CASCADE,
  jmap_email_id         TEXT NOT NULL,
  change_type           TEXT NOT NULL CHECK (change_type IN ('read','unread','label_add','label_remove','trash','untrash')),
  payload_json          TEXT NOT NULL CHECK (json_valid(payload_json)),
  created_at            TEXT NOT NULL,
  applied_at            TEXT,
  attempt_count         INTEGER NOT NULL DEFAULT 0,
  last_error            TEXT
);
INSERT INTO provider_outbound_changes_new
SELECT * FROM provider_outbound_changes;
DROP TABLE provider_outbound_changes;
ALTER TABLE provider_outbound_changes_new RENAME TO provider_outbound_changes;
CREATE INDEX idx_provider_outbound_changes_pending
  ON provider_outbound_changes(provider_account_id, applied_at)
  WHERE applied_at IS NULL;
CREATE INDEX idx_provider_outbound_changes_email_created
  ON provider_outbound_changes(provider_account_id, jmap_email_id, created_at);

DROP TRIGGER messages_ai;
DROP TRIGGER messages_ad;
DROP TRIGGER messages_au;
DROP TABLE messages_fts;

CREATE TABLE messages_new (
  id              INTEGER PRIMARY KEY,
  account_id      INTEGER NOT NULL REFERENCES mail_accounts(id)
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
INSERT INTO messages_new
SELECT * FROM messages;
DROP TABLE messages;
ALTER TABLE messages_new RENAME TO messages;
CREATE INDEX idx_messages_thread     ON messages(account_id, thread_id);
CREATE INDEX idx_messages_received   ON messages(account_id, internal_date DESC);
CREATE INDEX idx_messages_from       ON messages(account_id, from_addr);
CREATE INDEX idx_messages_lru        ON messages(account_id, pinned, accessed_at)
                                       WHERE body_blob_id IS NOT NULL;
CREATE VIRTUAL TABLE messages_fts USING fts5(
  from_addr, subject, body_text,
  content='messages', content_rowid='id'
);
INSERT INTO messages_fts(rowid, from_addr, subject, body_text)
SELECT id, from_addr, subject, body_text FROM messages;
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

CREATE TABLE cache_policy_new (
  account_id        INTEGER PRIMARY KEY REFERENCES mail_accounts(id),
  mode              TEXT    NOT NULL CHECK (mode IN ('off','bounded','full')),
  keep_days         INTEGER,
  keep_max_msgs     INTEGER,
  keep_max_bytes    INTEGER,
  backfill          TEXT    NOT NULL CHECK (backfill IN ('off','incremental')),
  updated_at        TEXT    NOT NULL
);
INSERT INTO cache_policy_new
SELECT * FROM cache_policy;
DROP TABLE cache_policy;
ALTER TABLE cache_policy_new RENAME TO cache_policy;

CREATE TABLE outbound_changes_new (
  id              INTEGER PRIMARY KEY,
  account_id      INTEGER NOT NULL REFERENCES mail_accounts(id)
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
INSERT INTO outbound_changes_new
SELECT * FROM outbound_changes;
DROP TABLE outbound_changes;
ALTER TABLE outbound_changes_new RENAME TO outbound_changes;
CREATE INDEX idx_outbound_pending ON outbound_changes(account_id, applied_at)
  WHERE applied_at IS NULL;

DROP TABLE provider_accounts;

PRAGMA foreign_key_check;
PRAGMA foreign_keys = ON;
