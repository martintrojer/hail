-- Short-lived server-side undo tokens for destructive user actions.
CREATE TABLE undo_actions (
  id              TEXT PRIMARY KEY,          -- opaque 256-bit hex token
  user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  action          TEXT NOT NULL,
  payload_json    TEXT NOT NULL,
  expires_at      TEXT NOT NULL,
  used_at         TEXT,
  created_at      TEXT NOT NULL
);
CREATE INDEX idx_undo_actions_user_live ON undo_actions(user_id, expires_at) WHERE used_at IS NULL;
