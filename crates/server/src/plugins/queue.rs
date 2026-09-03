//! Queue plugin: durable background job processing, backed by an ordinary
//! collection instead of a separate broker — the job survives a process
//! restart because it's just a row (SQLite or Postgres, whichever
//! `DATABASE_URL` points at, no code change between them, same as
//! everything else in Cratebase).
//!
//! - `POST /api/plugins/queue/enqueue` is the only write path: it always
//!   sets `status`/`attempts`/`availableAt` itself, ignoring anything a
//!   caller supplies for them, the same reason `autodate` fields ignore
//!   client input — a job record's lifecycle fields are server state, not
//!   user input. The `_queue_jobs` collection's own rules are admin-only
//!   end to end so the generic records API can't be used to forge one.
//! - A `scheduled_tasks()` tick claims one due job, runs it through the
//!   small built-in `run_job` registry (same trade-off as
//!   `cron_jobs::run_job`: add a match arm, ship your own binary — no
//!   scripting runtime), and writes back `status`/`attempts`/`lastError`.
//!   Failed jobs are retried with exponential backoff up to `maxAttempts`,
//!   then parked as `"failed"`.
//! - A job stuck in `"processing"` because the process died mid-run is
//!   reclaimed after `STALE_AFTER` and retried — this is what makes it
//!   "durable" rather than "as long as the worker stays up."
//!
//! Single in-process worker, same scope decision as `RealtimeHub` and
//! `cron_jobs`: no distributed claim protocol, because there's no second
//! node to race with yet.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use cratebase_core::field::{Field, FieldOptions, FieldType};
use cratebase_core::{new_id, now, AppError, AuthOptions, Collection, CollectionType};
use cratebase_db::records::{self, update_record, ListParams};
use cratebase_db::resolver::{AuthContext, RequestContext};
use cratebase_db::{collections, Db, DbError, DbResult};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::extract::CurrentAuth;
use crate::http_error::{ApiError, ApiResult};
use crate::plugin::{Plugin, ScheduledTask};
use crate::state::AppState;

const COLLECTION_NAME: &str = "_queue_jobs";
const TICK_INTERVAL: Duration = Duration::from_secs(2);
const DEFAULT_MAX_ATTEMPTS: i64 = 5;
/// A job left `"processing"` longer than this is assumed to belong to a
/// worker that died mid-run and is put back in the queue.
const STALE_AFTER: chrono::Duration = chrono::Duration::minutes(5);

fn queue_jobs_collection() -> Collection {
    let ts = now();
    Collection {
        id: new_id(),
        name: COLLECTION_NAME.to_string(),
        collection_type: CollectionType::Base,
        schema: vec![
            Field {
                id: new_id(),
                name: "queue".into(),
                field_type: FieldType::Text,
                required: true,
                unique: false,
                options: FieldOptions::default(),
            },
            Field {
                id: new_id(),
                name: "payload".into(),
                field_type: FieldType::Json,
                required: false,
                unique: false,
                options: FieldOptions::default(),
            },
            Field {
                id: new_id(),
                name: "status".into(),
                field_type: FieldType::Select,
                required: true,
                unique: false,
                options: FieldOptions {
                    values: Some(
                        ["pending", "processing", "completed", "failed"]
                            .map(String::from)
                            .to_vec(),
                    ),
                    ..Default::default()
                },
            },
            Field {
                id: new_id(),
                name: "attempts".into(),
                field_type: FieldType::Number,
                required: true,
                unique: false,
                options: FieldOptions {
                    only_int: Some(true),
                    ..Default::default()
                },
            },
            Field {
                id: new_id(),
                name: "maxAttempts".into(),
                field_type: FieldType::Number,
                required: true,
                unique: false,
                options: FieldOptions {
                    only_int: Some(true),
                    ..Default::default()
                },
            },
            Field {
                id: new_id(),
                name: "availableAt".into(),
                field_type: FieldType::Date,
                required: true,
                unique: false,
                options: FieldOptions::default(),
            },
            Field {
                id: new_id(),
                name: "lastError".into(),
                field_type: FieldType::Text,
                required: false,
                unique: false,
                options: FieldOptions::default(),
            },
        ],
        // The only sanctioned write path is `POST .../queue/enqueue`
        // (constructs status/attempts/availableAt itself) and the worker
        // tick (both bypass these rules as internal calls) — admin-only
        // end to end so the generic records API can't forge a job.
        list_rule: None,
        view_rule: None,
        create_rule: None,
        update_rule: None,
        delete_rule: None,
        auth_options: AuthOptions::default(),
        view_query: None,
        created: ts.clone(),
        updated: ts,
    }
}

pub async fn ensure_queue_collection(db: &Db) -> DbResult<()> {
    match collections::get_collection_by_name(db, COLLECTION_NAME).await {
        Ok(_) => Ok(()),
        Err(DbError::NotFound) => {
            collections::create_collection(db, &queue_jobs_collection()).await
        }
        Err(e) => Err(e),
    }
}

fn system_auth() -> AuthContext {
    AuthContext {
        id: "system".into(),
        collection_id: String::new(),
        is_superuser: true,
        record: Map::new(),
    }
}

/// The built-in job body registry. `"log_message"` is a reference job
/// (logs its payload) proving enqueue → claim → run → status update works
/// end to end. Add a match arm here for a real job type.
async fn run_job(_state: &AppState, queue: &str, payload: &Value) -> Result<(), String> {
    match queue {
        "log_message" => {
            println!("[plugin:queue] log_message: {payload}");
            Ok(())
        }
        other => Err(format!(
            "unknown queue '{other}' — add a match arm in queue::run_job"
        )),
    }
}

