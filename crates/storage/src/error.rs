/// Errors from configuring or talking to the object store.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// No object at that key.
    #[error("file not found: {0}")]
    NotFound(String),
    /// The underlying `object_store` driver failed.
    #[error(transparent)]
    Backend(#[from] object_store::Error),
    /// Bad driver configuration (unwritable local dir, bad S3 settings).
    #[error("invalid storage configuration: {0}")]
    Config(String),
    /// The upload stream itself yielded an error.
    #[error("upload stream error: {0}")]
    Io(#[from] std::io::Error),
}

pub type StorageResult<T> = Result<T, StorageError>;
