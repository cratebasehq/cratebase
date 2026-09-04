//! HTTP-level tests for `field+`/`field-` on a multi-file (`maxSelect >
//! 1`) field — PocketBase's "append/remove without replacing the whole
//! value" syntax (`ROADMAP.md`'s "Later" section).
//!
//! The feature is already wired end to end through the existing pieces:
//! `routes::common::parse_multipart` stages an uploaded file under its
//! (possibly modifier-suffixed) key, `cratebase_db::validate::apply_modifiers`
//! merges that against the stored value the same way it already does for
//! `select`/`relation`, and `routes::records::update_record`'s
//! before/after file-name diff deletes whatever dropped out of the list
//! from the object store. These tests pin that whole path against a real
//! `App` + router, including a genuine multipart request over HTTP, in a
//! module of its own (rather than appended to `routes::records`'s own
//! test module) because that file is under concurrent edit from sibling
//! work in this batch.
//!
//! The unknown-filename-on-remove case is deliberately a silent no-op,
//! not a 400: probed directly against a real PocketBase v0.40.2 instance
//! (`POST` a 3-file record, then `PATCH {"attachments-":
//! "not_a_real_file.txt"}`), which answers `200` with the list
//! unchanged. `field-` is a plain set-difference, and PocketBase does not
//! treat "nothing to remove" as an error.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use cratebase_core::{Collection, CollectionType, Field, FieldKind};

use crate::app::App;
use crate::config::Config;
use crate::extract::RequestInfo;
use crate::routes::collections;
use crate::routes::common;

async fn test_app() -> (App, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("temp dir");
    let app = App::new(Config::memory(dir.path()));
    app.bootstrap().await.expect("bootstrap");
    (app, dir)
}

/// A base collection with a `title` text field and an `attachments` file
/// field capped at `max_select`, wide open on every rule so the test can
/// write to it without an auth token.
async fn docs_collection(app: &App, max_select: i64) -> Arc<Collection> {
    let mut next = Collection::new("docs", CollectionType::Base);
    next.list_rule = Some(String::new());
    next.view_rule = Some(String::new());
    next.create_rule = Some(String::new());
    next.update_rule = Some(String::new());
    next.delete_rule = Some(String::new());
    next.fields.push(Field::new(
        "title",
        FieldKind::Text {
            min: 0,
            max: 0,
            pattern: String::new(),
            autogenerate_pattern: String::new(),
            primary_key: false,
        },
    ));
    next.fields.push(Field::new(
        "attachments",
        FieldKind::File {
            max_select,
            max_size: 0,
            mime_types: vec![],
            thumbs: vec![],
            protected: false,
        },
    ));
    collections::prepare_new(&mut next);
    collections::validate(app, &next, None, "test").unwrap();
    let info = RequestInfo::default();
    collections::apply(app, next, None, collections::Change::Create, &info, None)
        .await
        .unwrap()
}

/// One part of a hand-built `multipart/form-data` body.
enum Part {
    Text(&'static str, &'static str),
    File(&'static str, &'static str, &'static [u8]),
}

fn multipart_body(parts: &[Part]) -> (String, Vec<u8>) {
    let boundary = "cratebase-test-boundary".to_string();
    let mut body = Vec::new();
    for part in parts {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        match part {
            Part::Text(name, value) => {
                body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
                );
                body.extend_from_slice(value.as_bytes());
            }
            Part::File(name, filename, data) => {
                body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: text/plain\r\n\r\n"
                    )
                    .as_bytes(),
                );
                body.extend_from_slice(data);
            }
        }
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (boundary, body)
}

async fn multipart_request(
    app: &App,
    method: &str,
    uri: &str,
    parts: &[Part],
) -> (StatusCode, Value) {
    let (boundary, body) = multipart_body(parts);
    let router = crate::routes::api_router(app).with_state(app.clone());
    let response = router
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, json)
}

async fn json_request(app: &App, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let router = crate::routes::api_router(app).with_state(app.clone());
    let response = router
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, json)
}

