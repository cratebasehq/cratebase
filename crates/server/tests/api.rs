//! End-to-end tests for the W4a surface: the app lifecycle, the hook
//! chain, the middleware and the five live route groups.
//!
//! Everything runs against an in-memory SQLite database and a temp
//! directory, driven through `tower::ServiceExt::oneshot` — no listener,
//! no ports, so the whole file runs in parallel with everything else.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use cratebase_core::{AppError, Settings};
use cratebase_server::app::App;
use cratebase_server::config::Config;
use cratebase_server::hooks::{Event, Handler, Hook, HookResult};
use cratebase_server::plugin::FnPlugin;
use serde_json::{json, Value};
use tower::ServiceExt;

const SUPERUSER_EMAIL: &str = "admin@example.com";
const SUPERUSER_PASSWORD: &str = "hunter2hunter2";

/// A bootstrapped app plus the temp dir keeping its data alive.
struct Harness {
    app: App,
    token: String,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn new() -> Harness {
        Harness::with(|_| {}).await
    }

    /// `tweak` runs on the freshly built `App` before `bootstrap`, so a
    /// test can register hooks and plugins that boot with it.
    async fn with(tweak: impl FnOnce(&App)) -> Harness {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::new(Config::memory(dir.path()));
        tweak(&app);
        app.bootstrap().await.expect("bootstrap");
        let id = app
            .create_superuser(SUPERUSER_EMAIL, SUPERUSER_PASSWORD)
            .await
            .expect("superuser");
        let token = app
            .mint_token("_superusers", &id, cratebase_auth::TokenType::Auth, 3600)
            .await
            .expect("token");
        Harness {
            app,
            token,
            _dir: dir,
        }
    }

    fn router(&self) -> axum::Router {
        cratebase_server::router(self.app.clone())
    }

    async fn send(&self, request: Request<Body>) -> (StatusCode, Value) {
        let response = self.router().oneshot(request).await.expect("response");
        split(response).await
    }

    async fn get(&self, uri: &str) -> (StatusCode, Value) {
        self.send(Request::get(uri).body(Body::empty()).unwrap())
            .await
    }

