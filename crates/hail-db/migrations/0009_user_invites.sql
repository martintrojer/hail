-- Admin-created, public invite tokens for provisioning users without sharing passwords.
CREATE TABLE user_invites (
  id                  INTEGER PRIMARY KEY,
  email               TEXT NOT NULL,
  display_name        TEXT,
  token_hash          TEXT NOT NULL UNIQUE,
  created_by_user_id  INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  expires_at          TEXT NOT NULL,
  accepted_at         TEXT,
  accepted_user_id    INTEGER REFERENCES users(id) ON DELETE SET NULL,
  created_at          TEXT NOT NULL
);
CREATE INDEX idx_user_invites_token_hash ON user_invites(token_hash);
CREATE INDEX idx_user_invites_email ON user_invites(email);
CREATE INDEX idx_user_invites_pending ON user_invites(expires_at) WHERE accepted_at IS NULL;
