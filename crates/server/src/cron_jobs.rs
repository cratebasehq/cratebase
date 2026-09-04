//! Custom cron jobs (`_cron_jobs`): schedule an arbitrary SQL statement to
//! run on a cron expression, entirely from the dashboard or the generic
//! Records API — no Rust code, no redeploy.
//!
//! # Trust boundary
//!
//! `_cron_jobs` is superuser-only end to end (every rule stays at
//! `Collection::new`'s default `None` — see
//! `cratebase_core::Collection::default_system_collections`). A job's
//! `sql` runs with no rule enforcement in between, the same tier as a
//! collection schema edit: this is an operator tool, not something a
//! `type: "auth"` record should ever reach.
//!
//! # Why registration is reactive, not just at boot
//!
//! [`crate::cron::CronService`] is an in-process registry with no
//! knowledge of `_cron_jobs` rows on its own. [`sync_all`] loads every row
//! once at [`crate::app::App::bootstrap`]; after that, two hooks tagged to
//! `_cron_jobs` — `on_record_after_create_success` and
//! `on_record_after_update_success` — call [`sync_one`] and
//! `on_record_after_delete_success` calls [`unsync`], so a dashboard edit
//! takes effect immediately, with no restart.
//!
//! A third, earlier hook — `on_record_create`/`on_record_update`, before
//! the row is even written — validates `expression` as a real 5-field
//! cron string and rejects the write with a normal field-error 400
//! otherwise; a bad expression never reaches the scheduler at all.
//!
//! # Why the status write-back bypasses the record API
//!
//! [`run_custom_job`] writes `lastRunAt`/`lastStatus`/`lastMessage` back
//! with a raw `UPDATE`, not `records::update`. Going through the record
//! API would re-fire the very hooks that call [`sync_one`] — harmless
//! (`add_boxed` is register-or-replace) but pointless, and it would also
//! run full field validation for a write that only ever touches three
//! system-owned columns.

use std::future::Future;
use std::pin::Pin;

use cratebase_core::codes::REQUIRED;
use cratebase_core::{AppError, DateTime, FieldError};
use cratebase_db::engine::{Executor, Row, Sql};

use crate::app::App;
use crate::events::RecordEvent;
use crate::hooks::{Event, Handler};

const COLLECTION: &str = "_cron_jobs";

/// Job ids for custom jobs are namespaced so they can never collide with
/// the four built-in system job ids (`__pbDBOptimize__` and friends).
fn job_id(record_id: &str) -> String {
    format!("custom:{record_id}")
}

/// Load every `_cron_jobs` row and (re-)register or unregister it with
/// [`crate::cron::CronService`]. Called once at boot; safe to call again
/// (idempotent — `add_boxed` replaces, `remove` on a missing id is a
/// no-op).
pub async fn sync_all(app: &App) {
    let rows = match app
        .db()
        .query(&format!(r#"SELECT * FROM "{COLLECTION}""#), &[])
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "failed to load custom cron jobs at boot");
            return;
        }
    };
    for row in &rows {
        sync_one(app, row);
    }
}

/// Register `row` with the scheduler if `enabled`, otherwise unregister
/// it. Never fails the caller: an invalid cron expression is logged and
/// the row is simply not scheduled rather than rejected here — the
/// create/update hook below rejects a bad expression before it can ever
/// reach a stored row in the first place, so this path only degrades for
/// data written some other way (a direct SQL insert, say).
fn sync_one(app: &App, row: &Row) {
    let Some(id) = row.get_str("id") else { return };
    let job = job_id(id);
    let enabled = row.get("enabled").and_then(Sql::as_i64).unwrap_or(0) != 0;
    if !enabled {
        app.cron().remove(&job);
        return;
    }
    let Some(expression) = row.get_str("expression") else {
        app.cron().remove(&job);
        return;
    };
    let sql = row.get_str("sql").unwrap_or_default().to_string();
    let record_id = id.to_string();
    if let Err(e) = app.cron().add(job.clone(), expression, {
        let app = app.clone();
        let sql = sql.clone();
        let record_id = record_id.clone();
        move || run_custom_job(app.clone(), record_id.clone(), sql.clone())
    }) {
        tracing::warn!(error = %e, job = %job, "invalid custom cron expression, not scheduled");
    }
}

fn unsync(app: &App, record_id: &str) {
    app.cron().remove(&job_id(record_id));
}

