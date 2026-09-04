//! `POST /api/push/send` — app-triggered push notification delivery
//! against `_push_subscriptions` rows, through `crate::push::PushService`.
//!
//! # Auth: superuser-only, not "any authenticated record"
//!
//! This is deliberately a stricter gate than `POST /api/llm/chat`'s "any
//! authenticated record" (`crate::routes::llm::chat`'s own doc comment).
//! `/api/llm/chat` spends the operator's quota on *the caller's own*
//! conversation — an ordinary end-user action, gated the same tier as an
//! ordinary record write. This endpoint's `recordId`/`subscriptionIds`
//! target is caller-chosen and can name *any* record's subscriptions, not
//! just the caller's own: an unauthenticated (or merely authenticated,
//! non-superuser) caller could otherwise spam arbitrary end users with
//! notifications, or drain the operator's FCM/APNs/VAPID sending quota,
//! with no rule of the caller's own account to stand in the way (unlike
//! the record API, where `createRule`/`updateRule` decide access to a
//! specific row). That makes it an operator/ops action — the same trust
//! tier as `_cron_jobs`/`_webhooks`, both superuser-only end to end — not
//! an end-user one, so it is gated with [`RequireSuperuser`] rather than
//! [`crate::extract::Auth`].
//!
//! A collection's own record-driven push (a comment landing on a post
//! notifying its author, say) does not go through this endpoint at all;
//! that is what `settings.push.triggers` + `crate::push::bind_hooks`
//! cover, with no per-request authorization question to answer because
//! nothing external ever names a target — see `crate::push`'s module
//! doc comment.

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use cratebase_db::engine::{Executor, Sql};

use crate::app::App;
use crate::extract::RequireSuperuser;
use crate::http_error::{ApiError, ApiResult};
use crate::push::{PushError, PushPayload, PushService};

pub fn router() -> Router<App> {
    Router::new().route("/push/send", post(send))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendBody {
    /// Target a single record's subscriptions: both `collection` and
    /// `recordId` must be set together.
    #[serde(default)]
    collection: Option<String>,
    #[serde(default)]
    record_id: Option<String>,
    /// Or target specific `_push_subscriptions` rows directly. Ignored
    /// when `collection`/`recordId` are set.
    #[serde(default)]
    subscription_ids: Vec<String>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    data: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SendResult {
    id: String,
    platform: String,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SendResponse {
    sent: usize,
    failed: usize,
    results: Vec<SendResult>,
}

async fn send(
    State(app): State<App>,
    RequireSuperuser(_): RequireSuperuser,
    Json(body): Json<SendBody>,
) -> ApiResult<Json<SendResponse>> {
    if body.title.is_empty() && body.body.is_empty() {
        return Err(ApiError::bad_request("title or body is required."));
    }

    let targets = load_targets(&app, &body).await?;
    if targets.is_empty() {
        return Ok(Json(SendResponse {
            sent: 0,
            failed: 0,
            results: Vec::new(),
        }));
    }

    let service = PushService::from_settings(&app.settings().push);
    let payload = PushPayload {
        title: body.title.clone(),
        body: body.body.clone(),
        data: body.data.clone(),
    };

    let mut results = Vec::with_capacity(targets.len());
    for (id, platform, token) in targets {
        match service.send(&platform, &token, &payload).await {
            Ok(()) => results.push(SendResult {
                id,
                platform,
                ok: true,
                error: None,
            }),
            Err(PushError::InvalidToken) => {
                crate::push::disable_subscription(&app, &id).await;
                results.push(SendResult {
                    id,
                    platform,
                    ok: false,
                    error: Some("dead token; subscription disabled".into()),
                });
            }
            Err(e) => results.push(SendResult {
                id,
                platform,
                ok: false,
                error: Some(e.to_string()),
            }),
        }
    }

    let sent = results.iter().filter(|r| r.ok).count();
    let failed = results.len() - sent;
    Ok(Json(SendResponse {
        sent,
        failed,
        results,
    }))
}

/// Resolve `body`'s target into `(id, platform, token)` triples from
/// `_push_subscriptions`, honouring `enabled = 0` rows by excluding them —
/// the same "disabled means don't send" contract
/// `crate::push::dispatch_trigger` uses for its own broadcast/targeted
/// lookups.
async fn load_targets(app: &App, body: &SendBody) -> ApiResult<Vec<(String, String, String)>> {
    let rows = if let (Some(collection), Some(record_id)) = (&body.collection, &body.record_id) {
        app.db()
            .query(
                r#"SELECT "id", "platform", "token" FROM "_push_subscriptions" WHERE "enabled" = 1 AND "collectionRef" = $1 AND "recordRef" = $2"#,
                &[Sql::from(collection.clone()), Sql::from(record_id.clone())],
            )
            .await
    } else if !body.subscription_ids.is_empty() {
        let placeholders: Vec<String> = (1..=body.subscription_ids.len())
            .map(|i| format!("${i}"))
            .collect();
        let sql = format!(
            r#"SELECT "id", "platform", "token" FROM "_push_subscriptions" WHERE "enabled" = 1 AND "id" IN ({})"#,
            placeholders.join(", ")
        );
        let params: Vec<Sql> = body
            .subscription_ids
            .iter()
            .cloned()
            .map(Sql::from)
            .collect();
        app.db().query(&sql, &params).await
    } else {
        return Err(ApiError::bad_request(
            "either collection+recordId or subscriptionIds is required.",
        ));
    };

    let rows = rows.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            let id = r.get_str("id")?.to_string();
            let platform = r.get_str("platform")?.to_string();
            let token = r.get_str("token")?.to_string();
            Some((id, platform, token))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::app::App;
    use crate::config::Config;

    async fn test_app() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::new(Config::memory(dir.path()));
        app.bootstrap().await.expect("bootstrap");
        (app, dir)
    }

    /// The auth gate this module's doc comment documents: an
    /// unauthenticated `POST /api/push/send` must be rejected before ever
    /// reaching `_push_subscriptions`, not merely return an empty result.
    #[tokio::test]
    async fn send_requires_a_superuser_token() {
        let (app, _dir) = test_app().await;
        let router = crate::routes::api_router(&app).with_state(app);

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/push/send")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "subscriptionIds": ["anything"],
                            "title": "hi",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
