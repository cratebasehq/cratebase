use bytes::Bytes;
use cratebase_storage::{Storage, StorageConfig};

/// Proves the S3 driver actually round-trips against a real S3-compatible
/// server (RustFS/MinIO in dev, R2 in production all speak the same
/// protocol), including the multipart streaming path and prefix listing /
/// deletion. Skips when `TEST_S3_ENDPOINT` isn't set.
#[tokio::test]
async fn s3_round_trip() {
    let Ok(endpoint) = std::env::var("TEST_S3_ENDPOINT") else {
        eprintln!("skipping s3_round_trip: TEST_S3_ENDPOINT not set");
        return;
    };
    let bucket = std::env::var("TEST_S3_BUCKET").unwrap_or_else(|_| "cratebase-test".into());
    let access_key_id =
        std::env::var("TEST_S3_ACCESS_KEY").unwrap_or_else(|_| "rustfsadmin".into());
    let secret_access_key =
        std::env::var("TEST_S3_SECRET_KEY").unwrap_or_else(|_| "rustfsadmin".into());

    let storage = Storage::connect(&StorageConfig::S3 {
        bucket,
        endpoint: Some(endpoint),
        region: "us-east-1".into(),
        access_key_id,
        secret_access_key,
        force_path_style: true,
    })
    .unwrap();
    assert!(!storage.is_local());

    let prefix = format!("cratebase-test/{}", uuid_like());
    let key = format!("{prefix}/hello.txt");
    assert!(!storage.exists(&key).await.unwrap());

    storage
        .put(&key, Bytes::from_static(b"hello from rustfs"))
        .await
        .unwrap();
    assert!(storage.exists(&key).await.unwrap());
    assert_eq!(storage.size(&key).await.unwrap(), Some(17));

    let data = storage.get(&key).await.unwrap();
    assert_eq!(&data[..], b"hello from rustfs");

    // Streaming upload large enough to force a real multipart upload.
    let big_key = format!("{prefix}/big.bin");
    let chunk = Bytes::from(vec![9u8; 1024 * 1024]);
    let chunks: Vec<Result<Bytes, std::io::Error>> = (0..11).map(|_| Ok(chunk.clone())).collect();
    storage
        .put_stream(&big_key, futures::stream::iter(chunks), None)
        .await
        .unwrap();
    assert_eq!(
        storage.size(&big_key).await.unwrap(),
        Some(11 * 1024 * 1024)
    );

    let listed = storage.list(&prefix).await.unwrap();
    assert_eq!(listed.len(), 2);

    assert_eq!(storage.delete_prefix(&prefix).await.unwrap(), 2);
    assert!(!storage.exists(&key).await.unwrap());
    assert!(!storage.exists(&big_key).await.unwrap());
    assert!(storage.list(&prefix).await.unwrap().is_empty());
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    format!(
        "{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}
