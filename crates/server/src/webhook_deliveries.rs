//! Durable, retrying delivery for `_webhooks` (`_webhookDeliveries`): a
//! dedicated, always-on worker — independent of `settings.queue.enabled`
//! — that claims `pending` delivery rows and actually POSTs them, with
//! exponential backoff, timeouts, response-excerpt capture, and
//! automatic disable of a chronically-failing webhook.
//!
//! # Why a dedicated worker, not the general job queue
//!
//! The general queue (`crate::queue`) is toggle-gated: `settings.queue.enabled`
//! defaults `false`, and an idle install never provisions `_queue_jobs`
//! or spawns its worker. Outgoing webhook delivery has no equivalent
//! opt-in — a `_webhooks` row is either enabled or it isn't, with no
//! separate "and also turn on the thing that actually sends it" switch —
//! so tying delivery to the queue toggle would silently break webhooks
//! for anyone who hasn't also enabled the unrelated job-queue feature.
//! [`start`] is called unconditionally from `App::bootstrap`, the same
//! "always on" tier `crate::cron::CronService` and `crate::realtime`'s
//! cross-node listener already are. This is the simplest robust design
//! available: no new toggle to document, no interaction with the queue's
//! own toggle to reason about, and the claim/backoff/reclaim machinery
//! below is a direct, much smaller copy of `crate::queue`'s own (see that
//! module's doc for why the claim is race-free on both engines — the
//! same `FOR UPDATE SKIP LOCKED` / re-checked `WHERE status = 'pending'`
//! update applies here verbatim).
//!
//! # Lifecycle
//!
//! `pending` → `in_progress` → `success`, or back to `pending` with a
//! bumped `attempts` and a pushed-out `nextAttemptAt` on failure, until
//! `attempts` reaches the delivery's own `maxAttempts` (snapshotted from
//! the webhook's `maxAttempts` override, or [`DEFAULT_MAX_ATTEMPTS`], at
//! enqueue time — a later edit to the webhook's own `maxAttempts` never
//! changes an in-flight delivery's budget), at which point it becomes
//! `failed` and is never retried again automatically (an operator can
//! still hit it with [`replay`]). The backoff schedule
//! ([`BACKOFF_SECONDS`]) is the spec's own: 1m, 5m, 30m, 2h, 6h.
//!
//! # Auto-disable
//!
//! Every attempt — success or failure — updates the owning `_webhooks`
//! row's `consecutiveFailures` (reset to `0` on success, incremented on
//! failure). Once it reaches `settings.webhooks.disableAfterFailures`
//! (default 50), [`attempt`] flips that webhook's `enabled` to `false`
//! and writes an `_audit_log` row recording why — a permanently-broken
//! endpoint stops burning delivery attempts (and retry backoff slots)
//! forever, and a new delivery is never queued for it again until a
//! superuser re-enables it.
//!
//! # Signature, delivery id and timestamp
//!
//! Every attempt sends `X-Cratebase-Delivery` (this `_webhookDeliveries`
//! row's id — stable across every retry of the same delivery, so a
//! receiver can deduplicate by it), `X-Cratebase-Timestamp` (unix
//! seconds at send time), and, when the webhook has a `secret`,
//! `X-Cratebase-Signature: sha256=<hex>` — HMAC-SHA256 over
//! `"{timestamp}.{raw body}"` keyed by that secret. A receiver verifies
//! by recomputing the same HMAC over the timestamp and raw body it
//! received and comparing in constant time; the site docs' webhooks page
//! has a full Node recipe. Signing the timestamp together with the body
//! (rather than the body alone) means a captured, replayed request is
//! only ever valid for the timestamp it was actually sent with — a
//! receiver that also rejects a timestamp too far from "now" gets replay
//! protection for free.

use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::post;
use axum::Router;
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;

use cratebase_core::{AppError, DateTime, Record};
use cratebase_db::engine::{Executor, Sql};
use cratebase_filter::Dialect;

use crate::app::App;
use crate::extract::RequireSuperuser;
use crate::http_error::{ApiError, ApiResult};

pub const COLLECTION: &str = "_webhookDeliveries";

