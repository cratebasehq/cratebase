//! `POST /api/notifications/send`, `POST /api/notifications/read-all` and
//! `GET /api/notifications/unread-count`.
//!
//! Everything else a client needs (list/view one, mark one read, delete)
//! is the ordinary generic records API against `_notifications` —
//! `owner_rule` already scopes it to "my own", and realtime subscriptions
//! on the collection just work (see `crate::notify`'s module doc). These
//! three exist because they don't fit that shape: `send` is a superuser/
//! API-key action that fans out across channels rather than writing one
//! row, and `read-all`/`unread-count` are bulk operations the per-row
//! rule API has no way to express (no filter-rule can express "every row
//! matching this rule", and a client-side "list everything unread, then
//! update each" would defeat the whole point of an index-backed count).

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use cratebase_core::AppError;
use cratebase_db::engine::{Executor, Sql};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::App;
use crate::extract::{Auth, RequireSuperuser};
use crate::http_error::{ApiError, ApiResult};
use crate::notify::{self, NotifySendInput, NOTIFICATIONS_COLLECTION};

pub fn router() -> Router<App> {
    Router::new()
        .route("/notifications/send", post(send))
        .route("/notifications/read-all", post(read_all))
        .route("/notifications/unread-count", get(unread_count))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendBody {
    to: ToField,
    #[serde(default)]
    collection: Option<String>,
    #[serde(rename = "type")]
    kind: String,
    title: String,
    body: String,
    #[serde(default)]
    data: Value,
    #[serde(default)]
    link: Option<String>,
    #[serde(default)]
    channels: Option<Vec<String>>,
}

/// `to` accepts a single id or an array — same ergonomic convention
/// `crate::mails::parse_recipients` gives `to`/`cc`/`bcc`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ToField {
    One(String),
    Many(Vec<String>),
}

impl From<ToField> for Vec<String> {
    fn from(value: ToField) -> Self {
        match value {
            ToField::One(id) => vec![id],
            ToField::Many(ids) => ids,
        }
    }
}

/// Superuser/API-key only — an operator/integration action that can
/// target *any* record's notifications, same trust tier as
/// `crate::routes::push::send`'s own doc comment explains for the
/// equivalent push endpoint.
async fn send(
    State(app): State<App>,
    _su: RequireSuperuser,
    Json(body): Json<SendBody>,
) -> ApiResult<Json<notify::SendOutcome>> {
    let input = NotifySendInput {
        to: body.to.into(),
        collection: body.collection,
        kind: body.kind,
        title: body.title,
        body: body.body,
        data: body.data,
        link: body.link,
        channels: body.channels,
    };
    let outcome = notify::send(&app, input).await.map_err(ApiError)?;
    Ok(Json(outcome))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadAllResponse {
    updated: u64,
}

/// Marks every one of the caller's own unread notifications read in one
/// statement, then best-effort announces each affected row over realtime
/// so an open dashboard/app updates live — same shape `crate::mails`
/// gives a caller ("the action was accepted"), not a per-row report.
async fn read_all(State(app): State<App>, auth: Auth) -> ApiResult<Json<ReadAllResponse>> {
    let Some(collection) = app.db().collections.get_by_name(NOTIFICATIONS_COLLECTION) else {
        return Err(ApiError(AppError::internal(
            "_notifications collection is missing.",
        )));
    };
    let ids: Vec<String> = app
        .db()
        .query(
            r#"SELECT "id" FROM "_notifications"
               WHERE "collectionRef" = $1 AND "recordRef" = $2 AND ("readAt" = '' OR "readAt" IS NULL)"#,
            &[
                Sql::from(auth.collection_id.clone()),
                Sql::from(auth.id.clone()),
            ],
        )
        .await
        .map_err(|e| ApiError(e.into()))?
        .iter()
        .filter_map(|row| row.get_str("id").map(str::to_string))
        .collect();

    if ids.is_empty() {
        return Ok(Json(ReadAllResponse { updated: 0 }));
    }

    let now = cratebase_core::DateTime::now().to_pb_string();
    let updated = app
        .db()
        .execute(
            r#"UPDATE "_notifications" SET "readAt" = $1, "updated" = $1
               WHERE "collectionRef" = $2 AND "recordRef" = $3 AND ("readAt" = '' OR "readAt" IS NULL)"#,
            &[
                Sql::from(now),
                Sql::from(auth.collection_id.clone()),
                Sql::from(auth.id.clone()),
            ],
        )
        .await
        .map_err(|e| ApiError(e.into()))?;

    // Only bother re-fetching/publishing when someone could actually be
    // listening — a bulk mark-read with no open realtime subscription is
    // the common case and shouldn't cost a query per row.
    if app.realtime().client_count() > 0 {
        let app = app.clone();
        let collection = collection.clone();
        tokio::spawn(async move {
            for id in ids {
                if let Ok(record) =
                    cratebase_db::records::find_by_id_raw(app.db(), &collection, &id).await
                {
                    crate::realtime::publish(
                        &app,
                        &collection,
                        cratebase_core::RecordAction::Update,
                        &record,
                    );
                }
            }
        });
    }

    Ok(Json(ReadAllResponse { updated }))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UnreadCountResponse {
    count: i64,
}

/// A cheap `COUNT(*)` against the `(collectionRef, recordRef, readAt,
/// created)` index — see `Collection::default_system_collections`'s
/// comment on `notifications` and `tests::unread_count_uses_the_index`
/// (Postgres/SQLite `EXPLAIN`, `crates/server/tests/`) for the query plan
/// this relies on.
async fn unread_count(
    State(app): State<App>,
    auth: Auth,
) -> ApiResult<Json<UnreadCountResponse>> {
    let row = app
        .db()
        .query_one(
            r#"SELECT COUNT(*) AS "count" FROM "_notifications"
               WHERE "collectionRef" = $1 AND "recordRef" = $2 AND ("readAt" = '' OR "readAt" IS NULL)"#,
            &[
                Sql::from(auth.collection_id.clone()),
                Sql::from(auth.id.clone()),
            ],
        )
        .await
        .map_err(|e| ApiError(e.into()))?;
    let count = row.and_then(|r| r.get_i64("count")).unwrap_or(0);
    Ok(Json(UnreadCountResponse { count }))
}
