-- User-defined workflow/mail rules.
-- Stored as explicit JSON condition/action blobs for API stability while the
-- worker evaluator evolves independently.
CREATE TABLE workflow_rules (
  id              INTEGER PRIMARY KEY,
  user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name            TEXT NOT NULL,
  enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
  conditions_json TEXT NOT NULL,
  action_json     TEXT NOT NULL,
  created_at      TEXT NOT NULL,
  updated_at      TEXT NOT NULL
);
CREATE INDEX idx_workflow_rules_user ON workflow_rules(user_id, created_at DESC, id DESC);
