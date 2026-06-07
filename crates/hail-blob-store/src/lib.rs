use std::collections::HashSet;
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use hail_core::{BlobId, BlobKind};
use sqlx::{Row, SqlitePool};
use tokio::io::AsyncWriteExt;

const ZSTD_LEVEL: i32 = 3;
const DEFAULT_SWEEP_GRACE: Duration = Duration::from_secs(60 * 60);
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub type Result<T> = std::result::Result<T, BlobStoreError>;

#[derive(Debug)]
pub enum BlobStoreError {
    Io(std::io::Error),
    Sqlx(sqlx::Error),
    Zstd(std::io::Error),
    InvalidBlobId(hail_core::BlobIdParseError),
    HashMismatch { expected: BlobId, actual: BlobId },
    UnsupportedFileName(PathBuf),
    Join(tokio::task::JoinError),
}

impl fmt::Display for BlobStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "blob store I/O error: {err}"),
            Self::Sqlx(err) => write!(f, "blob store database error: {err}"),
            Self::Zstd(err) => write!(f, "blob store zstd error: {err}"),
            Self::InvalidBlobId(err) => write!(f, "invalid blob id: {err}"),
            Self::HashMismatch { expected, actual } => {
                write!(f, "blob hash mismatch: expected {expected}, got {actual}")
            }
            Self::UnsupportedFileName(path) => {
                write!(f, "unsupported blob store filename: {}", path.display())
            }
            Self::Join(err) => write!(f, "blob store blocking task failed: {err}"),
        }
    }
}

impl std::error::Error for BlobStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) | Self::Zstd(err) => Some(err),
            Self::Sqlx(err) => Some(err),
            Self::Join(err) => Some(err),
            Self::InvalidBlobId(err) => Some(err),
            Self::HashMismatch { .. } | Self::UnsupportedFileName(_) => None,
        }
    }
}

impl From<std::io::Error> for BlobStoreError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<sqlx::Error> for BlobStoreError {
    fn from(value: sqlx::Error) -> Self {
        Self::Sqlx(value)
    }
}

impl From<hail_core::BlobIdParseError> for BlobStoreError {
    fn from(value: hail_core::BlobIdParseError) -> Self {
        Self::InvalidBlobId(value)
    }
}

impl From<tokio::task::JoinError> for BlobStoreError {
    fn from(value: tokio::task::JoinError) -> Self {
        Self::Join(value)
    }
}

pub trait BlobStore: Send + Sync {
    fn put<'a>(
        &'a self,
        kind: BlobKind,
        bytes: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<BlobId>> + Send + 'a>>;
    fn get<'a>(
        &'a self,
        id: &'a BlobId,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + 'a>>;
    fn delete<'a>(
        &'a self,
        id: &'a BlobId,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
    fn exists<'a>(
        &'a self,
        id: &'a BlobId,
    ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>>;
    fn verify<'a>(
        &'a self,
        id: &'a BlobId,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
    fn sweep_unreferenced<'a>(
        &'a self,
        db: &'a SqlitePool,
    ) -> Pin<Box<dyn Future<Output = Result<SweepStats>> + Send + 'a>>;
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SweepStats {
    pub scanned: usize,
    pub referenced: usize,
    pub retained_grace: usize,
    pub deleted: usize,
    pub bytes_deleted: u64,
}

#[derive(Debug, Clone)]
pub struct FilesystemBlobStore {
    root: PathBuf,
    sweep_grace: Duration,
}

