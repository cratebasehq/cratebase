//! Reference plugin: exercises both extension points in
//! [`crate::plugin::Plugin`] so there's a working example to copy when
//! building a real one (e.g. cron jobs, team management — see
//! `ROADMAP.md`).
//!
//! - `routes()`: mounts `GET /api/plugins/example/stats`, returning the
//!   record count for every collection.
//! - `scheduled_tasks()`: logs that same snapshot to stdout every 5
//!   minutes, proving the interval-scheduler extension point runs
//!   independently of any HTTP request.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use cratebase_db::{collections, records};
use serde_json::{json, Value};

use crate::plugin::{Plugin, ScheduledTask};
use crate::state::AppState;

pub struct ExamplePlugin;

async fn collection_stats(state: &AppState) -> Result<Value, cratebase_db::error::DbError> {
    let mut counts = serde_json::Map::new();
    for collection in collections::list_collections(&state.db).await? {
        let count = records::count_records(&state.db, &collection).await?;
        counts.insert(collection.name, json!(count));
    }
    Ok(json!({ "recordCounts": counts }))
}

async fn stats_handler(State(state): State<AppState>) -> Json<Value> {
    match collection_stats(&state).await {
        Ok(body) => Json(body),
        Err(err) => Json(json!({ "error": err.to_string() })),
    }
}

fn log_stats(state: AppState) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async move {
        match collection_stats(&state).await {
            Ok(body) => println!("[plugin:example] record counts: {body}"),
            Err(err) => eprintln!("[plugin:example] failed to collect stats: {err}"),
        }
    })
}

impl Plugin for ExamplePlugin {
    fn name(&self) -> &'static str {
        "example"
    }

    fn routes(&self) -> Option<Router<AppState>> {
        Some(Router::new().route("/stats", get(stats_handler)))
    }

    fn scheduled_tasks(&self) -> Vec<ScheduledTask> {
        vec![ScheduledTask {
            name: "log-record-counts",
            interval: Duration::from_secs(5 * 60),
            run: log_stats,
        }]
    }
}