fn names(record: &Value) -> Vec<String> {
    record["attachments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn append_adds_without_disturbing_existing_files() {
    let (app, _dir) = test_app().await;
    docs_collection(&app, 5).await;

    let (status, created) = multipart_request(
        &app,
        "POST",
        "/collections/docs/records",
        &[
            Part::Text("title", "Multi"),
            Part::File("attachments", "a.txt", b"A"),
            Part::File("attachments", "b.txt", b"B"),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let original = names(&created);
    assert_eq!(original.len(), 2);

    let (status, updated) = multipart_request(
        &app,
        "PATCH",
        &format!(
            "/collections/docs/records/{}",
            created["id"].as_str().unwrap()
        ),
        &[Part::File("attachments+", "c.txt", b"C")],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let after = names(&updated);
    assert_eq!(after.len(), 3);
    // The two original names are untouched, just joined by a third.
    assert!(after.contains(&original[0]));
    assert!(after.contains(&original[1]));
}

#[tokio::test]
async fn append_respects_max_select_and_rejects_when_exceeded() {
    let (app, _dir) = test_app().await;
    docs_collection(&app, 2).await;

    let (_status, created) = multipart_request(
        &app,
        "POST",
        "/collections/docs/records",
        &[
            Part::Text("title", "Capped"),
            Part::File("attachments", "a.txt", b"A"),
            Part::File("attachments", "b.txt", b"B"),
        ],
    )
    .await;
    let id = created["id"].as_str().unwrap().to_string();

    let (status, err) = multipart_request(
        &app,
        "PATCH",
        &format!("/collections/docs/records/{id}"),
        &[Part::File("attachments+", "c.txt", b"C")],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        err["data"]["attachments"]["code"],
        "validation_too_many_files"
    );

    // Rejected: the record's file list is untouched.
    let (status, record) = json_request(
        &app,
        "GET",
        &format!("/collections/docs/records/{id}"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(names(&record).len(), 2);
}

#[tokio::test]
async fn remove_deletes_only_named_files_and_their_storage_blobs() {
    let (app, _dir) = test_app().await;
    let collection = docs_collection(&app, 5).await;

    let (_status, created) = multipart_request(
        &app,
        "POST",
        "/collections/docs/records",
        &[
            Part::Text("title", "Multi"),
            Part::File("attachments", "a.txt", b"A"),
            Part::File("attachments", "b.txt", b"B"),
            Part::File("attachments", "c.txt", b"C"),
        ],
    )
    .await;
    let id = created["id"].as_str().unwrap().to_string();
    let original = names(&created);
    let target = original[1].clone();

    let (status, updated) = json_request(
        &app,
        "PATCH",
        &format!("/collections/docs/records/{id}"),
        serde_json::json!({ "attachments-": target }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let after = names(&updated);
    assert_eq!(after.len(), 2);
    assert!(!after.contains(&target));
    assert!(after.contains(&original[0]));
    assert!(after.contains(&original[2]));

    // The removed file's actual blob is gone, not just dropped from the
    // JSON list.
    let key = common::file_key(&collection.id, &id, &target);
    assert!(!app.storage().exists(&key).await.unwrap());
    // The two kept files are still there.
    let kept_key = common::file_key(&collection.id, &id, &original[0]);
    assert!(app.storage().exists(&kept_key).await.unwrap());
}

#[tokio::test]
async fn plain_field_name_still_fully_replaces_as_before() {
    let (app, _dir) = test_app().await;
    let collection = docs_collection(&app, 5).await;

    let (_status, created) = multipart_request(
        &app,
        "POST",
        "/collections/docs/records",
        &[
            Part::Text("title", "Multi"),
            Part::File("attachments", "a.txt", b"A"),
            Part::File("attachments", "b.txt", b"B"),
        ],
    )
    .await;
    let id = created["id"].as_str().unwrap().to_string();
    let original = names(&created);

    let (status, updated) = multipart_request(
        &app,
        "PATCH",
        &format!("/collections/docs/records/{id}"),
        &[Part::File("attachments", "z.txt", b"Z")],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let after = names(&updated);
    // Unsuffixed field name: whole-value replace, exactly as before this
    // feature existed.
    assert_eq!(after.len(), 1);
    assert!(!after.contains(&original[0]));
    assert!(!after.contains(&original[1]));

    // Both original files were orphaned by the replace and are gone from
    // storage.
    let key0 = common::file_key(&collection.id, &id, &original[0]);
    let key1 = common::file_key(&collection.id, &id, &original[1]);
    assert!(!app.storage().exists(&key0).await.unwrap());
    assert!(!app.storage().exists(&key1).await.unwrap());
}

#[tokio::test]
async fn removing_an_unknown_filename_is_a_clean_noop() {
    let (app, _dir) = test_app().await;
    docs_collection(&app, 5).await;

    let (_status, created) = multipart_request(
        &app,
        "POST",
        "/collections/docs/records",
        &[
            Part::Text("title", "Multi"),
            Part::File("attachments", "a.txt", b"A"),
        ],
    )
    .await;
    let id = created["id"].as_str().unwrap().to_string();
    let original = names(&created);

    let (status, updated) = json_request(
        &app,
        "PATCH",
        &format!("/collections/docs/records/{id}"),
        serde_json::json!({ "attachments-": "not_a_real_file.txt" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(names(&updated), original);
}
