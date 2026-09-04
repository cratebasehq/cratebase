//! File storage abstraction. One trait-object `ObjectStore` (from the
//! `object_store` crate) backs both supported drivers, so the rest of
//! Cratebase never needs to know whether files live on local disk or in an
//! S3-compatible bucket (RustFS/MinIO in dev, R2/S3/B2 in production).
//!
//! Two instances exist at runtime: the record-file store
//! ([`Storage::from_settings`], `settings.s3` or `<data_dir>/storage`) and
//! the backups store ([`Storage::backups_from_settings`],
//! `settings.backups.s3` or `<data_dir>/backups`).

mod config;
mod error;

pub use config::StorageConfig;
pub use error::{StorageError, StorageResult};

use std::io;
use std::path::Path;
use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use chrono::{DateTime, Utc};
use cratebase_core::settings::{Backups, S3};
use futures::{Stream, StreamExt, TryStreamExt};
use object_store::aws::AmazonS3Builder;
use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;
use object_store::{ObjectStore, ObjectStoreExt, PutPayload, WriteMultipart};

/// Multipart part size for [`Storage::put_stream`]. S3 requires every
/// part but the last to be at least 5 MiB; uploads that finish before
/// filling one part are sent with a single `put` instead.
const MULTIPART_CHUNK_SIZE: usize = 5 * 1024 * 1024;

/// Subdirectory of the data dir holding record files when S3 is off.
pub const LOCAL_STORAGE_DIR: &str = "storage";
/// Subdirectory of the data dir holding backups when backups S3 is off.
pub const LOCAL_BACKUPS_DIR: &str = "backups";

/// A handle to one object store. Cheap to clone.
#[derive(Clone)]
pub struct Storage {
    store: Arc<dyn ObjectStore>,
    local_root: Option<Arc<Path>>,
}

/// One object as seen by [`Storage::list`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectInfo {
    pub key: String,
    pub size: u64,
    pub last_modified: DateTime<Utc>,
}

impl Storage {
    /// Opens the driver described by `config`. Local directories are
    /// created on demand; S3 credentials are not checked until first use.
    pub fn connect(config: &StorageConfig) -> StorageResult<Self> {
        match config {
            StorageConfig::Local { base_dir } => {
                std::fs::create_dir_all(base_dir).map_err(|e| {
                    StorageError::Config(format!("cannot create '{base_dir}': {e}"))
                })?;
                let fs = LocalFileSystem::new_with_prefix(base_dir)
                    .map_err(|e| StorageError::Config(e.to_string()))?;
                Ok(Storage {
                    store: Arc::new(fs),
                    local_root: Some(Arc::from(Path::new(base_dir))),
                })
            }
            StorageConfig::S3 {
                bucket,
                endpoint,
                region,
                access_key_id,
                secret_access_key,
                force_path_style,
            } => {
                if bucket.trim().is_empty() {
                    return Err(StorageError::Config("s3 bucket is empty".into()));
                }
                let mut builder = AmazonS3Builder::new()
                    .with_bucket_name(bucket)
                    .with_region(region)
                    .with_access_key_id(access_key_id)
                    .with_secret_access_key(secret_access_key)
                    .with_virtual_hosted_style_request(!force_path_style);
                if let Some(endpoint) = endpoint {
                    builder = builder
                        .with_endpoint(endpoint)
                        .with_allow_http(endpoint.starts_with("http://"));
                }
                Ok(Storage {
                    store: Arc::new(builder.build().map_err(StorageError::Backend)?),
                    local_root: None,
                })
            }
        }
    }

    /// A local driver rooted at `dir`.
    pub fn local(dir: impl AsRef<Path>) -> StorageResult<Self> {
        Storage::connect(&StorageConfig::local(dir))
    }

