//! Outgoing webhooks (`_webhooks`): POST a JSON payload to a configured
//! URL whenever a record event fires on a chosen collection — entirely
//! from the dashboard or the generic Records API, no Rust code, no
//! redeploy.
//!
//! # Trust boundary
//!
//! `_webhooks` is superuser-only end to end (every rule stays at
//! `Collection::new`'s default `None` — see
//! `cratebase_core::Collection::default_system_collections`), exactly
//! like `_cron_jobs`: a webhook's `url`/`secret` are operator-configured
//! integration points, not something a non-superuser record should ever
//! reach.
//!
//! # No in-process registry, unlike `_cron_jobs`
//!
//! [`crate::cron_jobs`] keeps an in-process scheduler ([`crate::cron::CronService`])
//! that has to be told about every `_cron_jobs` row up front and kept in
//! sync as rows change. Webhooks have no equivalent standing state: every
//! dispatch already runs a fresh `SELECT` against `_webhooks` for the
//! collection that just changed, so there is nothing to load at boot and
//! nothing to resync on a dashboard edit — the next write to that
//! collection simply sees the new row. [`bind_hooks`] therefore only
//! registers handlers; there is no `sync_all`.
//!
//! # Why the dispatch hook is untagged
//!
//! Every other hook in this codebase that only cares about one system
//! collection is tagged to it (see `_cron_jobs`'s `with_tags([COLLECTION])`).
//! A webhook, by definition, watches *some other* collection — the one
//! named in its own `collectionRef` — so the reactive
//! `on_record_after_{create,update,delete}_success` handlers below are
//! bound with no tags at all, which makes them run for every collection's
//! writes (`Hook::trigger` only tag-filters handlers that opted in). Each
//! invocation looks up matching `_webhooks` rows by the triggering
//! collection's name/id at dispatch time.
//!
//! # Breaking the recursion
//!
//! A write to `_webhooks` itself must never trigger a webhook dispatch
//! for `_webhooks` — the untagged handler above would otherwise fire
//! on every webhook status write-back, forever. [`dispatch_after_success`]
//! simply skips events whose own collection is `_webhooks`; a `_webhooks`
//! row can never be selected as a dispatch target in the first place
//! (`default_system_collections` never gives it PocketBase's "no
//! equivalent" caveat away — see its own comment).
//!
//! # Why the status write-back bypasses the record API
//!
//! [`deliver`] writes `lastTriggeredAt`/`lastStatus`/`lastMessage` back
//! with a raw `UPDATE`, not `records::update` — same reasoning as
//! `_cron_jobs`'s `run_custom_job`: going through the record API would
//! re-fire the very hook that dispatches webhooks (harmless here since
//! `_webhooks` is never a dispatch target, but still pointless), and it
//! would run full field validation for a write that only ever touches
//! three system-owned columns.
//!
//! # Why delivery is fire-and-forget
//!
//! [`dispatch_after_success`] spawns one `tokio::spawn` per triggering
//! record event, and the lookup+delivery work inside it spawns one more
//! `tokio::spawn` per matching webhook row, each POSTing with a 5 second
//! timeout via `reqwest`. A slow or dead target therefore never blocks
//! the record write's HTTP response, and one slow target never blocks
//! another target's delivery either.

use std::time::Duration;

use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;

use cratebase_core::codes::REQUIRED;
use cratebase_core::{AppError, DateTime, FieldError};
use cratebase_db::engine::{Executor, Sql};

use crate::app::App;
use crate::events::RecordEvent;
use crate::hooks::{Event, Handler};

const COLLECTION: &str = "_webhooks";
const EVENT_KINDS: [&str; 3] = ["create", "update", "delete"];

/// Bind the validating and reactive hooks. Called once from
/// [`crate::app::App::bootstrap`].
pub fn bind_hooks(app: &App) {
    // Reject a malformed `events` value before the row is even written,
    // rather than accepting it and silently never dispatching.
    app.hooks().on_record_create.bind(
        Handler::new(|e: &mut RecordEvent| {
            Box::pin(async move {
                let events = e
                    .record
                    .get("events")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                validate_events(&events)?;
                e.next().await
            })
        })
        .with_tags([COLLECTION]),
    );
    app.hooks().on_record_update.bind(
        Handler::new(|e: &mut RecordEvent| {
            Box::pin(async move {
                let events = e
                    .record
                    .get("events")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                validate_events(&events)?;
                e.next().await
            })
        })
        .with_tags([COLLECTION]),
    );

    // Reactive dispatch. Deliberately untagged — see the module doc
    // comment for why this needs to watch every collection, not just
    // `_webhooks`. Each handler still calls `e.next()`: this is a
    // notification, not a gate, and every other handler that might be
    // registered on the same hook (e.g. `_cron_jobs`'/`_teams`' own
    // reactive sync) must still run after it.
    let a = app.clone();
    app.hooks()
        .on_record_after_create_success
        .bind(Handler::new(move |e: &mut RecordEvent| {
            dispatch_after_success(&a, "create", e);
            e.next()
        }));
    let a = app.clone();
    app.hooks()
        .on_record_after_update_success
        .bind(Handler::new(move |e: &mut RecordEvent| {
            dispatch_after_success(&a, "update", e);
            e.next()
        }));
    let a = app.clone();
    app.hooks()
        .on_record_after_delete_success
        .bind(Handler::new(move |e: &mut RecordEvent| {
            dispatch_after_success(&a, "delete", e);
            e.next()
        }));
}