#[derive(Deserialize)]
struct EnqueueRequest {
    queue: String,
    #[serde(default)]
    payload: Value,
    #[serde(rename = "maxAttempts")]
    max_attempts: Option<i64>,
}

fn forbidden() -> ApiError {
    ApiError(AppError::Forbidden(
        "you must be signed in to enqueue a job".into(),
    ))
}

/// `POST /api/plugins/queue/enqueue` — any authenticated caller (a
/// superuser or an auth-collection record; anonymous is rejected to avoid
/// an open job-spam endpoint). Returns the created job record.
async fn enqueue(
    State(state): State<AppState>,
    CurrentAuth(auth): CurrentAuth,
    Json(body): Json<EnqueueRequest>,
) -> ApiResult<Json<Value>> {
    if auth.is_none() {
        return Err(forbidden());
    }
    let collection = collections::get_collection_by_name(&state.db, COLLECTION_NAME)
        .await
        .map_err(|_| {
            ApiError(AppError::Internal(
                "queue collection not provisioned".into(),
            ))
        })?;

    let mut fields = Map::new();
    fields.insert("queue".into(), json!(body.queue));
    fields.insert("payload".into(), body.payload);
    fields.insert("status".into(), json!("pending"));
    fields.insert("attempts".into(), json!(0));
    fields.insert(
        "maxAttempts".into(),
        json!(body.max_attempts.unwrap_or(DEFAULT_MAX_ATTEMPTS)),
    );
    fields.insert("availableAt".into(), json!(now()));

    let record = records::create_record(&state.db, &collection, fields).await?;
    Ok(Json(record))
}

/// Computes the next `availableAt` for a retry: `2^attempts` seconds,
/// capped at 5 minutes so a flaky job doesn't wait an hour to try again.
fn backoff(attempts: i64) -> chrono::DateTime<chrono::Utc> {
    let seconds = 2i64.saturating_pow(attempts.clamp(0, 8) as u32).min(300);
    chrono::Utc::now() + chrono::Duration::seconds(seconds)
}

fn tick(state: AppState) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async move {
        let Ok(collection) = collections::get_collection_by_name(&state.db, COLLECTION_NAME).await
        else {
            return;
        };
        let ctx = RequestContext {
            auth: Some(system_auth()),
            data: None,
        };

        // Reclaim jobs a dead worker left stuck in "processing".
        let stale_cutoff = (chrono::Utc::now() - STALE_AFTER).to_rfc3339();
        if let Ok(stuck) = records::list_records(
            &state.db,
            &collection,
            &ctx,
            None,
            ListParams {
                filter: Some(&format!(
                    "status = \"processing\" && updated <= \"{stale_cutoff}\""
                )),
                sort: None,
                page: 1,
                per_page: 50,
            },
        )
        .await
        {
            for job in &stuck.items {
                let Some(id) = job["id"].as_str() else {
                    continue;
                };
                let mut patch = Map::new();
                patch.insert("status".into(), json!("pending"));
                let _ = update_record(&state.db, &collection, id, patch).await;
            }
        }

        // Claim and run one due job per tick.
        let now_iso = now();
        let Ok(due) = records::list_records(
            &state.db,
            &collection,
            &ctx,
            None,
            ListParams {
                filter: Some(&format!(
                    "status = \"pending\" && availableAt <= \"{now_iso}\""
                )),
                sort: Some("created"),
                page: 1,
                per_page: 1,
            },
        )
        .await
        else {
            return;
        };
        let Some(job) = due.items.first() else { return };
        let Some(id) = job["id"].as_str() else { return };

        let mut claim = Map::new();
        claim.insert("status".into(), json!("processing"));
        if update_record(&state.db, &collection, id, claim)
            .await
            .is_err()
        {
            return;
        }

        let queue = job["queue"].as_str().unwrap_or_default().to_string();
        let payload = job["payload"].clone();
        let attempts = job["attempts"].as_i64().unwrap_or(0) + 1;
        let max_attempts = job["maxAttempts"].as_i64().unwrap_or(DEFAULT_MAX_ATTEMPTS);

        let outcome = run_job(&state, &queue, &payload).await;
        let mut patch = Map::new();
        patch.insert("attempts".into(), json!(attempts));
        match outcome {
            Ok(()) => {
                patch.insert("status".into(), json!("completed"));
            }
            Err(err) => {
                patch.insert("lastError".into(), json!(err));
                if attempts >= max_attempts {
                    patch.insert("status".into(), json!("failed"));
                } else {
                    patch.insert("status".into(), json!("pending"));
                    patch.insert("availableAt".into(), json!(backoff(attempts).to_rfc3339()));
                }
            }
        }
        let _ = update_record(&state.db, &collection, id, patch).await;
    })
}

pub struct QueuePlugin;

impl Plugin for QueuePlugin {
    fn name(&self) -> &'static str {
        "queue"
    }

    fn setup<'a>(
        &'a self,
        db: &'a Db,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            ensure_queue_collection(db).await?;
            Ok(())
        })
    }

    fn routes(&self) -> Option<Router<AppState>> {
        Some(Router::new().route("/enqueue", post(enqueue)))
    }

    fn scheduled_tasks(&self) -> Vec<ScheduledTask> {
        vec![ScheduledTask {
            name: "queue-tick",
            interval: TICK_INTERVAL,
            run: tick,
        }]
    }
}