pub const STATUS_PENDING: &str = "pending";
const STATUS_IN_PROGRESS: &str = "in_progress";
pub const STATUS_SUCCESS: &str = "success";
pub const STATUS_FAILED: &str = "failed";

/// Used when a webhook's own `maxAttempts` override is absent/`0`.
pub const DEFAULT_MAX_ATTEMPTS: i64 = 6;

/// Retry delays in seconds after attempts 1 through 5 fail — the spec's
/// own schedule (1m, 5m, 30m, 2h, 6h). An attempt beyond the table's
/// length (only reachable with a webhook-level `maxAttempts` above 6)
/// reuses the last entry rather than growing further.
const BACKOFF_SECONDS: [i64; 5] = [60, 300, 1800, 7200, 21600];

/// A delivery whose `status` has been `in_progress` longer than this is
/// assumed to belong to a crashed worker and is put back to `pending` —
/// same reasoning, and the same default, as `crate::queue`'s own
/// `stale_timeout`.
const STALE_TIMEOUT: Duration = Duration::from_secs(300);

/// A response body excerpt is capped at this many bytes before storage —
/// the spec's own "≤2KB".
const RESPONSE_BODY_EXCERPT_MAX: usize = 2048;

const DELIVERY_TIMEOUT: Duration = Duration::from_secs(10);

/// Cron id for the retention cleanup (`settings.webhooks.deliveryRetentionDays`).
pub const CLEANUP_JOB_ID: &str = "__cbWebhookDeliveriesCleanup__";

/// Start the always-on ticker and register the retention-cleanup cron.
/// Called once, unconditionally, from `App::bootstrap` — see the module
/// doc for why this has no toggle.
pub fn start(app: &App) {
    let tick_app = app.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            reclaim_stale(&tick_app).await;
            match claim_next(&tick_app).await {
                Ok(Some(claimed)) => attempt(&tick_app, claimed).await,
                Ok(None) => {}
                Err(e) => tracing::warn!(error = %e, "webhook delivery: failed to claim next row"),
            }
        }
    });

    let cron_app = app.clone();
    let _ = app.cron().add(CLEANUP_JOB_ID, "0 */6 * * *", move || {
        let app = cron_app.clone();
        async move { cleanup(&app).await }
    });
}

/// Insert one `pending` `_webhookDeliveries` row. `record` is the
/// generic `{event, collection, record}`-shaped snapshot; the
/// chat-app-specific reshaping (`crate::webhooks::format_payload`) is
/// applied fresh on every attempt from the *current* webhook `url`,
/// rather than baked in here, so an operator can point an existing
/// webhook at Slack after the fact and have even its already-queued
/// deliveries pick up the new shape.
pub async fn enqueue_delivery(
    app: &App,
    webhook_id: &str,
    event: &str,
    collection: &str,
    record_id: &str,
    record: Value,
    max_attempts: i64,
) -> Result<String, AppError> {
    let Some(collection_def) = app.db().collections.get_by_name(COLLECTION) else {
        return Err(AppError::internal(
            "the _webhookDeliveries collection is missing",
        ));
    };
    let mut row = Record::new(collection_def);
    row.set("webhookRef", Value::String(webhook_id.to_string()));
    row.set("event", Value::String(event.to_string()));
    row.set("collectionRef", Value::String(collection.to_string()));
    row.set("recordId", Value::String(record_id.to_string()));
    row.set(
        "payload",
        json!({ "event": event, "collection": collection, "record": record }),
    );
    row.set("attempts", json!(0));
    row.set("maxAttempts", json!(max_attempts.max(1)));
    row.set("status", Value::String(STATUS_PENDING.to_string()));
    row.set(
        "nextAttemptAt",
        Value::String(DateTime::now().to_pb_string()),
    );

    cratebase_db::records::create(app.db(), &app.db().collections, &mut row)
        .await
        .map_err(AppError::from)?;
    Ok(row.id().to_string())
}

