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
