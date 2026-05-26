-- Provider import account and mapping foundation.
-- OAuth/token crypto implementation lands in follow-up tasks; this migration only
-- creates durable sidecar state and idempotency keys.

CREATE TABLE provider_accounts (
  id                              INTEGER PRIMARY KEY,
  user_id                         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  jmap_account_id                 TEXT NOT NULL,
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
    sync_status IN ('disabled','initial_sync','active','error','revoked','disconnected')
  ),
  backfill_cursor_json            TEXT,
  last_sync_attempted_at          TEXT,
  last_sync_succeeded_at          TEXT,
  last_error_class                TEXT,
  last_error_message              TEXT,
  disconnected_at                 TEXT,
  revoked_at                      TEXT,
  created_at                      TEXT NOT NULL,
  updated_at                      TEXT NOT NULL,
  CHECK (
    refresh_token_enc IS NOT NULL
    OR refresh_token_ref IS NOT NULL
    OR sync_status IN ('disabled','revoked','disconnected')
  ),
  UNIQUE (user_id, provider_kind, provider_account_id)
);
CREATE INDEX idx_provider_accounts_user ON provider_accounts(user_id);
CREATE INDEX idx_provider_accounts_status ON provider_accounts(sync_status);
CREATE INDEX idx_provider_accounts_provider_email ON provider_accounts(provider_kind, provider_email);

CREATE TABLE provider_message_mappings (
  id                    INTEGER PRIMARY KEY,
  provider_account_id   INTEGER NOT NULL REFERENCES provider_accounts(id) ON DELETE CASCADE,
  provider_message_id   TEXT NOT NULL,
  provider_thread_id    TEXT,
  provider_history_id   TEXT,
  rfc822_message_id     TEXT,
  content_sha256        BLOB,
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

CREATE TABLE provider_sync_events (
  id                    INTEGER PRIMARY KEY,
  provider_account_id   INTEGER NOT NULL REFERENCES provider_accounts(id) ON DELETE CASCADE,
  event_type            TEXT NOT NULL CHECK (
    event_type IN ('oauth_connected','sync_started','sync_completed','sync_failed','message_imported','message_skipped','token_revoked','disconnected')
  ),
  provider_message_id   TEXT,
  safe_error_class      TEXT,
  safe_error_message    TEXT,
  metadata_json         TEXT,
  created_at            TEXT NOT NULL
);
CREATE INDEX idx_provider_sync_events_account_time
  ON provider_sync_events(provider_account_id, created_at);
CREATE INDEX idx_provider_sync_events_type
  ON provider_sync_events(event_type);
