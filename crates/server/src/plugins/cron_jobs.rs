//! Cron-jobs plugin: real calendar cron expressions (`croner`), driven by
//! `ScheduledTask`'s fixed-interval mechanism — the trait itself stays
//! `Duration`-based (see `crate::plugin`'s doc comment for why), but this
//! plugin ticks every 30s and only actually *fires* a job whose stored
//! `schedule` cron expression matches the current minute.
//!
//! Job bodies are not arbitrary/scripted (Cratebase has no scripting
//! runtime): each `_cron_jobs` record names a `job` key from a small
//! built-in registry (`run_job` below). Adding a new job type means adding
//! a match arm here and shipping your own binary, same trade-off every
//! other plugin makes.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use cratebase_core::field::{Field, FieldOptions, FieldType};
use cratebase_core::{new_id, now, AuthOptions, Collection, CollectionType};
use cratebase_db::records::{self, update_record, ListParams};
use cratebase_db::resolver::{AuthContext, RequestContext};
use cratebase_db::{collections, Db, DbError, DbResult};
use croner::Cron;
use serde_json::{json, Map, Value};

use crate::plugin::{Plugin, ScheduledTask};
use crate::state::AppState;

const COLLECTION_NAME: &str = "_cron_jobs";
const TICK_INTERVAL: Duration = Duration::from_secs(30);

fn cron_jobs_collection() -> Collection {
    let ts = now();
    Collection {
        id: new_id(),
        name: COLLECTION_NAME.to_string(),
        collection_type: CollectionType::Base,
        schema: vec![
            Field {
                id: new_id(),
                name: "name".into(),
                field_type: FieldType::Text,
                required: true,
                unique: true,
                options: FieldOptions::default(),
            },
            Field {
                id: new_id(),
                name: "schedule".into(),
                field_type: FieldType::Text,
                required: true,
                unique: false,
                options: FieldOptions::default(),
            },
            Field {
                id: new_id(),
                name: "job".into(),
                field_type: FieldType::Text,
                required: true,
                unique: false,
                options: FieldOptions::default(),
            },
            Field {
                id: new_id(),
                name: "enabled".into(),
                field_type: FieldType::Bool,
                required: true,
                unique: false,
                options: FieldOptions::default(),
            },
            Field {
                id: new_id(),
                name: "lastRunAt".into(),
                field_type: FieldType::Date,
                required: false,
                unique: false,
                options: FieldOptions::default(),
            },
            Field {
                id: new_id(),
                name: "lastStatus".into(),
                field_type: FieldType::Text,
                required: false,
                unique: false,
                options: FieldOptions::default(),
            },
        ],
        // Job definitions are operational config, not app data: admin-only
        // end to end (`None` == superuser-only, same convention every
        // other collection uses).
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

/// Seed the `_cron_jobs` collection on first boot. No-op once it exists.
pub async fn ensure_cron_jobs_collection(db: &Db) -> DbResult<()> {
    match collections::get_collection_by_name(db, COLLECTION_NAME).await {
        Ok(_) => Ok(()),
        Err(DbError::NotFound) => collections::create_collection(db, &cron_jobs_collection()).await,
        Err(e) => Err(e),
    }
}

/// A synthetic internal identity for reads/writes the tick loop makes on
/// its own behalf — bypasses the admin-only rules the same way a real
/// superuser token would, since there is no HTTP caller to attach a rule
/// check to here.
fn system_auth() -> AuthContext {
    AuthContext {
        id: "system".into(),
        collection_id: String::new(),
        is_superuser: true,
        record: Map::new(),
    }
}

/// The built-in job body registry. `"log_stats"` mirrors
/// `plugins::example::log_stats` — proof the collection-driven scheduler
/// actually invokes real work, not just a no-op tick.
async fn run_job(state: &AppState, job: &str) -> Result<(), String> {
    match job {
        "log_stats" => {
            let mut counts = Map::new();
            for collection in collections::list_collections(&state.db)
                .await
                .map_err(|e| e.to_string())?
            {
                let count = records::count_records(&state.db, &collection)
                    .await
                    .map_err(|e| e.to_string())?;
                counts.insert(collection.name, json!(count));
            }
            println!("[plugin:cron-jobs] log_stats: {}", Value::Object(counts));
            Ok(())
        }
        other => Err(format!(
            "unknown job '{other}' — add a match arm in cron_jobs::run_job"
        )),
    }
}

/// Runs every `TICK_INTERVAL`, checks every enabled `_cron_jobs` record's
/// `schedule` against the current time, and fires the ones that match —
/// skipping a job already run within the current UTC minute so a 30s tick
/// interval never double-fires a once-a-minute schedule.
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
        let Ok(result) = records::list_records(
            &state.db,
            &collection,
            &ctx,
            None,
            ListParams {
                filter: None,
                sort: None,
                page: 1,
                per_page: 500,
            },
        )
        .await
        else {
            return;
        };

        let now_ts = chrono::Utc::now();
        for record in &result.items {
            if !record["enabled"].as_bool().unwrap_or(false) {
                continue;
            }
            let Some(schedule) = record["schedule"].as_str() else {
                continue;
            };
            let Ok(cron) = schedule.parse::<Cron>() else {
                eprintln!(
                    "[plugin:cron-jobs] job '{}': invalid cron expression '{schedule}'",
                    record["name"].as_str().unwrap_or("?")
                );
                continue;
            };
            if !cron.is_time_matching(&now_ts).unwrap_or(false) {
                continue;
            }
            if let Some(last_run) = record["lastRunAt"].as_str() {
                if let Ok(last_run) = chrono::DateTime::parse_from_rfc3339(last_run) {
                    let already_ran_this_minute = last_run
                        .with_timezone(&chrono::Utc)
                        .format("%Y-%m-%dT%H:%M")
                        .to_string()
                        == now_ts.format("%Y-%m-%dT%H:%M").to_string();
                    if already_ran_this_minute {
                        continue;
                    }
                }
            }

            let job = record["job"].as_str().unwrap_or_default();
            let outcome = run_job(&state, job).await;
            let mut patch = Map::new();
            patch.insert("lastRunAt".into(), json!(now()));
            patch.insert(
                "lastStatus".into(),
                json!(match &outcome {
                    Ok(()) => "ok".to_string(),
                    Err(e) => format!("error: {e}"),
                }),
            );
            let id = record["id"].as_str().unwrap_or_default();
            if let Err(e) = update_record(&state.db, &collection, id, patch).await {
                eprintln!("[plugin:cron-jobs] failed to record run for '{id}': {e}");
            }
        }
    })
}

pub struct CronJobsPlugin;

impl Plugin for CronJobsPlugin {
    fn name(&self) -> &'static str {
        "cron-jobs"
    }

    fn setup<'a>(
        &'a self,
        db: &'a Db,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            ensure_cron_jobs_collection(db).await?;
            Ok(())
        })
    }
    fn scheduled_tasks(&self) -> Vec<ScheduledTask> {
        vec![ScheduledTask {
            name: "cron-jobs-tick",
            interval: TICK_INTERVAL,
            run: tick,
        }]
    }
}
