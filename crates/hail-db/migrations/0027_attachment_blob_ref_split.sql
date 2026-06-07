-- Split provider-native attachment blob refs from local blob-store ids.
-- Existing attachments.blob_id values were provider refs before this migration;
-- cached local attachment ids must live in cached_blob_id only after this point.
ALTER TABLE attachments ADD COLUMN backend_blob_ref TEXT;
ALTER TABLE attachments ADD COLUMN cached_blob_id TEXT;

UPDATE attachments
SET backend_blob_ref = blob_id
WHERE blob_id IS NOT NULL;

CREATE INDEX idx_attachments_backend_blob_ref
  ON attachments(backend_blob_ref)
  WHERE backend_blob_ref IS NOT NULL;
CREATE INDEX idx_attachments_cached_blob_id
  ON attachments(cached_blob_id)
  WHERE cached_blob_id IS NOT NULL;
