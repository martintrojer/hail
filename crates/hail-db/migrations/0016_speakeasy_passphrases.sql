-- Speakeasy is a per-user monthly Screener bypass passphrase. It is not a
-- sender allow-list and not route management; the current phrase lets only
-- matching incoming messages bypass sender approval.
CREATE TABLE speakeasy_passphrases (
  user_id             INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  passphrase          TEXT NOT NULL CHECK (length(trim(passphrase)) >= 16 AND passphrase = trim(passphrase)),
  period              TEXT NOT NULL CHECK (
    period GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]'
    AND substr(period, 6, 2) BETWEEN '01' AND '12'
  ),
  rotates_at          TEXT NOT NULL,
  generated_at        TEXT NOT NULL,
  manually_rotated_at TEXT,
  updated_at          TEXT NOT NULL
);

CREATE INDEX idx_speakeasy_passphrases_rotates_at
  ON speakeasy_passphrases(rotates_at);
