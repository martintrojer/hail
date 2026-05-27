-- Local thread-level labels. Labels are flat full paths (for example
-- "Work/Receipts"); there is no hierarchy table and no tombstone state.
CREATE TABLE labels (
  id                INTEGER PRIMARY KEY,
  user_id           INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name              TEXT NOT NULL CHECK (
    length(trim(name)) > 0
    AND name = trim(name)
    AND name NOT LIKE '/%'
    AND name NOT LIKE '%/'
    AND name NOT LIKE '%//%'
  ),
  normalized_name   TEXT NOT NULL CHECK (
    length(trim(normalized_name)) > 0
    AND normalized_name = trim(normalized_name)
    AND normalized_name NOT LIKE '/%'
    AND normalized_name NOT LIKE '%/'
    AND normalized_name NOT LIKE '%//%'
  ),
  source            TEXT NOT NULL CHECK (source IN ('manual','gmail')),
  provider_kind     TEXT CHECK (provider_kind IS NULL OR provider_kind IN ('gmail')),
  provider_label_id TEXT,
  color             TEXT,
  created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  CHECK ((provider_kind IS NULL) = (provider_label_id IS NULL))
);

CREATE UNIQUE INDEX idx_labels_user_id ON labels(user_id, id);
CREATE UNIQUE INDEX idx_labels_user_normalized_name ON labels(user_id, normalized_name);
CREATE UNIQUE INDEX idx_labels_provider_identity
  ON labels(user_id, provider_kind, provider_label_id)
  WHERE provider_kind IS NOT NULL AND provider_label_id IS NOT NULL;
CREATE INDEX idx_labels_user_name ON labels(user_id, name);

CREATE TABLE thread_labels (
  user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  thread_id  TEXT NOT NULL CHECK (length(trim(thread_id)) > 0),
  label_id   INTEGER NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  PRIMARY KEY (user_id, thread_id, label_id),
  FOREIGN KEY (user_id, label_id) REFERENCES labels(user_id, id) ON DELETE CASCADE
);

CREATE INDEX idx_thread_labels_label
  ON thread_labels(user_id, label_id, created_at DESC, thread_id);