    async fn get_auth(&self, uri: &str) -> (StatusCode, Value) {
        self.send(
            Request::get(uri)
                .header("authorization", &self.token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
    }

    async fn json_auth(&self, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
        self.send(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", &self.token)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
    }

    async fn raw_auth(&self, method: &str, uri: &str) -> Response {
        self.router()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("authorization", &self.token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("response")
    }
}

async fn split(response: Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

// --------------------------------------------------------------- lifecycle

#[tokio::test]
async fn bootstrap_opens_everything_and_refuses_a_second_call() {
    let harness = Harness::new().await;
    assert!(harness.app.is_bootstrapped());
    assert!(harness.app.try_db().is_some());
    assert!(harness.app.db().collections.get("_superusers").is_some());
    assert!(harness.app.db().collections.get("users").is_some());
    assert!(harness.app.bootstrap().await.is_err(), "second bootstrap");
}

#[tokio::test]
async fn plugins_run_at_bootstrap_and_reject_duplicates() {
    let ran = Arc::new(AtomicUsize::new(0));
    let counter = ran.clone();
    let harness = Harness::with(move |app| {
        let counter = counter.clone();
        app.register_plugin(FnPlugin::new("audit", move |_app| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }))
        .unwrap();
        // A second plugin with the same name is refused.
        assert!(app
            .register_plugin(FnPlugin::new("audit", |_| Ok(())))
            .is_err());
        // ... as is one named after a built-in route group.
        assert!(app
            .register_plugin(FnPlugin::new("settings", |_| Ok(())))
            .is_err());
    })
    .await;
    assert_eq!(ran.load(Ordering::SeqCst), 1);
    assert_eq!(harness.app.plugin_names(), ["audit"]);
}

#[tokio::test]
async fn run_in_transaction_commits_and_rolls_back() {
    use cratebase_db::engine::{Executor, Sql};

    let harness = Harness::new().await;
    let app = &harness.app;

    app.run_in_transaction(|tx| async move {
        tx.execute(
            r#"INSERT INTO "_params" ("id", "key", "value", "created", "updated") VALUES ($1, $2, $3, '', '')"#,
            &[Sql::from("id1"), Sql::from("committed"), Sql::from("yes")],
        )
        .await?;
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(
        cratebase_db::params::get(app.db(), "committed")
            .await
            .unwrap(),
        Some("yes".into())
    );

    let failed: Result<(), AppError> = app
        .run_in_transaction(|tx| async move {
            tx.execute(
                r#"INSERT INTO "_params" ("id", "key", "value", "created", "updated") VALUES ($1, $2, $3, '', '')"#,
                &[Sql::from("id2"), Sql::from("rolled-back"), Sql::from("no")],
            )
            .await?;
            Err(AppError::bad_request("nope"))
        })
        .await;
    assert!(failed.is_err());
    assert_eq!(
        cratebase_db::params::get(app.db(), "rolled-back")
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn a_bearer_token_is_resolved_exactly_once_per_request() {
    // The Phase 1 regression this guards: the logging layer resolved the
    // caller, then the handler's extractor resolved it again, doubling
    // the JWT verify + database round trip on every authenticated call.
    let harness = Harness::new().await;
    let before = harness.app.auth_resolutions();

    // `/api/settings` passes through the log layer, the rate-limit layer,
    // `RequireSuperuser` and `RequestInfo` — four askers, one resolution.
    let (status, _) = harness.get_auth("/api/settings").await;
    assert_eq!(status, 200);
    assert_eq!(
        harness.app.auth_resolutions() - before,
        1,
        "one authenticated request must cost exactly one token resolution"
    );

    // An anonymous request resolves nothing at all.
    let before = harness.app.auth_resolutions();
    assert_eq!(harness.get("/api/health").await.0, 200);
    assert_eq!(harness.app.auth_resolutions() - before, 0);
}

// -------------------------------------------------------------- hook chain

/// A minimal event so the chain can be exercised without a database.
#[derive(Default)]
struct Probe {
    hook_chain: cratebase_server::hooks::Chain<Self>,
    log: Arc<Mutex<Vec<String>>>,
}
cratebase_server::impl_event!(Probe);

fn note(name: &'static str) -> Handler<Probe> {
    Handler::new(move |e: &mut Probe| {
        Box::pin(async move {
            e.log.lock().unwrap().push(name.to_string());
            e.next().await
        })
    })
}

#[tokio::test]
async fn the_hook_chain_runs_in_order_and_a_handler_can_abort_it() {
    let hook: Hook<Probe> = Hook::new();
    hook.bind(note("second").with_priority(0));
    hook.bind(note("first").with_priority(-1));
    hook.bind(note("third").with_id("third").with_priority(1));

    let log = Arc::new(Mutex::new(Vec::new()));
    let mut event = Probe {
        log: log.clone(),
        ..Default::default()
    };
    hook.trigger(&mut event, |e| {
        Box::pin(async move {
            e.log.lock().unwrap().push("framework".into());
            Ok(())
        })
    })
    .await
    .unwrap();
    assert_eq!(
        *log.lock().unwrap(),
        ["first", "second", "third", "framework"]
    );

    // Removing a handler by id takes it out of the next trigger.
    assert_eq!(hook.unbind("third"), 1);
    log.lock().unwrap().clear();
    let mut event = Probe {
        log: log.clone(),
        ..Default::default()
    };
    hook.trigger_bare(&mut event).await.unwrap();
    assert_eq!(*log.lock().unwrap(), ["first", "second"]);

    // A handler that never calls `next()` stops everything after it,
    // including the framework's own action.
    let hook: Hook<Probe> = Hook::new();
    hook.bind_func(|e: &mut Probe| {
        Box::pin(async move {
            e.log.lock().unwrap().push("stop".into());
            Ok(())
        })
    });
    hook.bind(note("unreachable").with_priority(1));
    log.lock().unwrap().clear();
    let mut event = Probe {
        log: log.clone(),
        ..Default::default()
    };
    hook.trigger(&mut event, |e| {
        Box::pin(async move {
            e.log.lock().unwrap().push("framework".into());
            Ok(())
        })
    })
    .await
    .unwrap();
    assert_eq!(*log.lock().unwrap(), ["stop"]);
}

#[tokio::test]
async fn a_settings_hook_can_veto_the_update() {
    let harness = Harness::with(|app| {
        app.hooks().on_settings_update_request.bind_func(
            |_e| -> futures::future::BoxFuture<'_, HookResult> {
                Box::pin(async { Err(AppError::forbidden("settings are frozen")) })
            },
        );
    })
    .await;

    let (status, body) = harness
        .json_auth(
            "PATCH",
            "/api/settings",
            json!({"meta": {"appName": "Nope"}}),
        )
        .await;
    assert_eq!(status, 403);
    assert_eq!(body["message"], "settings are frozen");
    assert_eq!(harness.app.settings().meta.app_name, "Acme");
}

// ------------------------------------------------------------------ health

#[tokio::test]
async fn health_matches_the_pocketbase_envelope() {
    let harness = Harness::new().await;

    let (status, body) = harness.get("/api/health").await;
    assert_eq!(status, 200);
    // `code`, not `status` — the one endpoint that kept the old key.
    assert_eq!(body["code"], 200);
    assert_eq!(body["message"], "API is healthy.");
    assert_eq!(body["data"], json!({}));

    let (status, body) = harness.get_auth("/api/health").await;
    assert_eq!(status, 200);
    assert!(body["data"]["canBackup"].is_boolean());
    assert!(body["data"]["realIP"].is_string());
    assert!(body["data"].get("possibleProxyHeader").is_some());
}

// ---------------------------------------------------------------- settings

#[tokio::test]
async fn settings_round_trip_keeps_secrets_out_of_responses() {
    let harness = Harness::new().await;

    let (status, body) = harness.get_auth("/api/settings").await;
    assert_eq!(status, 200);
    let mut keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "backups",
            "batch",
            "llm",
            "logs",
            "meta",
            "rateLimits",
            "s3",
            "smtp",
            "superuserIPs",
            "trustedProxy",
        ]
    );
    assert!(body["smtp"].get("password").is_none());
    assert!(body["s3"].get("secret").is_none());
    assert!(body["backups"]["s3"].get("secret").is_none());

    // Store a secret, then patch an unrelated key: the secret survives
    // because the merge is deep and the client never echoed it back.
    let mut with_secret = Settings::default();
    with_secret.smtp.password = "hunter2".into();
    harness.app.set_settings(with_secret).await.unwrap();

    let (status, body) = harness
        .json_auth(
            "PATCH",
            "/api/settings",
            json!({"meta": {"appName": "Renamed"}, "logs": {"maxDays": 9}}),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(body["meta"]["appName"], "Renamed");
    assert_eq!(body["logs"]["maxDays"], 9);
    assert!(body["smtp"].get("password").is_none());
    assert_eq!(harness.app.settings().smtp.password, "hunter2");
    // Untouched keys keep their values.
    assert_eq!(body["meta"]["senderAddress"], "support@example.com");

    // ...and the change is hot: no restart, the next GET sees it.
    let (_, body) = harness.get_auth("/api/settings").await;
    assert_eq!(body["meta"]["appName"], "Renamed");
}

#[tokio::test]
async fn settings_validation_reports_a_nested_tree() {
    let harness = Harness::new().await;
    let (status, body) = harness
        .json_auth(
            "PATCH",
            "/api/settings",
            json!({"meta": {"appName": ""}, "logs": {"maxDays": -1}}),
        )
        .await;
    assert_eq!(status, 400);
    assert_eq!(
        body["message"],
        "An error occurred while saving the new settings."
    );
    assert_eq!(
        body["data"]["meta"]["appName"]["code"],
        "validation_required"
    );
    assert!(body["data"]["logs"]["maxDays"].is_object());
    // Nothing was persisted.
    assert_eq!(harness.app.settings().meta.app_name, "Acme");
}

#[tokio::test]
async fn test_s3_reports_a_disabled_filesystem() {
    let harness = Harness::new().await;
    let (status, body) = harness
        .json_auth(
            "POST",
            "/api/settings/test/s3",
            json!({"filesystem": "storage"}),
        )
        .await;
    assert_eq!(status, 400);
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains("Failed to test the S3 filesystem."),
        "{message}"
    );
    assert!(
        message.contains("S3 storage filesystem is not enabled."),
        "{message}"
    );
}

#[tokio::test]
async fn test_email_validates_its_arguments() {
    let harness = Harness::new().await;
    let (status, body) = harness
        .json_auth(
            "POST",
            "/api/settings/test/email",
            json!({"collection": "_superusers", "email": "not-an-email", "template": "verification"}),
        )
        .await;
    assert_eq!(status, 400);
    assert_eq!(body["data"]["email"]["code"], "validation_is_email");

    let (status, body) = harness
        .json_auth(
            "POST",
            "/api/settings/test/email",
            json!({"collection": "_superusers", "email": "a@b.co", "template": "bogus"}),
        )
        .await;
    assert_eq!(status, 400);
    assert!(body["data"]["template"].is_object());

    // A valid request goes through the (log-backed) mailer.
    let (status, _) = harness
        .json_auth(
            "POST",
            "/api/settings/test/email",
            json!({"collection": "_superusers", "email": "a@b.co", "template": "verification"}),
        )
        .await;
    assert_eq!(status, 204);
}

// -------------------------------------------------------------------- logs

#[tokio::test]
async fn requests_are_logged_then_listed_filtered_and_fetched() {
    let harness = Harness::new().await;

    // A success and a failure, so both slog levels are exercised.
    assert_eq!(harness.get("/api/health").await.0, 200);
    assert_eq!(harness.get("/api/not-a-route").await.0, 404);
    harness.app.logger().flush().await;

    let (status, body) = harness.get_auth("/api/logs?perPage=50").await;
    assert_eq!(status, 200);
    let mut keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        ["items", "page", "perPage", "totalItems", "totalPages"]
    );
    assert!(body["totalItems"].as_i64().unwrap() >= 2);

    let items = body["items"].as_array().unwrap();
    let health = items
        .iter()
        .find(|i| i["message"] == "GET /api/health")
        .expect("the health request was logged");
    let mut row_keys: Vec<&str> = health
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    row_keys.sort();
    assert_eq!(row_keys, ["created", "data", "id", "level", "message"]);
    assert_eq!(health["level"], 0);
    assert_eq!(health["data"]["type"], "request");
    assert_eq!(health["data"]["method"], "GET");
    assert_eq!(health["data"]["url"], "/api/health");
    assert_eq!(health["data"]["status"], 200);
    assert_eq!(health["data"]["auth"], "");
    assert!(health["data"]["execTime"].is_number());
    assert!(
        health["data"].get("error").is_none(),
        "a 2xx row has no error"
    );
    assert!(health["id"].as_str().unwrap().len() == 15);

    let failure = items
        .iter()
        .find(|i| i["message"] == "GET /api/not-a-route")
        .expect("the 404 was logged");
    assert_eq!(failure["level"], 8, "a failed request logs at error level");
    assert_eq!(failure["data"]["status"], 404);
    assert_eq!(
        failure["data"]["error"],
        AppError::DEFAULT_NOT_FOUND,
        "the error message travels on the response extensions"
    );

    // Filters compile against the synthetic log schema.
    let (status, filtered) = harness
        .get_auth(r#"/api/logs?filter=data.url%20%3D%20%22%2Fapi%2Fhealth%22"#)
        .await;
    assert_eq!(status, 200);
    assert!(filtered["totalItems"].as_i64().unwrap() >= 1);
    for item in filtered["items"].as_array().unwrap() {
        assert_eq!(item["data"]["url"], "/api/health");
    }

    let (status, by_level) = harness
        .get_auth("/api/logs?filter=level%20%3E%3D%208")
        .await;
    assert_eq!(status, 200);
    for item in by_level["items"].as_array().unwrap() {
        assert_eq!(item["level"], 8);
    }

    // getOne + 404.
    let id = health["id"].as_str().unwrap();
    let (status, one) = harness.get_auth(&format!("/api/logs/{id}")).await;
    assert_eq!(status, 200);
    assert_eq!(one["id"], id);
    let (status, missing) = harness.get_auth("/api/logs/nonexistent0001").await;
    assert_eq!(status, 404);
    assert_eq!(missing["message"], AppError::DEFAULT_NOT_FOUND);

    // Stats are hourly buckets.
    let (status, stats) = harness.get_auth("/api/logs/stats").await;
    assert_eq!(status, 200);
    let buckets = stats.as_array().unwrap();
    assert!(!buckets.is_empty());
    for bucket in buckets {
        let mut keys: Vec<&str> = bucket
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort();
        assert_eq!(keys, ["date", "total"]);
        let date = bucket["date"].as_str().unwrap();
        assert!(date.ends_with(":00:00.000Z"), "{date}");
    }

    // A malformed filter is a 400, not a 500.
    let (status, _) = harness.get_auth("/api/logs?filter=nonsense%20field").await;
    assert_eq!(status, 400);
}

// ------------------------------------------------------------------- crons

#[tokio::test]
async fn crons_list_the_system_jobs_and_run_on_demand() {
    let harness = Harness::new().await;

    let (status, jobs) = harness.get_auth("/api/crons").await;
    assert_eq!(status, 200);
    let by_id: std::collections::HashMap<&str, &str> = jobs
        .as_array()
        .unwrap()
        .iter()
        .map(|j| (j["id"].as_str().unwrap(), j["expression"].as_str().unwrap()))
        .collect();
    assert_eq!(by_id["__pbDBOptimize__"], "0 0 * * *");
    assert_eq!(by_id["__pbMFACleanup__"], "0 * * * *");
    assert_eq!(by_id["__pbOTPCleanup__"], "0 * * * *");
    assert_eq!(by_id["__pbLogsCleanup__"], "0 */6 * * *");
    // The auto-backup job only exists once a schedule is configured.
    assert!(!by_id.contains_key("__pbAutoBackup__"));
    for job in jobs.as_array().unwrap() {
        let mut keys: Vec<&str> = job
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort();
        assert_eq!(keys, ["expression", "id"]);
    }

    let response = harness
        .raw_auth("POST", "/api/crons/__pbLogsCleanup__")
        .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let (status, body) = harness
        .json_auth("POST", "/api/crons/__nope__", json!({}))
        .await;
    assert_eq!(status, 404);
    assert_eq!(
        body,
        json!({"status": 404, "message": "Missing or invalid cron job.", "data": {}})
    );
}

#[tokio::test]
async fn the_auto_backup_job_follows_the_settings() {
    let harness = Harness::new().await;
    let mut settings = Settings::default();
    settings.backups.cron = "0 3 * * *".into();
    harness.app.set_settings(settings).await.unwrap();

    let (_, jobs) = harness.get_auth("/api/crons").await;
    let backup = jobs
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["id"] == "__pbAutoBackup__")
        .expect("__pbAutoBackup__ is registered");
    assert_eq!(backup["expression"], "0 3 * * *");

    // Clearing the schedule removes it again.
    harness.app.set_settings(Settings::default()).await.unwrap();
    let (_, jobs) = harness.get_auth("/api/crons").await;
    assert!(!jobs
        .as_array()
        .unwrap()
        .iter()
        .any(|j| j["id"] == "__pbAutoBackup__"));
}

// ----------------------------------------------------------------- backups

#[tokio::test]
async fn backups_create_list_download_and_delete() {
    let dir = tempfile::tempdir().unwrap();
    // A real file-backed database, so there is something to snapshot.
    let mut config = Config::for_data_dir(dir.path());
    config.secret = "test-secret-0123456789".into();
    let app = App::new(config);
    app.bootstrap().await.unwrap();
    let id = app
        .create_superuser(SUPERUSER_EMAIL, SUPERUSER_PASSWORD)
        .await
        .unwrap();
    let token = app
        .mint_token("_superusers", &id, cratebase_auth::TokenType::Auth, 3600)
        .await
        .unwrap();
    let file_token = app
        .mint_token("_superusers", &id, cratebase_auth::TokenType::File, 3600)
        .await
        .unwrap();
    let router = || cratebase_server::router(app.clone());

    let created = router()
        .oneshot(
            Request::post("/api/backups")
                .header("authorization", &token)
                .header("content-type", "application/json")
                .body(Body::from(json!({"name": "conformance.zip"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::NO_CONTENT);

    let (status, list) = split(
        router()
            .oneshot(
                Request::get("/api/backups")
                    .header("authorization", &token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, 200);
    let entry = list
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["key"] == "conformance.zip")
        .expect("the backup is listed");
    let mut keys: Vec<&str> = entry
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(keys, ["key", "modified", "size"]);
    let size = entry["size"].as_u64().unwrap();
    assert!(size > 0);

    // Downloading needs a superuser file token in the query string.
    let denied = router()
        .oneshot(
            Request::get("/api/backups/conformance.zip")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied.status(), 403);
    let (_, body) = split(denied).await;
    assert_eq!(
        body["message"],
        "Insufficient permissions to access the resource."
    );

    let downloaded = router()
        .oneshot(
            Request::get(format!("/api/backups/conformance.zip?token={file_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(downloaded.status(), 200);
    assert_eq!(downloaded.headers()["content-type"], "application/zip");
    let bytes = to_bytes(downloaded.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes.len() as u64, size);
    assert_eq!(&bytes[..2], b"PK", "zip magic");

    // A duplicate name and a traversal attempt are both rejected.
    let (status, body) = split(
        router()
            .oneshot(
                Request::post("/api/backups")
                    .header("authorization", &token)
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"name": "conformance.zip"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(
        body["data"]["name"]["code"],
        "validation_backup_name_exists"
    );

    let (status, body) = split(
        router()
            .oneshot(
                Request::post("/api/backups")
                    .header("authorization", &token)
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"name": "../escape.zip"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, 400);
    assert!(body["data"]["name"].is_object());

    // Deleting a missing one is a 400 with PocketBase's wording.
    let (status, body) = split(
        router()
            .oneshot(
                Request::delete("/api/backups/missing.zip")
                    .header("authorization", &token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, 400);
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("Invalid or already deleted backup file."));

    let removed = router()
        .oneshot(
            Request::delete("/api/backups/conformance.zip")
                .header("authorization", &token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);

    let (_, list) = split(
        router()
            .oneshot(
                Request::get("/api/backups")
                    .header("authorization", &token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert!(!list
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b["key"] == "conformance.zip"));

    // Restoring something that isn't there never touches the data dir.
    let (status, body) = split(
        router()
            .oneshot(
                Request::post("/api/backups/missing.zip/restore")
                    .header("authorization", &token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(body["message"], "Missing or invalid backup file.");

    app.terminate(false).await;
}

// ------------------------------------------------------------------ errors

#[tokio::test]
async fn errors_use_the_pocketbase_envelope() {
    let harness = Harness::new().await;

    // 404: unknown route.
    let (status, body) = harness.get("/api/not-a-route").await;
    assert_eq!(status, 404);
    assert_eq!(
        body,
        json!({"status": 404, "message": AppError::DEFAULT_NOT_FOUND, "data": {}})
    );

    // 404 (not 405): the wrong method on a route that exists.
    let (status, body) = harness
        .send(Request::delete("/api/health").body(Body::empty()).unwrap())
        .await;
    assert_eq!(status, 404, "PocketBase answers 404, not 405");
    assert_eq!(body["data"], json!({}));

    // 401: an admin endpoint without a token.
    let (status, body) = harness.get("/api/settings").await;
    assert_eq!(status, 401);
    assert_eq!(
        body,
        json!({"status": 401, "message": AppError::DEFAULT_UNAUTHORIZED, "data": {}})
    );

    // 401: a token that is well-formed but not signed with the record key.
    let (status, _) = harness
        .send(
            Request::get("/api/settings")
                .header("authorization", "Bearer not.a.token")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, 401);

    // 403: a superuser whose IP is not on the allowlist.
    let settings = Settings {
        superuser_ips: vec!["10.99.99.99".into()],
        ..Settings::default()
    };
    harness.app.set_settings(settings).await.unwrap();
    let (status, body) = harness.get_auth("/api/settings").await;
    assert_eq!(status, 403);
    assert_eq!(body["message"], AppError::DEFAULT_FORBIDDEN);
    harness.app.set_settings(Settings::default()).await.unwrap();

    // 400: a malformed JSON body is PocketBase-shaped, not axum's text.
    let (status, body) = harness
        .send(
            Request::patch("/api/settings")
                .header("authorization", &harness.token)
                .header("content-type", "application/json")
                .body(Body::from("{not json"))
                .unwrap(),
        )
        .await;
    assert_eq!(status, 400);
    assert!(body["message"].is_string());
    assert_eq!(body["data"], json!({}));
}

#[tokio::test]
async fn the_rate_limiter_returns_pocketbases_429() {
    let harness = Harness::new().await;
    let mut settings = Settings::default();
    settings.rate_limits.enabled = true;
    settings.rate_limits.rules = vec![cratebase_core::settings::RateLimitRule {
        label: "/api/health".into(),
        audience: String::new(),
        duration: 60,
        max_requests: 2,
    }];
    harness.app.set_settings(settings).await.unwrap();

    assert_eq!(harness.get("/api/health").await.0, 200);
    assert_eq!(harness.get("/api/health").await.0, 200);
    let (status, body) = harness.get("/api/health").await;
    assert_eq!(status, 429);
    assert_eq!(
        body,
        json!({"status": 429, "message": "Too Many Requests.", "data": {}})
    );

    // A rule that does not cover the path leaves it alone.
    assert_eq!(harness.get("/api/not-a-route").await.0, 404);
}

#[tokio::test]
async fn a_spoofed_forwarded_header_cannot_dodge_the_rate_limit() {
    let harness = Harness::new().await;
    let mut settings = Settings::default();
    settings.rate_limits.enabled = true;
    settings.rate_limits.rules = vec![cratebase_core::settings::RateLimitRule {
        label: "/api/health".into(),
        audience: String::new(),
        duration: 60,
        max_requests: 1,
    }];
    harness.app.set_settings(settings).await.unwrap();

    // No trusted proxy is configured, so every one of these is the same
    // (empty, in-process) client however creative the header is.
    assert_eq!(harness.get("/api/health").await.0, 200);
    for spoof in ["1.1.1.1", "2.2.2.2", "3.3.3.3"] {
        let (status, _) = harness
            .send(
                Request::get("/api/health")
                    .header("x-forwarded-for", spoof)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(
            status, 429,
            "X-Forwarded-For: {spoof} must not reset the window"
        );
    }

    // Once the operator declares the header, it is honoured and each
    // forwarded IP gets its own budget.
    harness.app.rate_limiter().reset();
    let mut settings = harness.app.settings().as_ref().clone();
    settings.trusted_proxy.headers = vec!["X-Forwarded-For".into()];
    harness.app.set_settings(settings).await.unwrap();
    for distinct in ["1.1.1.1", "2.2.2.2", "3.3.3.3"] {
        let (status, _) = harness
            .send(
                Request::get("/api/health")
                    .header("x-forwarded-for", distinct)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(status, 200, "{distinct} has its own window");
    }
}

// -------------------------------------------------------------- superusers

#[tokio::test]
async fn the_superuser_cli_path_creates_a_usable_account() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::new(Config::memory(dir.path()));
    app.bootstrap().await.unwrap();

    let id = app
        .create_superuser("cli@example.com", "correct horse")
        .await
        .unwrap();
    assert_eq!(id.len(), 15);
    let row = app
        .find_superuser_by_email("cli@example.com")
        .await
        .unwrap()
        .expect("the row exists");
    let hash = row.get_str("password").unwrap();
    assert!(cratebase_auth::verify_password("correct horse", hash));
    assert_ne!(hash, "correct horse", "passwords are hashed");
    assert_eq!(row.get_str("tokenKey").unwrap().len(), 50);

    // A token minted for it authenticates.
    let token = app
        .mint_token("_superusers", &id, cratebase_auth::TokenType::Auth, 3600)
        .await
        .unwrap();
    let (status, _) = split(
        cratebase_server::router(app.clone())
            .oneshot(
                Request::get("/api/settings")
                    .header("authorization", &token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, 200);

    // Changing the password rotates `tokenKey`, so the old token dies.
    assert!(app
        .set_superuser_password("cli@example.com", "a different one")
        .await
        .unwrap());
    let (status, _) = split(
        cratebase_server::router(app.clone())
            .oneshot(
                Request::get("/api/settings")
                    .header("authorization", &token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        status, 401,
        "rotating tokenKey invalidates existing sessions"
    );

    assert!(app.delete_superuser("cli@example.com").await.unwrap());
    assert!(!app.delete_superuser("cli@example.com").await.unwrap());
}

// ------------------------------------------------------------------ plugins

#[tokio::test]
async fn plugin_routes_live_inside_the_api_nest() {
    struct Echo;
    impl cratebase_server::plugin::Plugin for Echo {
        fn name(&self) -> &str {
            "echo"
        }
        fn routes(&self) -> Option<axum::Router<App>> {
            Some(axum::Router::new().route(
                "/ping",
                axum::routing::get(|| async { axum::Json(json!({"pong": true})) }),
            ))
        }
    }

    let harness = Harness::with(|app| app.register_plugin(Echo).unwrap()).await;
    let (status, body) = harness.get("/api/plugins/echo/ping").await;
    assert_eq!(status, 200);
    assert_eq!(body["pong"], true);

    // ...which means the request-log middleware saw it.
    harness.app.logger().flush().await;
    let (_, logs) = harness
        .get_auth("/api/logs?filter=data.url%20%7E%20%22plugins%22")
        .await;
    assert!(
        logs["totalItems"].as_i64().unwrap() >= 1,
        "plugin routes must be logged like every other /api route"
    );
}