    /// The record-file store per app settings: `settings.s3` when it is
    /// `enabled`, otherwise `<data_dir>/storage` on disk.
    pub fn from_settings(s3: &S3, data_dir: impl AsRef<Path>) -> StorageResult<Self> {
        if s3.enabled {
            Storage::connect(&StorageConfig::from_s3_settings(s3))
        } else {
            Storage::local(data_dir.as_ref().join(LOCAL_STORAGE_DIR))
        }
    }

    /// The backups store per app settings: `settings.backups.s3` when it
    /// is `enabled`, otherwise `<data_dir>/backups` on disk.
    pub fn backups_from_settings(
        backups: &Backups,
        data_dir: impl AsRef<Path>,
    ) -> StorageResult<Self> {
        if backups.s3.enabled {
            Storage::connect(&StorageConfig::from_s3_settings(&backups.s3))
        } else {
            Storage::local(data_dir.as_ref().join(LOCAL_BACKUPS_DIR))
        }
    }

    /// The on-disk root for the local driver; `None` for S3.
    pub fn local_root(&self) -> Option<&Path> {
        self.local_root.as_deref()
    }

    /// Whether this store is backed by the local filesystem.
    pub fn is_local(&self) -> bool {
        self.local_root.is_some()
    }

    /// Writes `data` at `key`, replacing any existing object.
    pub async fn put(&self, key: &str, data: Bytes) -> StorageResult<()> {
        self.store
            .put(&ObjectPath::from(key), PutPayload::from_bytes(data))
            .await?;
        Ok(())
    }

    /// Writes a byte stream at `key` without buffering it whole in memory.
    /// Streams shorter than one multipart chunk (5 MiB) are written with a
    /// single `put`; longer ones go through the driver's multipart upload
    /// (S3 `CreateMultipartUpload`, or a temp file renamed into place for
    /// the local driver), which is aborted if the stream errors.
    /// `size_hint` lets a caller that knows the length skip the initial
    /// buffering decision; it's advisory only.
    pub async fn put_stream<S>(
        &self,
        key: &str,
        stream: S,
        size_hint: Option<u64>,
    ) -> StorageResult<()>
    where
        S: Stream<Item = Result<Bytes, io::Error>> + Send + 'static,
    {
        let path = ObjectPath::from(key);
        let mut stream = Box::pin(stream);
        let mut buffered = BytesMut::new();

        // Small uploads: buffer and `put` once, unless the caller already
        // told us the object is big.
        if size_hint.is_none_or(|n| n < MULTIPART_CHUNK_SIZE as u64) {
            while buffered.len() < MULTIPART_CHUNK_SIZE {
                match stream.next().await {
                    Some(chunk) => buffered.extend_from_slice(&chunk?),
                    None => {
                        self.store
                            .put(&path, PutPayload::from_bytes(buffered.freeze()))
                            .await?;
                        return Ok(());
                    }
                }
            }
        }

        let upload = self.store.put_multipart(&path).await?;
        let mut writer = WriteMultipart::new_with_chunk_size(upload, MULTIPART_CHUNK_SIZE);
        writer.write(&buffered);
        drop(buffered);
        loop {
            match stream.next().await {
                Some(Ok(chunk)) => {
                    writer.write(&chunk);
                    // Bound in-flight parts so a fast producer can't queue
                    // the whole file in memory ahead of the network.
                    writer.wait_for_capacity(4).await?;
                }
                Some(Err(e)) => {
                    writer.abort().await?;
                    return Err(StorageError::Io(e));
                }
                None => break,
            }
        }
        writer.finish().await?;
        Ok(())
    }

    /// Reads the whole object at `key`.
    pub async fn get(&self, key: &str) -> StorageResult<Bytes> {
        match self.store.get(&ObjectPath::from(key)).await {
            Ok(result) => Ok(result.bytes().await?),
            Err(object_store::Error::NotFound { .. }) => {
                Err(StorageError::NotFound(key.to_string()))
            }
            Err(e) => Err(StorageError::Backend(e)),
        }
    }

