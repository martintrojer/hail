-- Make send-later authorization lifecycle explicit.  v1 does not store a
-- separate long-lived outbound credential, so each scheduled send records the
-- browser session that accepted it and the session expiry.  The worker may use
-- that referenced encrypted token until expiry; after expiry the row becomes a
-- visible auth_required failure instead of pending forever.
CREATE TABLE scheduled_sends_new (
  id                    INTEGER PRIMARY KEY,
  user_id               INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  draft_email_id        TEXT NOT NULL,
  send_at               TEXT NOT NULL,
  status                  TEXT NOT NULL CHECK (status IN ('pending','processing','sent','cancelled','failed','auth_required')),
  auth_session_id         TEXT REFERENCES sessions(id) ON DELETE SET NULL,
  auth_session_expires_at TEXT,
  claimed_at              TEXT,
  sent_at               TEXT,
  error                 TEXT,
  created_at            TEXT NOT NULL
);

INSERT INTO scheduled_sends_new (
  id,
  user_id,
  draft_email_id,
  send_at,
  status,
  auth_session_id,
  auth_session_expires_at,
  claimed_at,
  sent_at,
  error,
  created_at
)
SELECT
  id,
  user_id,
  draft_email_id,
  send_at,
  CASE WHEN status = 'failed' AND error = 'auth_required' THEN 'auth_required' ELSE status END,
  NULL,
  NULL,
  claimed_at,
  sent_at,
  error,
  created_at
FROM scheduled_sends;

DROP TABLE scheduled_sends;
ALTER TABLE scheduled_sends_new RENAME TO scheduled_sends;
CREATE INDEX idx_scheduled_sends_due ON scheduled_sends(send_at) WHERE status = 'pending';
