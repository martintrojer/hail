-- Add provider outbound status/event values used by Gmail SMTP XOAUTH2 send.
-- SQLite cannot ALTER CHECK constraints directly. This migration updates the
-- stored CREATE TABLE SQL text; the on-disk layout is unchanged.

PRAGMA writable_schema = ON;

UPDATE sqlite_schema
SET sql = replace(
  sql,
  "sync_status IN ('disabled','initial_sync','active','error','paused','revoked','disconnected')",
  "sync_status IN ('disabled','initial_sync','active','error','needs_reauth','paused','revoked','disconnected')"
)
WHERE type = 'table'
  AND name = 'provider_accounts'
  AND sql LIKE "%sync_status IN ('disabled','initial_sync','active','error','paused','revoked','disconnected')%";

UPDATE sqlite_schema
SET sql = replace(
  sql,
  "event_type IN ('oauth_connected','sync_started','sync_completed','sync_paused','initial_sync_aborted','sync_failed','message_imported','message_skipped','message_retry_scheduled','message_failed','token_revoked','disconnected')",
  "event_type IN ('oauth_connected','sync_started','sync_completed','sync_paused','initial_sync_aborted','sync_failed','message_imported','message_skipped','message_retry_scheduled','message_failed','sent_via_provider','token_revoked','disconnected')"
)
WHERE type = 'table'
  AND name = 'provider_sync_events'
  AND sql LIKE "%event_type IN ('oauth_connected','sync_started','sync_completed','sync_paused','initial_sync_aborted','sync_failed','message_imported','message_skipped','message_retry_scheduled','message_failed','token_revoked','disconnected')%";

UPDATE sqlite_schema
SET sql = replace(
  sql,
  "operation_kind IN ('oauth','sync','message_import','message_skip','retry','failure','token','disconnect')",
  "operation_kind IN ('oauth','sync','message_import','message_skip','outbound_send','retry','failure','token','disconnect')"
)
WHERE type = 'table'
  AND name = 'provider_sync_events'
  AND sql LIKE "%operation_kind IN ('oauth','sync','message_import','message_skip','retry','failure','token','disconnect')%";

PRAGMA writable_schema = OFF;
PRAGMA schema_version = 21;
