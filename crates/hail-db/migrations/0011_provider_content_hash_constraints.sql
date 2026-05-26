CREATE TABLE provider_message_mappings_new (
  id                    INTEGER PRIMARY KEY,
  provider_account_id   INTEGER NOT NULL REFERENCES provider_accounts(id) ON DELETE CASCADE,
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

INSERT INTO provider_message_mappings_new (
  id,
  provider_account_id,
  provider_message_id,
  provider_thread_id,
  provider_history_id,
  rfc822_message_id,
  content_sha256,
  jmap_email_id,
  jmap_thread_id,
  jmap_mailbox_ids_json,
  import_status,
  imported_at,
  last_seen_at,
  error_class,
  error_message,
  created_at,
  updated_at
)
SELECT
  id,
  provider_account_id,
  provider_message_id,
  provider_thread_id,
  provider_history_id,
  rfc822_message_id,
  content_sha256,
  jmap_email_id,
  jmap_thread_id,
  jmap_mailbox_ids_json,
  import_status,
  imported_at,
  last_seen_at,
  error_class,
  error_message,
  created_at,
  updated_at
FROM provider_message_mappings;

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
