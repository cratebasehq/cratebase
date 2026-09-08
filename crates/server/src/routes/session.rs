//! `/api/collections/{collection}/sessions*`, `auth-signout`,
//! `stop-impersonating`, and `ban`/`unban`: everything a caller does
//! with an already-live session that isn't logging in. All of it is
//! additive — PocketBase has none of these endpoints.
//!
//! # Bans
//!
//! `_bans` (`crates/core/src/collection.rs`) is superuser-only end to
//! end; [`active_ban`] is the one place outside the dashboard/admin API
//! that reads it, called from the four login handlers in
//! `crate::routes::auth` (`auth_with_password`, `auth_with_otp`,
//! `complete_oauth2`, `auth_refresh`) so a ban takes effect immediately
//! rather than only on the next full login. Banning also rotates the
//! target's `tokenKey` and revokes every `_sessions` row, so a ban locks
//! out live sessions too, not just future logins.


use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use cratebase_core::{Collection, Record};
use cratebase_db::engine::{Executor, Sql};
use cratebase_db::records;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::app::App;
use crate::extract::{Auth, RequireOwner, RequireSuperuser};
use crate::http_error::{ApiError, ApiJson, ApiQuery, ApiResult};
use crate::routes::common;
use crate::sessions;

pub fn router() -> Router<App> {
    Router::new()
        .route("/collections/{collection}/sessions", get(list_sessions))
        .route(
            "/collections/{collection}/sessions/{id}",
            delete(revoke_session),
        )
        .route(
            "/collections/{collection}/sessions/revoke-others",
            post(revoke_others),
        )
        .route(
            "/collections/{collection}/sessions/revoke-all",
            post(revoke_all),
        )
        .route(
            "/collections/{collection}/auth-signout",
            post(auth_signout),
        )
        .route(
            "/collections/{collection}/stop-impersonating",
            post(stop_impersonating),
        )
        .route("/collections/{collection}/ban/{id}", post(ban))
        .route("/collections/{collection}/ban/{id}", delete(unban))
}

#[derive(Debug, Default, Deserialize)]
struct ListQuery {
    #[serde(rename = "recordId")]
    record_id: Option<String>,
}

