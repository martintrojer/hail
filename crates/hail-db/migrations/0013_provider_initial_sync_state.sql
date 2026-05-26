ALTER TABLE provider_accounts ADD COLUMN initial_sync_completed_at TEXT;

UPDATE provider_accounts
SET initial_sync_completed_at = COALESCE(last_sync_succeeded_at, updated_at)
WHERE provider_kind = 'gmail'
  AND sync_status = 'active'
  AND initial_sync_completed_at IS NULL;

CREATE INDEX idx_provider_accounts_initial_sync_completed
  ON provider_accounts(provider_kind, initial_sync_completed_at);
