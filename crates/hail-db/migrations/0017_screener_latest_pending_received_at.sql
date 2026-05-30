ALTER TABLE screener_rules ADD COLUMN latest_pending_received_at TEXT;

CREATE INDEX idx_screener_rules_user_pending_latest
  ON screener_rules(user_id, decision, latest_pending_received_at DESC, sender_address ASC);
