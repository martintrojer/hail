ALTER TABLE provider_accounts ADD COLUMN bidirectional_sync_enabled INTEGER NOT NULL DEFAULT 0 CHECK (bidirectional_sync_enabled IN (0, 1));