impl FilesystemBlobStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            sweep_grace: DEFAULT_SWEEP_GRACE,
        }
    }

    pub fn with_sweep_grace(mut self, grace: Duration) -> Self {
        self.sweep_grace = grace;
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path_for(&self, id: &BlobId) -> PathBuf {
        self.root
            .join(&id.hex()[0..2])
            .join(&id.hex()[2..4])
            .join(id.file_name())
    }

    async fn blob_files(&self) -> Result<Vec<(BlobId, PathBuf)>> {
        let mut out = Vec::new();
        match tokio::fs::read_dir(&self.root).await {
            Ok(mut first_dirs) => {
                while let Some(first) = first_dirs.next_entry().await? {
                    if !first.file_type().await?.is_dir() {
                        continue;
                    }
                    let mut second_dirs = tokio::fs::read_dir(first.path()).await?;
                    while let Some(second) = second_dirs.next_entry().await? {
                        if !second.file_type().await?.is_dir() {
                            continue;
                        }
                        let mut files = tokio::fs::read_dir(second.path()).await?;
                        while let Some(file) = files.next_entry().await? {
                            if !file.file_type().await?.is_file() {
                                continue;
                            }
                            let path = file.path();
                            if let Ok(id) = parse_blob_path(&path) {
                                out.push((id, path));
                            }
                        }
                    }
                }
                Ok(out)
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(out),
            Err(err) => Err(err.into()),
        }
    }
}

impl BlobStore for FilesystemBlobStore {
    fn put<'a>(
        &'a self,
        kind: BlobKind,
        bytes: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<BlobId>> + Send + 'a>> {
        Box::pin(async move { self.put_inner(kind, bytes).await })
    }

    fn get<'a>(
        &'a self,
        id: &'a BlobId,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + 'a>> {
        Box::pin(async move { self.get_inner(id).await })
    }

    fn delete<'a>(
        &'a self,
        id: &'a BlobId,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move { self.delete_inner(id).await })
    }

    fn exists<'a>(
        &'a self,
        id: &'a BlobId,
    ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>> {
        Box::pin(async move { self.exists_inner(id).await })
    }

    fn verify<'a>(
        &'a self,
        id: &'a BlobId,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move { self.verify_inner(id).await })
    }

    fn sweep_unreferenced<'a>(
        &'a self,
        db: &'a SqlitePool,
    ) -> Pin<Box<dyn Future<Output = Result<SweepStats>> + Send + 'a>> {
        Box::pin(async move { self.sweep_unreferenced_inner(db).await })
    }
}