    /// Like [`Self::get`] but returns a chunked byte stream instead of
    /// buffering the whole object in memory — used for serving file
    /// downloads so a large upload doesn't cost a large allocation per
    /// concurrent request.
    pub async fn get_stream(
        &self,
        key: &str,
    ) -> StorageResult<impl Stream<Item = StorageResult<Bytes>> + Send + 'static> {
        match self.store.get(&ObjectPath::from(key)).await {
            Ok(result) => Ok(result
                .into_stream()
                .map(|chunk| chunk.map_err(StorageError::Backend))),
            Err(object_store::Error::NotFound { .. }) => {
                Err(StorageError::NotFound(key.to_string()))
            }
            Err(e) => Err(StorageError::Backend(e)),
        }
    }

    /// Deletes the object at `key`; deleting a missing key is not an error.
    pub async fn delete(&self, key: &str) -> StorageResult<()> {
        match self.store.delete(&ObjectPath::from(key)).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(e) => Err(StorageError::Backend(e)),
        }
    }

    /// Deletes every object whose key starts with `prefix` (a record's
    /// `{collectionId}/{recordId}/` directory, say) and returns how many
    /// were removed. Missing objects are skipped, not errors.
    pub async fn delete_prefix(&self, prefix: &str) -> StorageResult<usize> {
        let keys = self.list(prefix).await?;
        if keys.is_empty() {
            return Ok(0);
        }
        let locations = futures::stream::iter(
            keys.into_iter()
                .map(|info| Ok::<_, object_store::Error>(ObjectPath::from(info.key))),
        )
        .boxed();
        let mut deleted = 0usize;
        let mut results = self.store.delete_stream(locations);
        while let Some(result) = results.next().await {
            match result {
                Ok(_) | Err(object_store::Error::NotFound { .. }) => deleted += 1,
                Err(e) => return Err(StorageError::Backend(e)),
            }
        }
        Ok(deleted)
    }

    /// Whether an object exists at `key`.
    pub async fn exists(&self, key: &str) -> StorageResult<bool> {
        Ok(self.size(key).await?.is_some())
    }

    /// Size in bytes of the object at `key`, or `None` if it doesn't exist.
    pub async fn size(&self, key: &str) -> StorageResult<Option<u64>> {
        match self.store.head(&ObjectPath::from(key)).await {
            Ok(meta) => Ok(Some(meta.size)),
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(e) => Err(StorageError::Backend(e)),
        }
    }

    /// Objects whose key starts with `prefix` (recursively), used by the
    /// backups feature to enumerate what's under `backups/` and by record
    /// deletion to find a record's files — the object store itself is the
    /// source of truth for what exists. An empty prefix lists everything.
    pub async fn list(&self, prefix: &str) -> StorageResult<Vec<ObjectInfo>> {
        let prefix = prefix.trim_matches('/');
        let prefix_path = (!prefix.is_empty()).then(|| ObjectPath::from(prefix));
        let metas = self
            .store
            .list(prefix_path.as_ref())
            .try_collect::<Vec<_>>()
            .await
            .map_err(StorageError::Backend)?;
        let mut infos: Vec<ObjectInfo> = metas
            .into_iter()
            .map(|m| ObjectInfo {
                key: m.location.to_string(),
                size: m.size,
                last_modified: m.last_modified,
            })
            .collect();
        infos.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(infos)
    }
}