/// Run one custom job's SQL and write the outcome back onto its own row.
fn run_custom_job(
    app: App,
    record_id: String,
    sql: String,
) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async move {
        let (status, message) = match app.db().execute(&sql, &[]).await {
            Ok(n) => ("success", format!("{n} row(s) affected")),
            Err(e) => ("error", e.to_string()),
        };
        let now = DateTime::now().to_pb_string();
        if let Err(e) = app
            .db()
            .execute(
                &format!(
                    r#"UPDATE "{COLLECTION}" SET "lastRunAt" = $1, "lastStatus" = $2, "lastMessage" = $3 WHERE "id" = $4"#
                ),
                &[
                    Sql::from(now),
                    Sql::from(status),
                    Sql::from(message),
                    Sql::from(record_id.clone()),
                ],
            )
            .await
        {
            tracing::warn!(error = %e, record_id = %record_id, "failed to record a custom cron job's run result");
        }
    })
}

/// Bind the reactive and validating hooks. Called once from
/// [`crate::app::App::bootstrap`], all tagged to `_cron_jobs` so none of
/// them ever fire for any other collection's writes.
pub fn bind_hooks(app: &App) {
    // Reject a malformed cron expression before the row is even written,
    // rather than accepting it and silently never scheduling it.
    app.hooks().on_record_create.bind(
        Handler::new(|e: &mut RecordEvent| {
            Box::pin(async move {
                let expr = e
                    .record
                    .get("expression")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                validate_expression(&expr)?;
                e.next().await
            })
        })
        .with_tags([COLLECTION]),
    );
    app.hooks().on_record_update.bind(
        Handler::new(|e: &mut RecordEvent| {
            Box::pin(async move {
                let expr = e
                    .record
                    .get("expression")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                validate_expression(&expr)?;
                e.next().await
            })
        })
        .with_tags([COLLECTION]),
    );

    let a = app.clone();
    app.hooks().on_record_after_create_success.bind(
        Handler::new(move |e: &mut RecordEvent| {
            let a = a.clone();
            let row = record_to_row(e);
            Box::pin(async move {
                sync_one(&a, &row);
                e.next().await
            })
        })
        .with_tags([COLLECTION]),
    );
    let a = app.clone();
    app.hooks().on_record_after_update_success.bind(
        Handler::new(move |e: &mut RecordEvent| {
            let a = a.clone();
            let row = record_to_row(e);
            Box::pin(async move {
                sync_one(&a, &row);
                e.next().await
            })
        })
        .with_tags([COLLECTION]),
    );
    let a = app.clone();
    app.hooks().on_record_after_delete_success.bind(
        Handler::new(move |e: &mut RecordEvent| {
            let a = a.clone();
            let id = e.record.id().to_string();
            Box::pin(async move {
                unsync(&a, &id);
                e.next().await
            })
        })
        .with_tags([COLLECTION]),
    );
}

/// Builds a synthetic [`Row`] from the record a hook just saved, so
/// [`sync_one`] has one code path regardless of whether it is called from
/// [`sync_all`]'s raw `SELECT *` at boot or from a hook holding a
/// [`cratebase_core::Record`].
fn record_to_row(e: &RecordEvent) -> Row {
    let value = e.record.to_json(Default::default());
    let obj = value.as_object().cloned().unwrap_or_default();
    let columns: Vec<String> = obj.keys().cloned().collect();
    let values: Vec<Sql> = columns
        .iter()
        .map(|k| match obj.get(k) {
            Some(serde_json::Value::Bool(b)) => Sql::from(*b),
            Some(serde_json::Value::String(s)) => Sql::from(s.clone()),
            Some(serde_json::Value::Number(n)) => n
                .as_i64()
                .map(Sql::from)
                .or_else(|| n.as_f64().map(Sql::from))
                .unwrap_or(Sql::Null),
            _ => Sql::Null,
        })
        .collect();
    Row {
        columns: columns.into(),
        values,
    }
}

/// A real 5-field cron expression, not just a non-empty string.
fn validate_expression(expression: &str) -> Result<(), AppError> {
    if expression.trim().is_empty() {
        return Err(AppError::validation(
            "An error occurred while validating the submitted data.",
            [(
                "expression".to_string(),
                FieldError::new(REQUIRED, "Cannot be blank."),
            )],
        ));
    }
    expression.parse::<croner::Cron>().map_err(|e| {
        AppError::validation(
            "An error occurred while validating the submitted data.",
            [(
                "expression".to_string(),
                FieldError::new("validation_invalid_format", e.to_string()),
            )],
        )
    })?;
    Ok(())
}
