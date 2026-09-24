//! `cratebase seed <path>`: populate collections from a JSON file (or a
//! directory of them) through the ordinary record-creation path — the
//! same validation, relation checks and auth password hashing an HTTP
//! `POST /api/collections/<name>/records` gets — rather than raw SQL, so
//! a seed file can never produce a record the API itself would reject.
//!
//! # File format
//!
//! A JSON seed file is one object keyed by collection name:
//!
//! ```json
//! {
//!   "users": [
//!     { "id": "seeduser0000001", "email": "a@example.com", "password": "supersecret123" }
//!   ],
//!   "posts": [
//!     { "id": "seedpost0000001", "title": "Hello", "author": "seeduser0000001" }
//!   ]
//! }
//! ```
//!
//! `id` is optional but, when present, is honoured verbatim (validated
//! the same way a client-supplied id on `POST .../records` is — see
//! `cratebase_db::validate::id_on_create`) precisely so other records in
//! the same seed run — in this file or a later one — can point relations
//! at a stable id instead of a value discovered after the fact. An auth
//! collection's `password` is hashed exactly like any other create
//! (`cratebase_db::records::create`'s own `hash_passwords`); there is no
//! `passwordConfirm` to also supply, since that check only exists at the
//! HTTP boundary.
//!
//! `path` may be a single `*.json` file, or a directory containing
//! `*.json` files (applied in file-name order, so `01-users.json` seeds
//! before `02-posts.json`) and, optionally, `*.js` files (see below).
//! Within one JSON file, collections are applied in **alphabetical key
//! order** (this crate's `serde_json` is not built with the
//! `preserve_order` feature, so a JSON object's insertion order is not
//! recoverable) — split cross-collection dependencies across
//! separately-numbered files when that matters, as in the example above.
//!
//! Every JSON file in one `seed::run` call is applied inside a single
//! database transaction: an invalid record anywhere aborts the entire
//! run and nothing already written by this call is committed. The error
//! names the file, collection and zero-based index of the record that
//! failed.
//!
//! # `--upsert`
//!
//! Without `--upsert`, re-running a seed that already ran fails outright
//! on the first id collision (a plain unique-constraint violation) —
//! seeding is meant to be run once against a fresh database. With
//! `--upsert`, a record whose `id` already exists is updated in place
//! instead (existing fields not mentioned in the seed record are left
//! alone, PocketBase's own `PATCH` semantics — see
//! `cratebase_db::records::apply_body`), making the whole run idempotent;
//! a record with no `id` is always created fresh either way, since there
//! is nothing to match an existing row against.
//!
//! # `*.js` seed files
//!
//! A `*.js` seed file is written exactly like a migration
//! (`migrate((app) => { ... }, (app) => {})`) and its `up` function runs
//! once, on the JS runtime, with `$app` available and the same
//! `$app`-in-a-transaction semantics as a real migration's `up`
//! ([`cratebase_jsvm::Runtime::run_seed_up`]) — just never recorded in
//! the migration ledger, since a seed is meant to be re-run (or
//! upserted), not tracked as "applied once". Because it runs on a
//! separate execution engine (the JS worker pool, its own database
//! connection) it cannot join the same Rust-level transaction as the
//! JSON files above: every `*.json` file in the run is applied together,
//! atomically, first; each `*.js` file then runs afterward, in file-name
//! order, each in its own transaction, so a `.js` seed file can rely on
//! data every `.json` file seeded but not vice versa. When no JS runtime
//! is already running (a non-`--dev` boot with empty `pb_hooks`/
//! `pb_migrations`), one is started on demand for the duration of the
//! seed run.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cratebase_core::{AppError, Collection};
use cratebase_db::records;
use serde_json::{Map, Value};

use crate::app::{App, TxApp};

/// What one `seed::run` call did.
#[derive(Debug, Default, Clone)]
pub struct SeedReport {
    /// Every file applied, in the order it was applied.
    pub files: Vec<PathBuf>,
    /// Records created (a fresh insert, no matching existing id).
    pub created: usize,
    /// Records updated in place because `--upsert` found a matching id.
    pub upserted: usize,
}

impl SeedReport {
    /// Total records written, created or upserted.
    pub fn total(&self) -> usize {
        self.created + self.upserted
    }
}