async fn reclaim_stale(app: &App) {
    let cutoff = DateTime::from_utc(
        DateTime::now().inner() - chrono::Duration::from_std(STALE_TIMEOUT).unwrap_or_default(),
    );
    let reclaimed = app
        .db()
        .execute(
            &format!(
                r#"UPDATE "{COLLECTION}" SET "status" = $1 WHERE "status" = $2 AND "startedAt" <= $3"#
            ),
            &[
                Sql::from(STATUS_PENDING),
                Sql::from(STATUS_IN_PROGRESS),
                Sql::from(cutoff.to_pb_string()),
            ],
        )
        .await;
    match reclaimed {
        Ok(0) => {}
        Ok(n) => tracing::info!(
            count = n,
            "webhook delivery: reclaimed stale in-progress row(s)"
        ),
        Err(e) => tracing::warn!(error = %e, "webhook delivery: failed to reclaim stale rows"),
    }
}

/// One claimed delivery, enough to attempt it and write the outcome back.
struct Claimed {
    id: String,
    webhook_id: String,
    event: String,
    collection: String,
    record: Value,
    attempts: i64,
    max_attempts: i64,
}

/// Atomically claim the earliest due `pending` row, if any — identical
/// claim strategy to `crate::queue::claim_next` (see that function's doc
/// for why it's race-free on both engines).
async fn claim_next(app: &App) -> Result<Option<Claimed>, AppError> {
    app.run_in_transaction(|tx| async move {
        let now = DateTime::now().to_pb_string();
        let select_sql = match tx.dialect() {
            Dialect::Postgres => format!(
                r#"SELECT "id", "webhookRef", "event", "collectionRef", "recordId", "payload",
                          "attempts", "maxAttempts" FROM "{COLLECTION}"
                   WHERE "status" = $1 AND "nextAttemptAt" <= $2
                   ORDER BY "nextAttemptAt" ASC LIMIT 1 FOR UPDATE SKIP LOCKED"#
            ),
            Dialect::Sqlite => format!(
                r#"SELECT "id", "webhookRef", "event", "collectionRef", "recordId", "payload",
                          "attempts", "maxAttempts" FROM "{COLLECTION}"
                   WHERE "status" = $1 AND "nextAttemptAt" <= $2
                   ORDER BY "nextAttemptAt" ASC LIMIT 1"#
            ),
        };
        let Some(row) = tx
            .query_one(
                &select_sql,
                &[Sql::from(STATUS_PENDING), Sql::from(now.clone())],
            )
            .await
            .map_err(AppError::from)?
        else {
            return Ok(None);
        };
        let id = row.get_str("id").unwrap_or_default().to_string();
        let payload: Value = row
            .get_str("payload")
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(Value::Null);
        let record = payload.get("record").cloned().unwrap_or(Value::Null);
        let event = payload
            .get("event")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let collection = payload
            .get("collection")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        let updated = tx
            .execute(
                &format!(
                    r#"UPDATE "{COLLECTION}" SET "status" = $1, "startedAt" = $2
                       WHERE "id" = $3 AND "status" = $4"#
                ),
                &[
                    Sql::from(STATUS_IN_PROGRESS),
                    Sql::from(now),
                    Sql::from(id.clone()),
                    Sql::from(STATUS_PENDING),
                ],
            )
            .await
            .map_err(AppError::from)?;
        if updated == 0 {
            return Ok(None);
        }
        Ok(Some(Claimed {
            id,
            webhook_id: row.get_str("webhookRef").unwrap_or_default().to_string(),
            event,
            collection,
            record,
            attempts: row.get_i64("attempts").unwrap_or(0),
            max_attempts: row.get_i64("maxAttempts").unwrap_or(DEFAULT_MAX_ATTEMPTS),
        }))
    })
    .await
}

/// `min(2^(attempts-1))`-free: a fixed lookup table, not exponential —
/// see [`BACKOFF_SECONDS`].
fn backoff_delay(attempts: i64) -> Duration {
    let idx = (attempts.max(1) as usize - 1).min(BACKOFF_SECONDS.len() - 1);
    Duration::from_secs(BACKOFF_SECONDS[idx] as u64)
}

static CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
    reqwest::Client::builder()
        // See the module doc's SSRF section: without this, a
        // `validate_webhook_url`-approved public URL could 302 to a
        // blocked address and the delivery would follow it there anyway.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("static client config is valid")
});