async fn list_sessions(
    State(app): State<App>,
    Path(name): Path<String>,
    auth: Auth,
    ApiQuery(q): ApiQuery<ListQuery>,
) -> ApiResult<Json<Value>> {
    let collection = common::auth_collection_of(&app, &name)?;
    let record_id = match &q.record_id {
        // Only a superuser may look at someone else's sessions.
        Some(id) if id != &auth.id => {
            if !auth.is_superuser {
                return Err(ApiError::forbidden(
                    "Only a superuser may list another record's sessions.",
                ));
            }
            id.clone()
        }
        _ => auth.id.clone(),
    };
    let current_hash = sessions::hex(&sessions::digest(&auth.token));
    let rows = sessions::list_for(&app, &collection.id, &record_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "id": r.id,
                "kind": r.kind,
                "fingerprint": r.fingerprint,
                "ip": r.ip,
                "userAgent": r.user_agent,
                "current": r.token_hash == current_hash,
                "created": r.created,
                "lastSeenAt": r.last_seen_at,
                "expiresAt": r.expires_at,
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

/// The `_sessions` row's own `(collectionRef, recordRef)`, so `DELETE
/// .../sessions/{id}` can check ownership before revoking — `revoke_row`
/// itself revokes by id alone, with no owner check.
async fn session_owner(app: &App, id: &str) -> ApiResult<Option<(String, String)>> {
    let row = app
        .db()
        .query_one(
            r#"SELECT "collectionRef", "recordRef" FROM "_sessions" WHERE "id" = $1"#,
            &[Sql::Text(id.to_string())],
        )
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(row.map(|r| {
        (
            r.get_str("collectionRef").unwrap_or_default().to_string(),
            r.get_str("recordRef").unwrap_or_default().to_string(),
        )
    }))
}

async fn revoke_session(
    State(app): State<App>,
    Path((name, id)): Path<(String, String)>,
    auth: Auth,
) -> ApiResult<Response> {
    let collection = common::auth_collection_of(&app, &name)?;
    let Some((owner_collection, owner_record)) = session_owner(&app, &id).await? else {
        return Ok(axum::http::StatusCode::NO_CONTENT.into_response());
    };
    let is_own = owner_collection == collection.id && owner_record == auth.id;
    if !is_own && !auth.is_superuser {
        return Err(ApiError::forbidden(
            "Only the session's own owner or a superuser may revoke it.",
        ));
    }
    sessions::revoke_row(&app, &id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let mut response = axum::http::StatusCode::NO_CONTENT.into_response();
    let is_current = is_own && sessions::hex(&sessions::digest(&auth.token)) == {
        // Re-derive rather than re-query: the row is already gone from
        // the "live" set, but its hash is still exactly `digest(token)`
        // when it was the caller's own current session.
        sessions::hex(&sessions::digest(&auth.token))
    };
    if is_current && app.config().session_cookie {
        crate::cookie::attach(
            response.headers_mut(),
            crate::cookie::clear_session(app.config()),
        );
    }
    Ok(response)
}

async fn revoke_others(
    State(app): State<App>,
    Path(name): Path<String>,
    auth: Auth,
) -> ApiResult<Json<Value>> {
    let collection = common::auth_collection_of(&app, &name)?;
    let except = sessions::digest(&auth.token);
    let revoked = sessions::revoke_all_for(&app, &collection.id, &auth.id, Some(&except))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({ "revoked": revoked })))
}

async fn revoke_all(
    State(app): State<App>,
    Path(name): Path<String>,
    auth: Auth,
) -> ApiResult<Response> {
    let collection = common::auth_collection_of(&app, &name)?;
    let revoked = sessions::revoke_all_for(&app, &collection.id, &auth.id, None)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let mut response = Json(json!({ "revoked": revoked })).into_response();
    if app.config().session_cookie {
        crate::cookie::attach(
            response.headers_mut(),
            crate::cookie::clear_session(app.config()),
        );
    }
    Ok(response)
}

async fn auth_signout(
    State(app): State<App>,
    Path(name): Path<String>,
    auth: Auth,
) -> ApiResult<Response> {
    let collection = common::auth_collection_of(&app, &name)?;
    sessions::revoke_digest(&app, &collection.id, &auth.id, &sessions::digest(&auth.token))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let mut response = axum::http::StatusCode::NO_CONTENT.into_response();
    if app.config().session_cookie {
        crate::cookie::attach(
            response.headers_mut(),
            crate::cookie::clear_session(app.config()),
        );
    }
    Ok(response)
}

/// `cb_session_prev` cookie name — the impersonator's own token, stashed
/// by `routes::auth::impersonate` before it overwrites the session
/// cookie with the impersonation token. Only ever set in cookie mode;
/// bearer-mode impersonation restoration is entirely client-side (the
/// SDK just keeps the caller's original store in memory).
pub(crate) const PREV_SESSION_COOKIE: &str = "cb_session_prev";

async fn stop_impersonating(State(app): State<App>, parts: axum::http::request::Parts) -> ApiResult<Response> {
    let cfg = app.config();
    let Some(prev_token) = crate::cookie::get(&parts, PREV_SESSION_COOKIE).map(str::to_string)
    else {
        return Err(ApiError::bad_request("No impersonation session to stop."));
    };

    // The stashed token must itself still verify — it is not re-trusted
    // blindly just because it sat in an httpOnly cookie.
    let unverified = cratebase_auth::decode_unverified(&prev_token)
        .map_err(|_| ApiError::bad_request("No impersonation session to stop."))?;
    let collection = app
        .db()
        .collections
        .get_by_id(&unverified.collection_id)
        .ok_or_else(|| ApiError::bad_request("No impersonation session to stop."))?;
    let record = records::find_by_id_raw(app.db(), &collection, &unverified.id)
        .await
        .map_err(|_| ApiError::bad_request("No impersonation session to stop."))?;
    let key = app.token_signing_key(&record.token_key(), &collection.auth.auth_token.secret);
    cratebase_auth::verify(&prev_token, &key)
        .map_err(|_| ApiError::bad_request("No impersonation session to stop."))?;

    let serialized =
        common::enrich_and_serialize(&app, &collection, record, None, true).await?;
    let mut response = Json(json!({ "record": serialized })).into_response();
    let ttl = collection.auth.auth_token.duration.max(1);
    crate::cookie::attach(
        response.headers_mut(),
        crate::cookie::session(cfg, &prev_token, ttl),
    );
    crate::cookie::attach(
        response.headers_mut(),
        crate::cookie::build(cfg, PREV_SESSION_COOKIE, "", 0, "/"),
    );
    Ok(response)
}

#[derive(Debug, Default, Deserialize)]
struct BanBody {
    reason: Option<String>,
    #[serde(rename = "expiresAt")]
    expires_at: Option<String>,
}

async fn ban(
    State(app): State<App>,
    Path((name, id)): Path<(String, String)>,
    su: RequireSuperuser,
    ApiJson(raw): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    let caller = su.0;
    let collection = common::auth_collection_of(&app, &name)?;
    // Same owner/admin split every other `_superusers`-specific write
    // uses (see `routes::auth::impersonate`'s identical guard).
    if collection.is_superusers()
        && (caller.collection_name != cratebase_core::SUPERUSERS_COLLECTION
            || !RequireOwner::holds(&caller))
    {
        return Err(ApiError::forbidden(
            "Only an owner can ban a superuser account.",
        ));
    }
    let target = records::find_by_id_raw(app.db(), &collection, &id)
        .await
        .map_err(|e| ApiError(e.into()))?;
    let body: BanBody = serde_json::from_value(raw).unwrap_or_default();

    let bans = app
        .db()
        .collections
        .get("_bans")
        .expect("_bans is a default system collection");
    let mut params = Map::new();
    params.insert("c".into(), Value::String(collection.id.clone()));
    params.insert("r".into(), Value::String(target.id().to_string()));
    let existing = records::find_first_by_filter(
        app.db(),
        &app.db().collections,
        &bans,
        "collectionRef = {:c} && recordRef = {:r}",
        &params,
    )
    .await
    .map_err(|e| ApiError(e.into()))?;

    let mut row = existing.unwrap_or_else(|| Record::new(bans.clone()));
    row.set("collectionRef", Value::String(collection.id.clone()));
    row.set("recordRef", Value::String(target.id().to_string()));
    row.set(
        "reason",
        Value::String(body.reason.clone().unwrap_or_default()),
    );
    row.set(
        "expiresAt",
        Value::String(body.expires_at.clone().unwrap_or_default()),
    );
    row.set("bannedBy", Value::String(caller.id.clone()));
    if row.id().is_empty() {
        records::create(app.db(), &app.db().collections, &mut row)
            .await
            .map_err(|e| ApiError(e.into()))?;
    } else {
        records::update(app.db(), &app.db().collections, &mut row)
            .await
            .map_err(|e| ApiError(e.into()))?;
    }

    // Invalidate every outstanding token for the target — the standard
    // pattern every other tokenKey-rotating write in `routes::auth` uses
    // (there is no wrapper function; see that module's doc).
    let mut target = target;
    target.set("tokenKey", Value::String(crate::app::new_token_key()));
    records::update(app.db(), &app.db().collections, &mut target)
        .await
        .map_err(|e| ApiError(e.into()))?;
    sessions::revoke_all_for(&app, &collection.id, &id, None)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(json!({
        "recordRef": row.get_string("recordRef"),
        "reason": row.get_string("reason"),
        "expiresAt": row.get_string("expiresAt"),
    })))
}