/// Seed `app`'s database from `path` (a JSON file, or a directory of
/// `*.json`/`*.js` seed files — see the module doc). `upsert` makes a
/// record with an already-existing `id` an update instead of a failure.
pub async fn run(app: &App, path: &Path, upsert: bool) -> Result<SeedReport, AppError> {
    let files = collect_seed_files(path)?;
    if files.is_empty() {
        return Err(AppError::bad_request(format!(
            "no *.json or *.js seed files found at {}",
            path.display()
        )));
    }

    let json_files: Vec<PathBuf> = files
        .iter()
        .filter(|f| has_extension(f, "json"))
        .cloned()
        .collect();
    let js_files: Vec<PathBuf> = files
        .iter()
        .filter(|f| has_extension(f, "js"))
        .cloned()
        .collect();

    // Every JSON file is applied together, atomically: an invalid record
    // in the last file rolls back every record the earlier files in this
    // same call already wrote (see the module doc's "nothing already
    // written by this call is committed").
    let mut report = app
        .run_in_transaction(move |tx| {
            Box::pin(async move {
                let mut report = SeedReport::default();
                for file in &json_files {
                    apply_json_file(&tx, file, upsert, &mut report).await?;
                    report.files.push(file.clone());
                }
                Ok(report)
            })
        })
        .await?;

    // `*.js` files run afterward — see the module doc for why they can't
    // join the transaction above.
    if !js_files.is_empty() {
        let runtime = ensure_runtime(app).await?;
        for file in &js_files {
            runtime.run_seed_up(file.clone()).await.map_err(|e| {
                AppError::bad_request(format!("{}: {e}", file.display()))
            })?;
            report.files.push(file.clone());
        }
    }

    Ok(report)
}

/// Files this run applies, in application order: every `*.json` and
/// `*.js` file (a single file is itself the whole list; a directory is
/// its matching entries sorted by name).
fn collect_seed_files(path: &Path) -> Result<Vec<PathBuf>, AppError> {
    let meta = std::fs::metadata(path)
        .map_err(|e| AppError::bad_request(format!("{}: {e}", path.display())))?;
    if meta.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    let entries = std::fs::read_dir(path)
        .map_err(|e| AppError::bad_request(format!("{}: {e}", path.display())))?;
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && (has_extension(p, "json") || has_extension(p, "js")))
        .collect();
    files.sort();
    Ok(files)
}

fn has_extension(path: &Path, ext: &str) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some(ext)
}

/// Apply one JSON seed file's `{ "<collection>": [ {..record}, ... ] }`
/// onto `tx`, in the collection-key order `serde_json::Map` gives us
/// (alphabetical — see the module doc).
async fn apply_json_file(
    tx: &TxApp,
    file: &Path,
    upsert: bool,
    report: &mut SeedReport,
) -> Result<(), AppError> {
    let raw = std::fs::read_to_string(file)
        .map_err(|e| AppError::bad_request(format!("{}: {e}", file.display())))?;
    let doc: Map<String, Value> = serde_json::from_str(&raw)
        .map_err(|e| AppError::bad_request(format!("{}: invalid JSON: {e}", file.display())))?;

    for (collection_name, items) in &doc {
        let collection = tx.db().collections.get(collection_name).ok_or_else(|| {
            AppError::bad_request(format!(
                "{}: unknown collection '{collection_name}'",
                file.display()
            ))
        })?;
        let items = items.as_array().ok_or_else(|| {
            AppError::bad_request(format!(
                "{}: '{collection_name}' must be an array of records",
                file.display()
            ))
        })?;
        for (index, item) in items.iter().enumerate() {
            let input = item.as_object().ok_or_else(|| {
                AppError::bad_request(format!(
                    "{}: collection '{collection_name}' record[{index}]: must be a JSON object",
                    file.display()
                ))
            })?;
            let upserted = seed_one_record(tx, &collection, input, upsert)
                .await
                .map_err(|e| {
                    AppError::bad_request(format!(
                        "{}: collection '{collection_name}' record[{index}]: {e}",
                        file.display()
                    ))
                })?;
            if upserted {
                report.upserted += 1;
            } else {
                report.created += 1;
            }
        }
    }
    Ok(())
}

/// Create (or, with `upsert` and a matching existing id, update) one
/// record. Returns `true` when an existing record was updated.
async fn seed_one_record(
    tx: &TxApp,
    collection: &Arc<Collection>,
    input: &Map<String, Value>,
    upsert: bool,
) -> Result<bool, AppError> {
    let store = &tx.db().collections;
    let id = input.get("id").and_then(Value::as_str).unwrap_or("");
    if upsert && !id.is_empty() {
        if let Ok(mut existing) = records::find_by_id_raw(tx, collection, id).await {
            records::apply_body(&mut existing, input);
            records::update(tx, store, &mut existing)
                .await
                .map_err(AppError::from)?;
            return Ok(true);
        }
    }
    let mut record = records::from_body(collection.clone(), input);
    records::create(tx, store, &mut record)
        .await
        .map_err(AppError::from)?;
    Ok(false)
}

