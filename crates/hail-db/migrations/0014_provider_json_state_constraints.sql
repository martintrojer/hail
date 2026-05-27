-- Enforce JSON validity for provider state fields that are stored as JSON
-- text. Durable import cursors and account/audit metadata must fail loudly
-- rather than silently defaulting to empty/none semantics when corrupted.

CREATE TEMP TABLE provider_json_state_constraint_check (
  ok INTEGER NOT NULL CHECK (ok = 1)
);
INSERT INTO provider_json_state_constraint_check(ok)
SELECT CASE
  WHEN COUNT(*) = 0 THEN 1
  ELSE 0
END
FROM provider_accounts
WHERE NOT json_valid(granted_scopes_json)
   OR (backfill_cursor_json IS NOT NULL AND NOT json_valid(backfill_cursor_json));
DROP TABLE provider_json_state_constraint_check;

CREATE TEMP TABLE provider_message_json_state_constraint_check (
  ok INTEGER NOT NULL CHECK (ok = 1)
);
INSERT INTO provider_message_json_state_constraint_check(ok)
SELECT CASE
  WHEN COUNT(*) = 0 THEN 1
  ELSE 0
END
FROM provider_message_mappings
WHERE jmap_mailbox_ids_json IS NOT NULL
  AND NOT json_valid(jmap_mailbox_ids_json);
DROP TABLE provider_message_json_state_constraint_check;

CREATE TEMP TABLE provider_audit_json_state_constraint_check (
  ok INTEGER NOT NULL CHECK (ok = 1)
);
INSERT INTO provider_audit_json_state_constraint_check(ok)
SELECT CASE
  WHEN COUNT(*) = 0 THEN 1
  ELSE 0
END
FROM provider_sync_events
WHERE metadata_json IS NOT NULL
  AND NOT json_valid(metadata_json);
DROP TABLE provider_audit_json_state_constraint_check;

CREATE TRIGGER provider_accounts_json_state_insert
BEFORE INSERT ON provider_accounts
FOR EACH ROW
WHEN NOT json_valid(NEW.granted_scopes_json)
  OR (NEW.backfill_cursor_json IS NOT NULL AND NOT json_valid(NEW.backfill_cursor_json))
BEGIN
  SELECT RAISE(ABORT, 'provider_accounts JSON state fields must be valid JSON');
END;

CREATE TRIGGER provider_accounts_json_state_update
BEFORE UPDATE OF granted_scopes_json, backfill_cursor_json ON provider_accounts
FOR EACH ROW
WHEN NOT json_valid(NEW.granted_scopes_json)
  OR (NEW.backfill_cursor_json IS NOT NULL AND NOT json_valid(NEW.backfill_cursor_json))
BEGIN
  SELECT RAISE(ABORT, 'provider_accounts JSON state fields must be valid JSON');
END;

CREATE TRIGGER provider_message_mappings_json_state_insert
BEFORE INSERT ON provider_message_mappings
FOR EACH ROW
WHEN NEW.jmap_mailbox_ids_json IS NOT NULL
  AND NOT json_valid(NEW.jmap_mailbox_ids_json)
BEGIN
  SELECT RAISE(ABORT, 'provider_message_mappings.jmap_mailbox_ids_json must be valid JSON');
END;

CREATE TRIGGER provider_message_mappings_json_state_update
BEFORE UPDATE OF jmap_mailbox_ids_json ON provider_message_mappings
FOR EACH ROW
WHEN NEW.jmap_mailbox_ids_json IS NOT NULL
  AND NOT json_valid(NEW.jmap_mailbox_ids_json)
BEGIN
  SELECT RAISE(ABORT, 'provider_message_mappings.jmap_mailbox_ids_json must be valid JSON');
END;

CREATE TRIGGER provider_sync_events_json_state_insert
BEFORE INSERT ON provider_sync_events
FOR EACH ROW
WHEN NEW.metadata_json IS NOT NULL
  AND NOT json_valid(NEW.metadata_json)
BEGIN
  SELECT RAISE(ABORT, 'provider_sync_events.metadata_json must be valid JSON');
END;

CREATE TRIGGER provider_sync_events_json_state_update
BEFORE UPDATE OF metadata_json ON provider_sync_events
FOR EACH ROW
WHEN NEW.metadata_json IS NOT NULL
  AND NOT json_valid(NEW.metadata_json)
BEGIN
  SELECT RAISE(ABORT, 'provider_sync_events.metadata_json must be valid JSON');
END;
