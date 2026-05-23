-- Add an in-progress claim state so scheduled-send workers claim rows before
-- performing non-idempotent JMAP EmailSubmission calls.
CREATE TABLE scheduled_sends_new (
  id              INTEGER PRIMARY KEY,
  user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  draft_email_id  TEXT NOT NULL,
  send_at         TEXT NOT NULL,
  status          TEXT NOT NULL CHECK (status IN ('pending','processing','sent','cancelled','failed')),
  claimed_at      TEXT,
  sent_at         TEXT,
  error           TEXT,
  created_at      TEXT NOT NULL
);

INSERT INTO scheduled_sends_new (
  id,
  user_id,
  draft_email_id,
  send_at,
  status,
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
  status,
  NULL,
  sent_at,
  error,
  created_at
FROM scheduled_sends;

DROP TABLE scheduled_sends;
ALTER TABLE scheduled_sends_new RENAME TO scheduled_sends;
CREATE INDEX idx_scheduled_sends_due ON scheduled_sends(send_at) WHERE status = 'pending';