/// The JS runtime, starting one on demand (scoped to `app`'s configured
/// `pb_hooks`/`pb_migrations` directories, same as a normal boot would)
/// when `*.js` seed files are used but nothing started it already — e.g.
/// a non-`--dev` boot against an empty `pb_hooks`/`pb_migrations`.
async fn ensure_runtime(app: &App) -> Result<cratebase_jsvm::Runtime, AppError> {
    if let Some(rt) = app.jsvm() {
        return Ok(rt);
    }
    let hooks_dir = PathBuf::from(&app.config().hooks_dir);
    let migrations_dir = PathBuf::from(&app.config().migrations_dir);
    let cfg = cratebase_jsvm::RuntimeConfig::new(hooks_dir, migrations_dir);
    let runtime = cratebase_jsvm::Runtime::start(crate::jsvm_host::wrap_host(app.clone()), cfg)
        .await
        .map_err(|e| AppError::internal(format!("failed to start the JS runtime: {e}")))?;
    // Best-effort: if something else raced us and already set it, keep
    // using our freshly-started one for this call (it's still correct,
    // just briefly redundant) rather than erroring the whole seed run.
    let _ = app.jsvm_cell().set(runtime.clone());
    Ok(app.jsvm().unwrap_or(runtime))
}

/// Used by [`crate::routes::schema`]-style callers that want a plain
/// `BTreeMap` view of a seed report for JSON output (the CLI just prints
/// it; kept here so both stay in sync with the field names above).
impl SeedReport {
    pub fn to_summary(&self) -> BTreeMap<&'static str, usize> {
        let mut m = BTreeMap::new();
        m.insert("created", self.created);
        m.insert("upserted", self.upserted);
        m.insert("files", self.files.len());
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use cratebase_db::Executor;
    use tower::ServiceExt;

