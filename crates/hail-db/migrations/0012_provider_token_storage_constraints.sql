-- Harden provider refresh-token storage invariants.
--
-- Provider sync currently knows how to open only DB-encrypted refresh tokens.
-- External secret references are not implemented yet, so active/schedulable
-- accounts must not use refresh_token_ref as an unconstrained escape hatch.
-- AES-GCM provider-token blobs are nonce (12) || non-empty ciphertext || tag
-- (16), so the shortest structurally valid encrypted non-empty token is 29
-- bytes.

CREATE TEMP TABLE provider_token_storage_constraint_check (
  ok INTEGER NOT NULL CHECK (ok = 1)
);
INSERT INTO provider_token_storage_constraint_check(ok)
SELECT CASE
  WHEN COUNT(*) = 0 THEN 1
  ELSE 0
END
FROM provider_accounts
WHERE sync_status IN ('initial_sync', 'active', 'error')
  AND NOT (
    refresh_token_ref IS NULL
    AND refresh_token_enc IS NOT NULL
    AND length(refresh_token_enc) >= 29
  );
DROP TABLE provider_token_storage_constraint_check;

CREATE TRIGGER provider_accounts_refresh_token_storage_insert
BEFORE INSERT ON provider_accounts
FOR EACH ROW
WHEN NEW.sync_status IN ('initial_sync', 'active', 'error')
  AND NOT (
    NEW.refresh_token_ref IS NULL
    AND NEW.refresh_token_enc IS NOT NULL
    AND length(NEW.refresh_token_enc) >= 29
  )
BEGIN
  SELECT RAISE(ABORT, 'active provider_accounts require encrypted refresh_token_enc and no refresh_token_ref');
END;

CREATE TRIGGER provider_accounts_refresh_token_storage_update
BEFORE UPDATE OF sync_status, refresh_token_enc, refresh_token_ref ON provider_accounts
FOR EACH ROW
WHEN NEW.sync_status IN ('initial_sync', 'active', 'error')
  AND NOT (
    NEW.refresh_token_ref IS NULL
    AND NEW.refresh_token_enc IS NOT NULL
    AND length(NEW.refresh_token_enc) >= 29
  )
BEGIN
  SELECT RAISE(ABORT, 'active provider_accounts require encrypted refresh_token_enc and no refresh_token_ref');
END;