impl std::fmt::Debug for Storage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Storage")
            .field("driver", &self.store.to_string())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("cratebase-storage-test-{nanos}-{n}"))
    }

    fn bytes_stream(
        chunks: Vec<Result<Bytes, io::Error>>,
    ) -> impl Stream<Item = Result<Bytes, io::Error>> + Send + 'static {
        futures::stream::iter(chunks)
    }

    #[tokio::test]
    async fn local_round_trip() {
        let dir = temp_dir();
        let storage = Storage::local(&dir).unwrap();
        assert!(storage.is_local());
        assert_eq!(storage.local_root(), Some(dir.as_path()));

        assert!(!storage.exists("a/b.txt").await.unwrap());
        assert_eq!(storage.size("a/b.txt").await.unwrap(), None);
        storage
            .put("a/b.txt", Bytes::from_static(b"hello"))
            .await
            .unwrap();
        assert!(storage.exists("a/b.txt").await.unwrap());
        assert_eq!(storage.size("a/b.txt").await.unwrap(), Some(5));
        let data = storage.get("a/b.txt").await.unwrap();
        assert_eq!(&data[..], b"hello");
        storage.delete("a/b.txt").await.unwrap();
        assert!(!storage.exists("a/b.txt").await.unwrap());
        storage.delete("a/b.txt").await.unwrap();

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn missing_key_is_not_found() {
        let dir = temp_dir();
        let storage = Storage::local(&dir).unwrap();
        let err = storage.get("missing.txt").await.unwrap_err();
        assert!(matches!(err, StorageError::NotFound(_)));
        assert!(storage.get_stream("missing.txt").await.is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn put_stream_small_upload() {
        let dir = temp_dir();
        let storage = Storage::local(&dir).unwrap();
        let stream = bytes_stream(vec![
            Ok(Bytes::from_static(b"hello ")),
            Ok(Bytes::from_static(b"world")),
        ]);
        storage.put_stream("c/d.txt", stream, None).await.unwrap();
        assert_eq!(&storage.get("c/d.txt").await.unwrap()[..], b"hello world");
        assert_eq!(storage.size("c/d.txt").await.unwrap(), Some(11));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn put_stream_empty_upload() {
        let dir = temp_dir();
        let storage = Storage::local(&dir).unwrap();
        storage
            .put_stream("empty.bin", bytes_stream(vec![]), Some(0))
            .await
            .unwrap();
        assert_eq!(storage.size("empty.bin").await.unwrap(), Some(0));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn put_stream_large_upload_goes_multipart() {
        let dir = temp_dir();
        let storage = Storage::local(&dir).unwrap();
        // 11 MiB in 1 MiB chunks: crosses the multipart threshold twice.
        let chunk = Bytes::from(vec![7u8; 1024 * 1024]);
        let total = 11 * chunk.len();
        let chunks: Vec<_> = (0..11).map(|_| Ok(chunk.clone())).collect();
        storage
            .put_stream("big.bin", bytes_stream(chunks), Some(total as u64))
            .await
            .unwrap();
        assert_eq!(storage.size("big.bin").await.unwrap(), Some(total as u64));
        let data = storage.get("big.bin").await.unwrap();
        assert_eq!(data.len(), total);
        assert!(data.iter().all(|b| *b == 7));

        // Same again without a size hint: the buffering path must switch
        // to multipart once the first chunk fills up.
        let chunks: Vec<_> = (0..6).map(|_| Ok(chunk.clone())).collect();
        storage
            .put_stream("big2.bin", bytes_stream(chunks), None)
            .await
            .unwrap();
        assert_eq!(
            storage.size("big2.bin").await.unwrap(),
            Some(6 * chunk.len() as u64)
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn put_stream_error_leaves_no_object() {
        let dir = temp_dir();
        let storage = Storage::local(&dir).unwrap();
        let chunk = Bytes::from(vec![1u8; 1024 * 1024]);
        let mut chunks: Vec<Result<Bytes, io::Error>> = (0..6).map(|_| Ok(chunk.clone())).collect();
        chunks.push(Err(io::Error::other("boom")));
        let err = storage
            .put_stream("broken.bin", bytes_stream(chunks), None)
            .await
            .unwrap_err();
        assert!(matches!(err, StorageError::Io(_)), "{err}");
        assert!(!storage.exists("broken.bin").await.unwrap());

        // Small (buffered) path too.
        let err = storage
            .put_stream(
                "broken-small.bin",
                bytes_stream(vec![Err(io::Error::other("boom"))]),
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, StorageError::Io(_)));
        assert!(!storage.exists("broken-small.bin").await.unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn list_and_delete_prefix() {
        let dir = temp_dir();
        let storage = Storage::local(&dir).unwrap();
        for key in [
            "pbc_1/rec_a/one.txt",
            "pbc_1/rec_a/two.txt",
            "pbc_1/rec_a/thumbs_two.txt/100x100_two.txt",
            "pbc_1/rec_b/three.txt",
            "pbc_2/rec_c/four.txt",
        ] {
            storage.put(key, Bytes::from_static(b"x")).await.unwrap();
        }

        let all = storage.list("").await.unwrap();
        assert_eq!(all.len(), 5);
        assert!(all.iter().all(|o| o.size == 1));

        let rec_a: Vec<String> = storage
            .list("pbc_1/rec_a")
            .await
            .unwrap()
            .into_iter()
            .map(|o| o.key)
            .collect();
        assert_eq!(
            rec_a,
            vec![
                "pbc_1/rec_a/one.txt",
                "pbc_1/rec_a/thumbs_two.txt/100x100_two.txt",
                "pbc_1/rec_a/two.txt",
            ]
        );
        assert_eq!(storage.list("pbc_1/").await.unwrap().len(), 4);
        assert!(storage.list("nothing/here").await.unwrap().is_empty());

        assert_eq!(storage.delete_prefix("pbc_1/rec_a").await.unwrap(), 3);
        assert!(storage.list("pbc_1/rec_a").await.unwrap().is_empty());
        assert!(storage.exists("pbc_1/rec_b/three.txt").await.unwrap());
        assert!(storage.exists("pbc_2/rec_c/four.txt").await.unwrap());
        assert_eq!(storage.delete_prefix("pbc_1/rec_a").await.unwrap(), 0);
        assert_eq!(storage.delete_prefix("pbc_1").await.unwrap(), 1);
        assert_eq!(storage.list("").await.unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn get_stream_round_trip() {
        let dir = temp_dir();
        let storage = Storage::local(&dir).unwrap();
        storage
            .put("s.txt", Bytes::from_static(b"streamed"))
            .await
            .unwrap();
        let stream = storage.get_stream("s.txt").await.unwrap();
        let chunks: Vec<Bytes> = stream.try_collect().await.unwrap();
        let joined: Vec<u8> = chunks.into_iter().flatten().collect();
        assert_eq!(joined, b"streamed");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn from_settings_picks_local_dirs_when_s3_disabled() {
        let dir = temp_dir();
        let storage = Storage::from_settings(&S3::default(), &dir).unwrap();
        assert_eq!(storage.local_root(), Some(dir.join("storage").as_path()));
        assert!(dir.join("storage").is_dir());

        let backups = Storage::backups_from_settings(&Backups::default(), &dir).unwrap();
        assert_eq!(backups.local_root(), Some(dir.join("backups").as_path()));
        assert!(dir.join("backups").is_dir());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn from_settings_builds_s3_when_enabled() {
        let s3 = S3 {
            enabled: true,
            bucket: "bucket".into(),
            region: "us-east-1".into(),
            endpoint: "http://localhost:9000".into(),
            access_key: "ak".into(),
            secret: "sk".into(),
            force_path_style: true,
        };
        let storage = Storage::from_settings(&s3, "/nonexistent").unwrap();
        assert!(!storage.is_local());

        let backups = Backups {
            s3: s3.clone(),
            ..Backups::default()
        };
        assert!(!Storage::backups_from_settings(&backups, "/nonexistent")
            .unwrap()
            .is_local());

        let no_bucket = S3 {
            bucket: String::new(),
            ..s3
        };
        assert!(matches!(
            Storage::from_settings(&no_bucket, "/nonexistent"),
            Err(StorageError::Config(_))
        ));
    }

    #[test]
    fn s3_settings_empty_endpoint_means_aws() {
        let cfg = StorageConfig::from_s3_settings(&S3 {
            endpoint: "  ".into(),
            ..S3::default()
        });
        match cfg {
            StorageConfig::S3 { endpoint, .. } => assert!(endpoint.is_none()),
            _ => panic!("expected S3 config"),
        }
    }
}