/// Look up the owning `_webhooks` row fresh (not snapshotted at enqueue
/// time — see the module doc), attempt the HTTP delivery, and write the
/// outcome back onto both the `_webhookDeliveries` row and the
/// `_webhooks` row's `lastTriggeredAt`/`lastStatus`/`lastMessage`/
/// `consecutiveFailures` (checking the auto-disable threshold).
async fn attempt(app: &App, claimed: Claimed) {
    let started = std::time::Instant::now();
    let webhook = app
        .db()
        .query_one(
            &format!(
                r#"SELECT * FROM "{}" WHERE "id" = $1"#,
                crate::webhooks::COLLECTION
            ),
            &[Sql::from(claimed.webhook_id.clone())],
        )
        .await
        .ok()
        .flatten();

    let Some(webhook) = webhook else {
        finish(
            app,
            &claimed,
            Outcome {
                success: false,
                response_code: None,
                response_body: String::new(),
                error: "the owning _webhooks row no longer exists".to_string(),
                duration_ms: started.elapsed().as_millis() as i64,
            },
        )
        .await;
        return;
    };
    let url = webhook.get_str("url").unwrap_or_default().to_string();
    let secret = webhook
        .get_str("secret")
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let outcome = match crate::webhooks::validate_webhook_url(&url).await {
        Err(reason) => Outcome {
            success: false,
            response_code: None,
            response_body: String::new(),
            error: reason,
            duration_ms: started.elapsed().as_millis() as i64,
        },
        Ok(()) => {
            let payload = crate::webhooks::format_payload(
                &url,
                &claimed.event,
                &claimed.collection,
                &claimed.record,
            );
            let body = serde_json::to_vec(&payload).unwrap_or_default();
            let timestamp = chrono::Utc::now().timestamp();
            let mut builder = CLIENT
                .post(&url)
                .header("Content-Type", "application/json")
                .header("X-Cratebase-Delivery", claimed.id.as_str())
                .header("X-Cratebase-Timestamp", timestamp.to_string())
                .timeout(DELIVERY_TIMEOUT);
            if let Some(secret) = &secret {
                let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
                    .expect("HMAC accepts a key of any length");
                mac.update(format!("{timestamp}.").as_bytes());
                mac.update(&body);
                let signature = mac
                    .finalize()
                    .into_bytes()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>();
                builder = builder.header("X-Cratebase-Signature", format!("sha256={signature}"));
            }
            match builder.body(body).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    let excerpt: String = text.chars().take(RESPONSE_BODY_EXCERPT_MAX).collect();
                    Outcome {
                        success: status.is_success(),
                        response_code: Some(status.as_u16()),
                        response_body: excerpt,
                        error: if status.is_success() {
                            String::new()
                        } else {
                            format!("received HTTP {status}")
                        },
                        duration_ms: started.elapsed().as_millis() as i64,
                    }
                }
                Err(e) => Outcome {
                    success: false,
                    response_code: None,
                    response_body: String::new(),
                    error: e.to_string(),
                    duration_ms: started.elapsed().as_millis() as i64,
                },
            }
        }
    };

    finish(app, &claimed, outcome).await;
}

struct Outcome {
    success: bool,
    response_code: Option<u16>,
    response_body: String,
    error: String,
    duration_ms: i64,
}

