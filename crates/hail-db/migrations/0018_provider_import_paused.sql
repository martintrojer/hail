-- Add explicit operator-paused provider import state and audit event.
-- SQLite cannot ALTER CHECK constraints directly. This migration updates the
-- stored CREATE TABLE SQL text in sqlite_schema; the on-disk layout is
-- unchanged because both affected columns are TEXT.

PRAGMA writable_schema = ON;

UPDATE sqlite_schema
SET sql = replace(
  sql,
  "sync_status IN ('disabled','initial_sync','active','error','revoked','disconnected')",
  "sync_status IN ('disabled','initial_sync','active','error','paused','revoked','disconnected')"
)
WHERE type = 'table'
  AND name = 'provider_accounts'
  AND sql LIKE "%sync_status IN ('disabled','initial_sync','active','error','revoked','disconnected')%";

UPDATE sqlite_schema
SET sql = replace(
  sql,
  "event_type IN ('oauth_connected','sync_started','sync_completed','sync_failed','message_imported','message_skipped','message_retry_scheduled','message_failed','token_revoked','disconnected')",
  "event_type IN ('oauth_connected','sync_started','sync_completed','sync_paused','sync_failed','message_imported','message_skipped','message_retry_scheduled','message_failed','token_revoked','disconnected')"
)
WHERE type = 'table'
  AND name = 'provider_sync_events'
  AND sql LIKE "%event_type IN ('oauth_connected','sync_started','sync_completed','sync_failed','message_imported','message_skipped','message_retry_scheduled','message_failed','token_revoked','disconnected')%";

PRAGMA writable_schema = OFF;
PRAGMA schema_version = 18;
