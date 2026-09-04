use std::path::Path;

/// Where uploaded files live. `Local` is the zero-config default; `S3`
/// works with AWS S3 itself and every S3-compatible service (RustFS, MinIO,
/// Cloudflare R2, Backblaze B2, ...) by pointing `endpoint` at it.
#[derive(Debug, Clone)]
pub enum StorageConfig {
    Local {
        base_dir: String,
    },
    S3 {
        bucket: String,
        /// `None` targets real AWS S3. `Some(url)` targets an S3-compatible
        /// endpoint (RustFS/MinIO in dev, R2/B2 in production).
        endpoint: Option<String>,
        region: String,
        access_key_id: String,
        secret_access_key: String,
        /// RustFS/MinIO need path-style requests (`endpoint/bucket/key`)
        /// rather than AWS's virtual-hosted style (`bucket.endpoint/key`).
        force_path_style: bool,
    },
}

impl StorageConfig {
    /// A local driver rooted at `dir`.
    pub fn local(dir: impl AsRef<Path>) -> Self {
        StorageConfig::Local {
            base_dir: dir.as_ref().to_string_lossy().into_owned(),
        }
    }

    /// The S3 driver described by a PocketBase-shaped `s3` settings block
    /// (used for both `settings.s3` and `settings.backups.s3`). An empty
    /// `endpoint` targets AWS itself. Ignores `enabled`; callers decide
    /// whether to use this or [`StorageConfig::local`].
    pub fn from_s3_settings(s3: &cratebase_core::settings::S3) -> Self {
        let endpoint = s3.endpoint.trim();
        StorageConfig::S3 {
            bucket: s3.bucket.clone(),
            endpoint: if endpoint.is_empty() {
                None
            } else {
                Some(endpoint.to_string())
            },
            region: s3.region.clone(),
            access_key_id: s3.access_key.clone(),
            secret_access_key: s3.secret.clone(),
            force_path_style: s3.force_path_style,
        }
    }
}