/// Snapshot what the hook needs from `e` and spawn the rest of the work,
/// so a slow `_webhooks` lookup or a slow delivery never delays the
/// record write's response.
fn dispatch_after_success(app: &App, event: &str, e: &RecordEvent) {
    // Never dispatch for writes to `_webhooks` itself — see the module
    // doc comment.
    if e.collection.name == COLLECTION || e.collection.id == COLLECTION {
        return;
    }
    let app = app.clone();
    let event = event.to_string();
    let collection_name = e.collection.name.clone();
    let collection_id = e.collection.id.clone();
    let record = e.record.to_json(Default::default());
    tokio::spawn(async move {
        dispatch(app, event, collection_name, collection_id, record).await;
    });
}

/// Load every enabled `_webhooks` row targeting `collection_name`/
/// `collection_id` and subscribed to `event`, and spawn one delivery per
/// match.
async fn dispatch(
    app: App,
    event: String,
    collection_name: String,
    collection_id: String,
    record: Value,
) {
    let rows = match app
        .db()
        .query(
            &format!(
                r#"SELECT * FROM "{COLLECTION}" WHERE "enabled" = 1 AND ("collectionRef" = $1 OR "collectionRef" = $2)"#
            ),
            &[Sql::from(collection_name.clone()), Sql::from(collection_id)],
        )
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "failed to load webhooks for dispatch");
            return;
        }
    };
    let payload = json!({
        "event": event,
        "collection": collection_name,
        "record": record,
    });
    for row in &rows {
        let Some(id) = row.get_str("id") else {
            continue;
        };
        let events = row.get_str("events").unwrap_or_default();
        if !events.split(',').map(str::trim).any(|k| k == event) {
            continue;
        }
        let url = row.get_str("url").unwrap_or_default().to_string();
        if url.is_empty() {
            continue;
        }
        let secret = row
            .get_str("secret")
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let record_id = id.to_string();
        let app = app.clone();
        let payload = payload.clone();
        tokio::spawn(deliver(app, record_id, url, secret, payload));
    }
}

/// POST `payload` to `url`, optionally signing the raw body with
/// HMAC-SHA256 in `X-Cratebase-Signature`, and write the outcome back
/// onto the triggering `_webhooks` row.
async fn deliver(app: App, record_id: String, url: String, secret: Option<String>, payload: Value) {
    static CLIENT: std::sync::LazyLock<reqwest::Client> =
        std::sync::LazyLock::new(reqwest::Client::new);
    let client = &*CLIENT;

    let body = serde_json::to_vec(&payload).unwrap_or_default();
    let mut builder = client
        .post(&url)
        .header("Content-Type", "application/json")
        .timeout(Duration::from_secs(5));
    if let Some(secret) = &secret {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
            .expect("HMAC accepts a key of any length");
        mac.update(&body);
        let signature = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        builder = builder.header("X-Cratebase-Signature", signature);
    }
    let (status, message) = match builder.body(body).send().await {
        Ok(resp) => {
            let status = resp.status();
            (status.as_u16().to_string(), format!("delivered to {url}"))
        }
        Err(e) => ("error".to_string(), e.to_string()),
    };

    let now = DateTime::now().to_pb_string();
    if let Err(e) = app
        .db()
        .execute(
            &format!(
                r#"UPDATE "{COLLECTION}" SET "lastTriggeredAt" = $1, "lastStatus" = $2, "lastMessage" = $3 WHERE "id" = $4"#
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
        tracing::warn!(error = %e, record_id = %record_id, "failed to record a webhook's delivery result");
    }
}

/// A non-empty, comma-separated subset of `create`, `update`, `delete`.
fn validate_events(events: &str) -> Result<(), AppError> {
    let trimmed = events.trim();
    if trimmed.is_empty() {
        return Err(AppError::validation(
            "An error occurred while validating the submitted data.",
            [(
                "events".to_string(),
                FieldError::new(REQUIRED, "Cannot be blank."),
            )],
        ));
    }
    for part in trimmed.split(',') {
        let part = part.trim();
        if !EVENT_KINDS.contains(&part) {
            return Err(AppError::validation(
                "An error occurred while validating the submitted data.",
                [(
                    "events".to_string(),
                    FieldError::new(
                        "validation_invalid_format",
                        format!(
                            "\"{part}\" is not a valid event; must be a comma-separated subset of create, update, delete."
                        ),
                    ),
                )],
            ));
        }
    }
    Ok(())
}
