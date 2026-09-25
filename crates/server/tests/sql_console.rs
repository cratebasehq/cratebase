//! Postgres-only proof that `POST /api/sql`'s read/write gate can't be
//! bypassed with a data-modifying CTE (see
//! `crates/server/src/routes/sql_console.rs`'s module doc, "The
//! read/write gate"): `is_read_statement` treats anything starting with
//! `WITH` as a read purely by syntax, so with `write: false` a
//! data-modifying CTE — `UPDATE "_superusers" SET "role" = 'admin'`
//! wrapped in a `WITH ... SELECT` — used to sail straight through to
//! `Executor::query` and run the `UPDATE` for real — on Postgres,
//! nothing at the database layer stopped a write from happening inside
//! what the caller declared a read-only request.
//!
//! The request below comes from an *owner* superuser deliberately: an
//! owner is allowed to change `_superusers` rows in general, so the
//! module's own `_superusers` guard (owner-only, `references_table` +
//! `RequireOwner::holds`) does not fire and cannot be the thing that
//! rejects this request. Only `PostgresEngine::query_interruptible`
//! running the statement inside `BEGIN READ ONLY` (see its doc) can
//! reject it — which is exactly the fix this test is pinned to. SQLite
//! never had this gap: its read pool runs under `PRAGMA query_only` (see
//! `crates/db/src/sqlite.rs`), which already stopped the write as a side
//! effect regardless of who ran it.
//!
//! Skips (rather than fails) when `TEST_POSTGRES_URL` is unset, matching
//! `crates/db/tests/postgres.rs`'s convention for Postgres-only tests.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use cratebase_db::engine::{Executor, Sql};
use cratebase_db::Db;
use cratebase_server::app::App;
use cratebase_server::config::Config;
use serde_json::{json, Value};
use tower::ServiceExt;

fn node_config(url: &str, dir: &std::path::Path) -> Config {
    Config {
        database_url: url.to_string(),
        log_requests: false,
        secret: "test-secret-0123456789".into(),
        ..Config::for_data_dir(dir)
    }
}

async fn split(response: Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("read body");
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, body)
}

#[tokio::test]
async fn sql_console_rejects_a_write_disguised_as_a_read_cte_via_postgres() {
    let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
        eprintln!(
            "skipping sql_console_rejects_a_write_disguised_as_a_read_cte_via_postgres: \
             TEST_POSTGRES_URL not set"
        );
        return;
    };

    // Fresh schema, same convention as `postgres_multi_node.rs`.
    {
        let dir = tempfile::tempdir().unwrap();
        let wipe = Db::connect(&url, &dir.path().to_string_lossy())
            .await
            .expect("connect to wipe schema");
        wipe.execute("DROP SCHEMA public CASCADE", &[])
            .await
            .unwrap();
        wipe.execute("CREATE SCHEMA public", &[]).await.unwrap();
        wipe.close().await.unwrap();
    }

    let dir = tempfile::tempdir().unwrap();
    let app = App::new(node_config(&url, dir.path()));
    app.bootstrap().await.expect("bootstrap");

    // `create_superuser` always mints an owner (see `App::create_superuser`
    // — the built-in bootstrap superuser and this one are both owners),
    // which matters here: an owner is allowed to write `_superusers` in
    // general, so the module's own owner-only guard does not fire and
    // cannot be what rejects this request — see the module doc.
    let owner_id = app
        .create_superuser("owner@example.com", "password12345")
        .await
        .expect("create owner superuser");
    let token = app
        .mint_token(
            "_superusers",
            &owner_id,
            cratebase_auth::TokenType::Auth,
            3600,
        )
        .await
        .expect("mint token");

    let cte = format!(
        "WITH x AS (UPDATE \"_superusers\" SET \"role\" = 'admin' \
         WHERE \"id\" = '{owner_id}' RETURNING 1) SELECT * FROM x"
    );
    let router = cratebase_server::router(app.clone());
    let (status, body) = split(
        router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/sql")
                    .header("authorization", &token)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({ "sql": cte, "write": false }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .expect("response"),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body:?}");

    let role = app
        .db()
        .query_scalar(
            r#"SELECT "role" FROM "_superusers" WHERE "id" = $1"#,
            &[Sql::from(owner_id.as_str())],
        )
        .await
        .expect("read back role")
        .and_then(|v| v.as_str().map(str::to_string))
        .expect("role column present");
    assert_eq!(role, "owner", "the CTE must not have changed the role");
}
