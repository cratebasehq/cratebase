#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("file not found: {0}")]
    NotFound(String),
    #[error(transparent)]
    Backend(#[from] object_store::Error),
    #[error("invalid storage configuration: {0}")]
    Config(String),
}

pub type StorageResult<T> = Result<T, StorageError>;
