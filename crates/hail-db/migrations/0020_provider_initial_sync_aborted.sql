-- Add provider initial-sync aborted audit event for quota/rate-limit terminal imports.
-- SQLite cannot ALTER CHECK constraints directly. This updates only the stored
-- CREATE TABLE SQL text; provider_sync_events layout is unchanged.

PRAGMA writable_schema = ON;

UPDATE sqlite_schema
SET sql = replace(
  sql,
  "event_type IN ('oauth_connected','sync_started','sync_completed','sync_paused','sync_failed','message_imported','message_skipped','message_retry_scheduled','message_failed','token_revoked','disconnected')",
  "event_type IN ('oauth_connected','sync_started','sync_completed','sync_paused','initial_sync_aborted','sync_failed','message_imported','message_skipped','message_retry_scheduled','message_failed','token_revoked','disconnected')"
)
WHERE type = 'table'
  AND name = 'provider_sync_events'
  AND sql LIKE "%event_type IN ('oauth_connected','sync_started','sync_completed','sync_paused','sync_failed','message_imported','message_skipped','message_retry_scheduled','message_failed','token_revoked','disconnected')%";

PRAGMA writable_schema = OFF;
PRAGMA schema_version = 20;