/// Write `outcome` onto the `_webhookDeliveries` row (retry/give-up same
/// as `crate::queue::mark_failed_or_retry`) and roll it up onto the
/// owning `_webhooks` row, including the auto-disable check.
async fn finish(app: &App, claimed: &Claimed, outcome: Outcome) {
    let response_code = outcome
        .response_code
        .map(|c| c.to_string())
        .unwrap_or_default();

    if outcome.success {
        if let Err(e) = app
            .db()
            .execute(
                &format!(
                    r#"UPDATE "{COLLECTION}" SET "status" = $1, "responseCode" = $2, "responseBody" = $3,
                       "durationMs" = $4, "deliveredAt" = $5, "error" = '' WHERE "id" = $6"#
                ),
                &[
                    Sql::from(STATUS_SUCCESS),
                    Sql::from(response_code.clone()),
                    Sql::from(outcome.response_body.clone()),
                    Sql::from(outcome.duration_ms),
                    Sql::from(DateTime::now().to_pb_string()),
                    Sql::from(claimed.id.clone()),
                ],
            )
            .await
        {
            tracing::warn!(error = %e, delivery = %claimed.id, "failed to record a successful webhook delivery");
        }
    } else {
        let attempts = claimed.attempts + 1;
        let status = if attempts >= claimed.max_attempts {
            STATUS_FAILED
        } else {
            STATUS_PENDING
        };
        let next_attempt_at = DateTime::from_utc(
            DateTime::now().inner()
                + chrono::Duration::from_std(backoff_delay(attempts)).unwrap_or_default(),
        );
        if let Err(e) = app
            .db()
            .execute(
                &format!(
                    r#"UPDATE "{COLLECTION}" SET "status" = $1, "attempts" = $2, "responseCode" = $3,
                       "responseBody" = $4, "durationMs" = $5, "nextAttemptAt" = $6, "error" = $7
                       WHERE "id" = $8"#
                ),
                &[
                    Sql::from(status),
                    Sql::from(attempts),
                    Sql::from(response_code.clone()),
                    Sql::from(outcome.response_body.clone()),
                    Sql::from(outcome.duration_ms),
                    Sql::from(next_attempt_at.to_pb_string()),
                    Sql::from(outcome.error.clone()),
                    Sql::from(claimed.id.clone()),
                ],
            )
            .await
        {
            tracing::warn!(error = %e, delivery = %claimed.id, "failed to record a failed webhook delivery attempt");
        }
    }

    roll_up_onto_webhook(app, &claimed.webhook_id, &outcome).await;
}

/// Update the owning `_webhooks` row's `lastTriggeredAt`/`lastStatus`/
/// `lastMessage`/`consecutiveFailures`, auto-disabling it (with an
/// `_audit_log` entry) once `consecutiveFailures` reaches
/// `settings.webhooks.disableAfterFailures`.
async fn roll_up_onto_webhook(app: &App, webhook_id: &str, outcome: &Outcome) {
    let status = if outcome.success {
        outcome
            .response_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| STATUS_SUCCESS.to_string())
    } else {
        outcome
            .response_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "error".to_string())
    };
    let message = if outcome.success {
        "delivered".to_string()
    } else {
        outcome.error.clone()
    };
    crate::webhooks::record_attempt_outcome(app, webhook_id, &status, &message).await;

    if outcome.success {
        if let Err(e) = app
            .db()
            .execute(
                &format!(
                    r#"UPDATE "{}" SET "consecutiveFailures" = 0 WHERE "id" = $1"#,
                    crate::webhooks::COLLECTION
                ),
                &[Sql::from(webhook_id)],
            )
            .await
        {
            tracing::warn!(error = %e, webhook_id = %webhook_id, "failed to reset consecutiveFailures");
        }
        return;
    }

    // Two round trips rather than `UPDATE ... RETURNING`: SQLite's read
    // connections (used by `query`/`query_one` when readers exist — a
    // `:memory:` test database has none, but a real on-disk one does)
    // can't run a write statement at all, so `RETURNING` would only ever
    // work by accident of which connection happened to serve it. A rare,
    // non-hot-path counter bump does not need to be atomic with its own
    // read-back.
    if let Err(e) = app
        .db()
        .execute(
            &format!(
                r#"UPDATE "{}" SET "consecutiveFailures" = "consecutiveFailures" + 1 WHERE "id" = $1"#,
                crate::webhooks::COLLECTION
            ),
            &[Sql::from(webhook_id)],
        )
        .await
    {
        tracing::warn!(error = %e, webhook_id = %webhook_id, "failed to bump consecutiveFailures");
        return;
    }
    let row = match app
        .db()
        .query_one(
            &format!(
                r#"SELECT "consecutiveFailures", "name" FROM "{}" WHERE "id" = $1"#,
                crate::webhooks::COLLECTION
            ),
            &[Sql::from(webhook_id)],
        )
        .await
    {
        Ok(row) => row,
        Err(e) => {
            tracing::warn!(error = %e, webhook_id = %webhook_id, "failed to read back consecutiveFailures");
            return;
        }
    };
    let Some(row) = row else { return };
    let failures = row.get_i64("consecutiveFailures").unwrap_or(0);
    let threshold = app.settings().webhooks.disable_after_failures.max(1);
    if failures < threshold {
        return;
    }
    if let Err(e) = app
        .db()
        .execute(
            &format!(
                r#"UPDATE "{}" SET "enabled" = 0 WHERE "id" = $1"#,
                crate::webhooks::COLLECTION
            ),
            &[Sql::from(webhook_id)],
        )
        .await
    {
        tracing::warn!(error = %e, webhook_id = %webhook_id, "failed to auto-disable a webhook");
        return;
    }
    let name = row.get_str("name").unwrap_or_default().to_string();
    crate::audit::write(
        app,
        None,
        "webhook.autoDisabled",
        webhook_id,
        json!({
            "name": name,
            "consecutiveFailures": failures,
            "threshold": threshold,
        }),
    )
    .await;
    tracing::warn!(
        webhook_id = %webhook_id,
        name = %name,
        failures,
        "webhook automatically disabled after too many consecutive failures"
    );
}

