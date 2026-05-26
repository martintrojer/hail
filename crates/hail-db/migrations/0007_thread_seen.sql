CREATE TABLE thread_seen (
  user_id   INTEGER NOT NULL REFERENCES users(id),
  thread_id TEXT NOT NULL,
  seen_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  PRIMARY KEY (user_id, thread_id)
);
CREATE INDEX idx_thread_seen_user ON thread_seen(user_id, seen_at DESC);