impl FilesystemBlobStore {
    async fn put_inner(&self, kind: BlobKind, bytes: &[u8]) -> Result<BlobId> {
        let id = blob_id_from_bytes(kind, bytes);
        let path = self.path_for(&id);
        if tokio::fs::try_exists(&path).await? {
            return Ok(id);
        }

        let compressed = compress(bytes.to_vec()).await?;
        let parent = path
            .parent()
            .ok_or_else(|| BlobStoreError::UnsupportedFileName(path.clone()))?;
        tokio::fs::create_dir_all(parent).await?;

        let tmp_path = path.with_extension(format!(
            "{}.{}.tmp",
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = tokio::fs::File::create(&tmp_path).await?;
        file.write_all(&compressed).await?;
        file.sync_all().await?;
        drop(file);

        match tokio::fs::rename(&tmp_path, &path).await {
            Ok(()) => Ok(id),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                remove_file_if_exists(&tmp_path).await?;
                Ok(id)
            }
            Err(err) => {
                remove_file_if_exists(&tmp_path).await?;
                Err(err.into())
            }
        }
    }

    async fn get_inner(&self, id: &BlobId) -> Result<Vec<u8>> {
        let compressed = tokio::fs::read(self.path_for(id)).await?;
        decompress(compressed).await
    }

    async fn delete_inner(&self, id: &BlobId) -> Result<()> {
        remove_file_if_exists(self.path_for(id)).await
    }

    async fn exists_inner(&self, id: &BlobId) -> Result<bool> {
        Ok(tokio::fs::try_exists(self.path_for(id)).await?)
    }

    async fn verify_inner(&self, id: &BlobId) -> Result<()> {
        let bytes = self.get_inner(id).await?;
        let actual = blob_id_from_bytes(id.kind(), &bytes);
        if &actual == id {
            Ok(())
        } else {
            Err(BlobStoreError::HashMismatch {
                expected: id.clone(),
                actual,
            })
        }
    }

    async fn sweep_unreferenced_inner(&self, db: &SqlitePool) -> Result<SweepStats> {
        let referenced = load_referenced_blob_ids(db).await?;
        let mut stats = SweepStats {
            referenced: referenced.len(),
            ..SweepStats::default()
        };
        let cutoff = SystemTime::now()
            .checked_sub(self.sweep_grace)
            .unwrap_or(SystemTime::UNIX_EPOCH);

        for (id, path) in self.blob_files().await? {
            stats.scanned += 1;
            if referenced.contains(&id.to_string()) || referenced.contains(id.hex()) {
                continue;
            }
            let metadata = tokio::fs::metadata(&path).await?;
            if self.sweep_grace > Duration::ZERO
                && metadata
                    .modified()
                    .map(|mtime| mtime > cutoff)
                    .unwrap_or(true)
            {
                stats.retained_grace += 1;
                continue;
            }
            let len = metadata.len();
            remove_file_if_exists(&path).await?;
            stats.deleted += 1;
            stats.bytes_deleted += len;
        }

        Ok(stats)
    }
}

fn blob_id_from_bytes(kind: BlobKind, bytes: &[u8]) -> BlobId {
    BlobId::new(blake3::hash(bytes).to_hex().to_string(), kind)
        .expect("BLAKE3 hex digest should always be a valid blob id")
}

async fn compress(bytes: Vec<u8>) -> Result<Vec<u8>> {
    tokio::task::spawn_blocking(move || zstd::bulk::compress(&bytes, ZSTD_LEVEL))
        .await?
        .map_err(BlobStoreError::Zstd)
}

async fn decompress(bytes: Vec<u8>) -> Result<Vec<u8>> {
    tokio::task::spawn_blocking(move || zstd::decode_all(bytes.as_slice()))
        .await?
        .map_err(BlobStoreError::Zstd)
}

async fn remove_file_if_exists(path: impl AsRef<Path>) -> Result<()> {
    match tokio::fs::remove_file(path.as_ref()).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

async fn load_referenced_blob_ids(db: &SqlitePool) -> Result<HashSet<String>> {
    let mut refs = HashSet::new();
    if table_exists(db, "messages").await? {
        let rows = sqlx::query("SELECT body_blob_id FROM messages WHERE body_blob_id IS NOT NULL")
            .fetch_all(db)
            .await?;
        for row in rows {
            let id: String = row.try_get("body_blob_id")?;
            refs.insert(id);
        }
    }
    if table_exists(db, "attachments").await? {
        let rows =
            sqlx::query("SELECT cached_blob_id FROM attachments WHERE cached_blob_id IS NOT NULL")
                .fetch_all(db)
                .await?;
        for row in rows {
            let id: String = row.try_get("cached_blob_id")?;
            refs.insert(id);
        }
    }
    Ok(refs)
}

async fn table_exists(db: &SqlitePool, table: &str) -> Result<bool> {
    let exists: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
    )
    .bind(table)
    .fetch_one(db)
    .await?;
    Ok(exists == 1)
}

fn parse_blob_path(path: &Path) -> Result<BlobId> {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| BlobStoreError::UnsupportedFileName(path.to_path_buf()))?;
    let name = file_name
        .strip_suffix(".zst")
        .ok_or_else(|| BlobStoreError::UnsupportedFileName(path.to_path_buf()))?;
    Ok(BlobId::parse(name)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;
    use tempfile::TempDir;

    #[tokio::test]
    async fn round_trip_compresses_and_verifies() {
        let temp = TempDir::new().unwrap();
        let store = FilesystemBlobStore::new(temp.path());
        let bytes = b"From: test@example.org\r\nSubject: hi\r\n\r\nHello hail";

        let id = store.put(BlobKind::Eml, bytes).await.unwrap();

        assert_eq!(id.kind(), BlobKind::Eml);
        assert!(store.exists(&id).await.unwrap());
        assert_eq!(store.get(&id).await.unwrap(), bytes);
        store.verify(&id).await.unwrap();
        let path = store.path_for(&id);
        assert!(path.ends_with(id.file_name()));
        assert_eq!(path.parent().unwrap().file_name().unwrap(), &id.hex()[2..4]);
        assert_eq!(
            path.parent()
                .unwrap()
                .parent()
                .unwrap()
                .file_name()
                .unwrap(),
            &id.hex()[0..2]
        );
        assert!(
            tokio::fs::read(path)
                .await
                .unwrap()
                .starts_with(&[0x28, 0xb5, 0x2f, 0xfd])
        );
    }

    #[tokio::test]
    async fn duplicate_puts_reuse_one_file() {
        let temp = TempDir::new().unwrap();
        let store = FilesystemBlobStore::new(temp.path());
        let bytes = b"same attachment bytes";

        let first = store.put(BlobKind::Att, bytes).await.unwrap();
        let second = store.put(BlobKind::Att, bytes).await.unwrap();

        assert_eq!(first, second);
        assert_eq!(store.blob_files().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn delete_is_idempotent() {
        let temp = TempDir::new().unwrap();
        let store = FilesystemBlobStore::new(temp.path());
        let id = store.put(BlobKind::Att, b"bytes").await.unwrap();

        store.delete(&id).await.unwrap();
        store.delete(&id).await.unwrap();

        assert!(!store.exists(&id).await.unwrap());
    }

    #[tokio::test]
    async fn sweep_deletes_only_unreferenced_old_blobs() {
        let temp = TempDir::new().unwrap();
        let store = FilesystemBlobStore::new(temp.path()).with_sweep_grace(Duration::ZERO);
        let keep_body = store.put(BlobKind::Eml, b"kept body").await.unwrap();
        let keep_att = store.put(BlobKind::Att, b"kept attachment").await.unwrap();
        let delete = store.put(BlobKind::Att, b"delete me").await.unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE messages (body_blob_id TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE attachments (cached_blob_id TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO messages (body_blob_id) VALUES (?1)")
            .bind(keep_body.to_string())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO attachments (cached_blob_id) VALUES (?1)")
            .bind(keep_att.to_string())
            .execute(&pool)
            .await
            .unwrap();

        let stats = store.sweep_unreferenced(&pool).await.unwrap();

        assert_eq!(stats.scanned, 3);
        assert_eq!(stats.referenced, 2);
        assert_eq!(stats.deleted, 1);
        assert!(store.exists(&keep_body).await.unwrap());
        assert!(store.exists(&keep_att).await.unwrap());
        assert!(!store.exists(&delete).await.unwrap());
    }

    #[tokio::test]
    async fn sweep_tolerates_missing_cache_tables() {
        let temp = TempDir::new().unwrap();
        let store = FilesystemBlobStore::new(temp.path()).with_sweep_grace(Duration::ZERO);
        let id = store.put(BlobKind::Eml, b"orphan").await.unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();

        let stats = store.sweep_unreferenced(&pool).await.unwrap();

        assert_eq!(stats.deleted, 1);
        assert!(!store.exists(&id).await.unwrap());
    }

    #[tokio::test]
    async fn sweep_honors_grace_window() {
        let temp = TempDir::new().unwrap();
        let store = FilesystemBlobStore::new(temp.path()).with_sweep_grace(Duration::from_secs(60));
        let id = store.put(BlobKind::Eml, b"new orphan").await.unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();

        let stats = store.sweep_unreferenced(&pool).await.unwrap();

        assert_eq!(stats.deleted, 0);
        assert_eq!(stats.retained_grace, 1);
        assert!(store.exists(&id).await.unwrap());
    }
}
