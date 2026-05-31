CREATE TABLE provider_outbound_changes (
  id                    INTEGER PRIMARY KEY,
  provider_account_id   INTEGER NOT NULL REFERENCES provider_accounts(id) ON DELETE CASCADE,
  jmap_email_id         TEXT NOT NULL,
  change_type           TEXT NOT NULL CHECK (change_type IN ('read','unread','label_add','label_remove','trash','untrash')),
  payload_json          TEXT NOT NULL CHECK (json_valid(payload_json)),
  created_at            TEXT NOT NULL,
  applied_at            TEXT,
  attempt_count         INTEGER NOT NULL DEFAULT 0,
  last_error            TEXT
);

CREATE INDEX idx_provider_outbound_changes_pending
  ON provider_outbound_changes(provider_account_id, applied_at)
  WHERE applied_at IS NULL;

CREATE INDEX idx_provider_outbound_changes_email_created
  ON provider_outbound_changes(provider_account_id, jmap_email_id, created_at);