/// `POST /api/webhooks/deliveries/{id}/replay` — superuser: reset a
/// delivery back to `pending` with a fresh `attempts` budget, `error`
/// cleared, `nextAttemptAt` now. Works on a `failed` delivery (the usual
/// case) or even a `success`/`pending` one — replaying an already-successful
/// delivery is a legitimate "resend this to the receiver again" action,
/// not just a dead-letter recovery tool.
pub async fn replay(app: &App, id: &str) -> Result<(), AppError> {
    let updated = app
        .db()
        .execute(
            &format!(
                r#"UPDATE "{COLLECTION}" SET "status" = $1, "attempts" = 0, "nextAttemptAt" = $2,
                   "error" = '' WHERE "id" = $3"#
            ),
            &[
                Sql::from(STATUS_PENDING),
                Sql::from(DateTime::now().to_pb_string()),
                Sql::from(id),
            ],
        )
        .await
        .map_err(AppError::from)?;
    if updated == 0 {
        return Err(AppError::not_found("Missing or invalid webhook delivery."));
    }
    Ok(())
}

/// `settings.webhooks.deliveryRetentionDays` (default 14): delete every
/// `_webhookDeliveries` row older than that, regardless of status. `<= 0`
/// disables the cleanup — same convention as `Logs::max_days`.
async fn cleanup(app: &App) {
    let days = app.settings().webhooks.delivery_retention_days;
    if days <= 0 {
        return;
    }
    let cutoff = DateTime::from_utc(chrono::Utc::now() - chrono::Duration::days(days));
    match app
        .db()
        .execute(
            &format!(r#"DELETE FROM "{COLLECTION}" WHERE "created" < $1"#),
            &[Sql::from(cutoff.to_pb_string())],
        )
        .await
    {
        Ok(n) if n > 0 => tracing::info!(removed = n, "pruned old _webhookDeliveries rows"),
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "_webhookDeliveries cleanup failed"),
    }
}

pub fn router() -> Router<App> {
    Router::new().route("/webhooks/deliveries/{id}/replay", post(replay_route))
}

async fn replay_route(
    State(app): State<App>,
    _su: RequireSuperuser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    replay(&app, &id).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_follows_the_fixed_schedule_and_caps_at_the_last_entry() {
        assert_eq!(backoff_delay(1), Duration::from_secs(60));
        assert_eq!(backoff_delay(2), Duration::from_secs(300));
        assert_eq!(backoff_delay(3), Duration::from_secs(1800));
        assert_eq!(backoff_delay(4), Duration::from_secs(7200));
        assert_eq!(backoff_delay(5), Duration::from_secs(21600));
        assert_eq!(backoff_delay(6), Duration::from_secs(21600));
        assert_eq!(backoff_delay(100), Duration::from_secs(21600));
    }
}