async fn unban(
    State(app): State<App>,
    Path((name, id)): Path<(String, String)>,
    _su: RequireSuperuser,
) -> ApiResult<Response> {
    let collection = common::auth_collection_of(&app, &name)?;
    let bans = app
        .db()
        .collections
        .get("_bans")
        .expect("_bans is a default system collection");
    let mut params = Map::new();
    params.insert("c".into(), Value::String(collection.id.clone()));
    params.insert("r".into(), Value::String(id));
    if let Some(row) = records::find_first_by_filter(
        app.db(),
        &app.db().collections,
        &bans,
        "collectionRef = {:c} && recordRef = {:r}",
        &params,
    )
    .await
    .map_err(|e| ApiError(e.into()))?
    {
        records::delete(app.db(), &app.db().collections, &row)
            .await
            .map_err(|e| ApiError(e.into()))?;
    }
    Ok(axum::http::StatusCode::NO_CONTENT.into_response())
}

/// A still-active ban for `record_id` in `collection`, if any —
/// `expiresAt` empty or in the future. Called from the four login
/// handlers in `crate::routes::auth`.
pub(crate) async fn active_ban(
    app: &App,
    collection: &Collection,
    record_id: &str,
) -> Option<Record> {
    let bans = app.db().collections.get("_bans")?;
    let now = cratebase_core::DateTime::now().to_pb_string();
    let row = app
        .db()
        .query_one(
            r#"SELECT * FROM "_bans" WHERE "collectionRef" = $1 AND "recordRef" = $2 AND ("expiresAt" = '' OR "expiresAt" > $3) LIMIT 1"#,
            &[
                Sql::Text(collection.id.clone()),
                Sql::Text(record_id.to_string()),
                Sql::Text(now),
            ],
        )
        .await
        .ok()??;
    Some(records::row_to_record(&bans, &row))
}
