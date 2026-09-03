//! File storage abstraction. One trait-object `ObjectStore` (from the
//! `object_store` crate) backs both supported drivers, so the rest of
//! Cratebase never needs to know whether files live on local disk or in an
//! S3-compatible bucket (RustFS/MinIO in dev, R2/S3/B2 in production).

mod config;
mod error;

pub use config::StorageConfig;
pub use error::{StorageError, StorageResult};

use std::sync::Arc;

use bytes::Bytes;
use object_store::aws::AmazonS3Builder;
use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;
use object_store::{ObjectStore, PutPayload};

#[derive(Clone)]
pub struct Storage {
    store: Arc<dyn ObjectStore>,
}

impl Storage {
    pub fn connect(config: &StorageConfig) -> StorageResult<Self> {
        let store: Arc<dyn ObjectStore> = match config {
            StorageConfig::Local { base_dir } => {
                std::fs::create_dir_all(base_dir)
                    .map_err(|e| StorageError::Config(format!("cannot create '{base_dir}': {e}")))?;
                Arc::new(
                    LocalFileSystem::new_with_prefix(base_dir)
                        .map_err(|e| StorageError::Config(e.to_string()))?,
                )
            }
            StorageConfig::S3 {
                bucket,
                endpoint,
                region,
                access_key_id,
                secret_access_key,
                force_path_style,
            } => {
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
                Arc::new(builder.build().map_err(StorageError::Backend)?)
            }
        };
        Ok(Storage { store })
    }

    pub async fn put(&self, key: &str, data: Bytes) -> StorageResult<()> {
        self.store
            .put(&ObjectPath::from(key), PutPayload::from_bytes(data))
            .await?;
        Ok(())
    }

    pub async fn get(&self, key: &str) -> StorageResult<Bytes> {
        match self.store.get(&ObjectPath::from(key)).await {
            Ok(result) => Ok(result.bytes().await?),
            Err(object_store::Error::NotFound { .. }) => Err(StorageError::NotFound(key.to_string())),
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
    ) -> StorageResult<impl futures::Stream<Item = StorageResult<Bytes>> + Send + 'static> {
        use futures::StreamExt;
        match self.store.get(&ObjectPath::from(key)).await {
            Ok(result) => Ok(result.into_stream().map(|chunk| chunk.map_err(StorageError::Backend))),
            Err(object_store::Error::NotFound { .. }) => Err(StorageError::NotFound(key.to_string())),
            Err(e) => Err(StorageError::Backend(e)),
        }
    }


    pub async fn delete(&self, key: &str) -> StorageResult<()> {
        match self.store.delete(&ObjectPath::from(key)).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(e) => Err(StorageError::Backend(e)),
        }
    }

    pub async fn exists(&self, key: &str) -> StorageResult<bool> {
        match self.store.head(&ObjectPath::from(key)).await {
            Ok(_) => Ok(true),
            Err(object_store::Error::NotFound { .. }) => Ok(false),
            Err(e) => Err(StorageError::Backend(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> std::path::PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        std::env::temp_dir().join(format!("cratebase-storage-test-{nanos}"))
    }

    #[tokio::test]
    async fn local_round_trip() {
        let dir = temp_dir();
        let storage = Storage::connect(&StorageConfig::Local {
            base_dir: dir.to_string_lossy().to_string(),
        })
        .unwrap();

        assert!(!storage.exists("a/b.txt").await.unwrap());
        storage.put("a/b.txt", Bytes::from_static(b"hello")).await.unwrap();
        assert!(storage.exists("a/b.txt").await.unwrap());
        let data = storage.get("a/b.txt").await.unwrap();
        assert_eq!(&data[..], b"hello");
        storage.delete("a/b.txt").await.unwrap();
        assert!(!storage.exists("a/b.txt").await.unwrap());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn missing_key_is_not_found() {
        let dir = temp_dir();
        let storage = Storage::connect(&StorageConfig::Local {
            base_dir: dir.to_string_lossy().to_string(),
        })
        .unwrap();
        let err = storage.get("missing.txt").await.unwrap_err();
        assert!(matches!(err, StorageError::NotFound(_)));
        std::fs::remove_dir_all(&dir).ok();
    }
}
