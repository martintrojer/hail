-- Durable product event outbox used to bridge hail-worker -> hail-api.
--
-- hail-worker runs in a separate process from hail-api, so the in-process
-- WebSocket broadcast channel cannot be used directly. Workers append
-- type-only invalidation events here; hail-api polls new rows and rebroadcasts
-- them to connected `/api/ws` clients. `payload_json` is reserved for later
-- per-thread/per-scheduled-send payloads; v1 clients treat these events as
-- cache-invalidation hints and refetch current state.
CREATE TABLE app_events (
  id            INTEGER PRIMARY KEY,
  user_id       INTEGER REFERENCES users(id) ON DELETE CASCADE,
  event_type    TEXT NOT NULL CHECK (event_type IN (
                  'imbox.new',
                  'feed.new',
                  'papertrail.new',
                  'screener.pending',
                  'thread.updated',
                  'thread.removed',
                  'bubble.fired',
                  'send.completed',
                  'send.failed'
                )),
  payload_json  TEXT NOT NULL DEFAULT '{}',
  created_at    TEXT NOT NULL
);
CREATE INDEX idx_app_events_id ON app_events(id);
CREATE INDEX idx_app_events_user_id ON app_events(user_id, id);
