use bytes::Bytes;
use cratebase_storage::{Storage, StorageConfig};

/// Proves the S3 driver actually round-trips against a real S3-compatible
/// server (RustFS/MinIO in dev, R2 in production all speak the same
/// protocol). Skips when `TEST_S3_*` env vars aren't set.
#[tokio::test]
async fn s3_round_trip() {
    let Ok(endpoint) = std::env::var("TEST_S3_ENDPOINT") else {
        eprintln!("skipping s3_round_trip: TEST_S3_ENDPOINT not set");
        return;
    };
    let bucket = std::env::var("TEST_S3_BUCKET").unwrap_or_else(|_| "cratebase-test".into());
    let access_key_id = std::env::var("TEST_S3_ACCESS_KEY").unwrap_or_else(|_| "rustfsadmin".into());
    let secret_access_key = std::env::var("TEST_S3_SECRET_KEY").unwrap_or_else(|_| "rustfsadmin".into());

    let storage = Storage::connect(&StorageConfig::S3 {
        bucket,
        endpoint: Some(endpoint),
        region: "us-east-1".into(),
        access_key_id,
        secret_access_key,
        force_path_style: true,
    })
    .unwrap();

    let key = format!("cratebase-test/{}.txt", uuid_like());
    assert!(!storage.exists(&key).await.unwrap());

    storage.put(&key, Bytes::from_static(b"hello from rustfs")).await.unwrap();
    assert!(storage.exists(&key).await.unwrap());

    let data = storage.get(&key).await.unwrap();
    assert_eq!(&data[..], b"hello from rustfs");

    storage.delete(&key).await.unwrap();
    assert!(!storage.exists(&key).await.unwrap());
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    format!("{}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos())
}