    async fn test_app() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let cfg = Config::memory(dir.path().join("pb_data"));
        let app = App::new(cfg);
        app.bootstrap().await.expect("bootstrap");
        (app, dir)
    }

    /// A `posts` collection with an `author` relation to `users`, so a
    /// seed run can prove relation ids resolve across files.
    async fn with_posts_collection(app: &App) {
        use cratebase_core::{Collection, CollectionType, Field, FieldKind};
        let users = app
            .db()
            .collections
            .get("users")
            .expect("users is a default system collection");
        let mut posts = Collection::new("posts", CollectionType::Base);
        posts.fields.push(Field::new(
            "title",
            FieldKind::Text {
                min: 0,
                max: 0,
                pattern: String::new(),
                autogenerate_pattern: String::new(),
                primary_key: false,
            },
        ));
        posts.fields.push(Field::new(
            "author",
            FieldKind::Relation {
                collection_id: users.id.clone(),
                cascade_delete: false,
                min_select: 0,
                max_select: 1,
            },
        ));
        posts.assign_field_ids();
        app.db()
            .collections
            .insert(&*app.db().engine, &posts)
            .await
            .expect("create posts collection");
    }

    fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, contents).expect("write seed file");
        path
    }

    #[tokio::test]
    async fn seeds_an_auth_user_and_a_related_record_and_login_works() {
        let (app, tmp) = test_app().await;
        with_posts_collection(&app).await;
        let seed_dir = tmp.path().join("seed");
        std::fs::create_dir_all(&seed_dir).unwrap();
        write(
            &seed_dir,
            "01-users.json",
            r#"{
                "users": [
                    { "id": "seeduser0000001", "email": "seed@example.com", "password": "supersecret123", "verified": true }
                ]
            }"#,
        );
        write(
            &seed_dir,
            "02-posts.json",
            r#"{
                "posts": [
                    { "id": "seedpost0000001", "title": "Hello", "author": "seeduser0000001" }
                ]
            }"#,
        );

        let report = run(&app, &seed_dir, false).await.expect("seed run");
        assert_eq!(report.created, 2);
        assert_eq!(report.upserted, 0);
        assert_eq!(report.files.len(), 2);

        let user = records::find_by_id_raw(
            app.db(),
            &app.db().collections.get("users").unwrap(),
            "seeduser0000001",
        )
        .await
        .expect("seeded user exists");
        assert_ne!(user.get("password").unwrap().as_str().unwrap(), "supersecret123");
        assert!(cratebase_auth::verify_password(
            "supersecret123",
            user.get("password").unwrap().as_str().unwrap()
        ));

        let post = records::find_by_id_raw(
            app.db(),
            &app.db().collections.get("posts").unwrap(),
            "seedpost0000001",
        )
        .await
        .expect("seeded post exists");
        assert_eq!(post.get("author").unwrap().as_str().unwrap(), "seeduser0000001");

        // Prove login actually works through the real HTTP path, not just
        // that a hash happens to verify in isolation.
        let router = crate::router(app.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/api/collections/users/auth-with-password")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "identity": "seed@example.com", "password": "supersecret123" })
                    .to_string(),
            ))
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "seeded user must be able to log in");
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json["token"].as_str().is_some_and(|t| !t.is_empty()));
    }

    #[tokio::test]
    async fn upsert_rerun_is_idempotent() {
        let (app, tmp) = test_app().await;
        let seed_dir = tmp.path().join("seed");
        std::fs::create_dir_all(&seed_dir).unwrap();
        write(
            &seed_dir,
            "users.json",
            r#"{ "users": [ { "id": "seeduser0000002", "email": "u2@example.com", "password": "supersecret123", "name": "one" } ] }"#,
        );

        run(&app, &seed_dir, true).await.expect("first seed run");
        let second = run(&app, &seed_dir, true).await.expect("second seed run");
        assert_eq!(second.created, 0, "an existing id must be upserted, not re-created");
        assert_eq!(second.upserted, 1);

        let count = app
            .db()
            .query_scalar(r#"SELECT COUNT(*) FROM "users" WHERE "id" = $1"#, &[cratebase_db::engine::Sql::Text("seeduser0000002".into())])
            .await
            .unwrap()
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        assert_eq!(count, 1, "re-running with --upsert must not duplicate the row");
    }

    #[tokio::test]
    async fn without_upsert_a_rerun_fails_instead_of_duplicating() {
        let (app, tmp) = test_app().await;
        let seed_dir = tmp.path().join("seed");
        std::fs::create_dir_all(&seed_dir).unwrap();
        write(
            &seed_dir,
            "users.json",
            r#"{ "users": [ { "id": "seeduser0000003", "email": "u3@example.com", "password": "supersecret123" } ] }"#,
        );
        run(&app, &seed_dir, false).await.expect("first seed run");
        let err = run(&app, &seed_dir, false)
            .await
            .expect_err("a second run without --upsert must fail on the id collision");
        assert!(err.to_string().contains("users.json"), "{err}");
    }

    #[tokio::test]
    async fn an_invalid_record_rolls_back_the_whole_run_and_names_the_culprit() {
        let (app, tmp) = test_app().await;
        let seed_dir = tmp.path().join("seed");
        std::fs::create_dir_all(&seed_dir).unwrap();
        write(
            &seed_dir,
            "01-users.json",
            r#"{ "users": [ { "id": "seeduser0000004", "email": "u4@example.com", "password": "supersecret123" } ] }"#,
        );
        // Missing password (required on the users auth collection) — the
        // second file's second record is invalid.
        write(
            &seed_dir,
            "02-more-users.json",
            r#"{ "users": [
                { "id": "seeduser0000005", "email": "u5@example.com", "password": "supersecret123" },
                { "id": "seeduser0000006", "email": "bad" }
            ] }"#,
        );

        let err = run(&app, &seed_dir, false)
            .await
            .expect_err("a missing required field must fail the run");
        let msg = err.to_string();
        assert!(msg.contains("02-more-users.json"), "{msg}");
        assert!(msg.contains("users"), "{msg}");
        assert!(msg.contains("record[1]"), "{msg}");

        let count = app
            .db()
            .query_scalar(r#"SELECT COUNT(*) FROM "users""#, &[])
            .await
            .unwrap()
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        assert_eq!(
            count, 0,
            "nothing from this run — including the valid records in earlier files — may be committed"
        );
    }

    #[tokio::test]
    async fn a_js_seed_file_runs_with_app_available() {
        let (app, tmp) = test_app().await;
        with_posts_collection(&app).await;
        let seed_dir = tmp.path().join("seed");
        std::fs::create_dir_all(&seed_dir).unwrap();
        write(
            &seed_dir,
            "seed.js",
            r#"migrate((app) => {
                const c = app.findCollectionByNameOrId("posts");
                const r = new Record(c);
                r.set("title", "from js seed");
                app.save(r);
            }, (app) => {});"#,
        );

        run(&app, &seed_dir, false).await.expect("js seed run");
        let count = app
            .db()
            .query_scalar(r#"SELECT COUNT(*) FROM "posts" WHERE "title" = 'from js seed'"#, &[])
            .await
            .unwrap()
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn unknown_collection_is_reported_with_file_context() {
        let (app, tmp) = test_app().await;
        let seed_dir = tmp.path().join("seed");
        std::fs::create_dir_all(&seed_dir).unwrap();
        write(&seed_dir, "bad.json", r#"{ "no_such_collection": [ {} ] }"#);
        let err = run(&app, &seed_dir, false).await.expect_err("unknown collection");
        assert!(err.to_string().contains("bad.json"));
        assert!(err.to_string().contains("no_such_collection"));
    }
}
