//! `.../totp/setup|confirm|disable|backup-codes/regenerate` and
//! `auth-with-totp`: per-record TOTP 2FA, backed by the `_totps` system
//! collection (`crates/core/src/collection.rs`) and integrated with the
//! existing `_mfas` challenge flow rather than a second one —
//! [`crate::routes::auth::mfa_gate`] now also gates a login on
//! [`totp_confirmed_for`], independent of the collection's own
//! `authOptions.mfa`.
//!
//! Every endpoint here acts on the *caller's own* record (there is no
//! `{id}` in any of these paths) — a superuser resetting someone else's
//! TOTP does it the same way it resets anything else system-collection
//! shaped: `DELETE /api/collections/_totps/records/{id}` (superuser-only
//! by `_totps`'s own `deleteRule`, same as `_sessions`/`_bans`), which
//! the dashboard's record drawer wraps in a button.
//!
//! # Storage
//!
//! `secret` is encrypted with `CB_ENCRYPTION` when set (the same
//! `ParamCipher` `_params` values use — see `cratebase_db::params`),
//! plaintext otherwise. `backupCodes` is a JSON array of SHA-256 hex
//! hashes (like `_otps`'s own codes); a hash is removed from the array
//! the instant it's consumed, so "already used" needs no separate flag.
//! `lastUsedStep` is TOTP's replay guard.

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use cratebase_core::Record;
use cratebase_db::params::ParamCipher;
use cratebase_db::records;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::app::App;
use crate::extract::{Auth, RequestInfo};
use crate::http_error::{ApiError, ApiJson, ApiResult};
use crate::routes::auth::{
    mfa_gate, mfa_pending_response, record_login_origin, respond_with_token, MfaGate,
};
use crate::routes::common;

/// PocketBase-shaped `validation_required`-style wording is out of scope
/// here (these are new endpoints, not a PocketBase-compatible surface),
/// so messages are plain English.
const ALREADY_ENABLED: &str = "TOTP is already enabled for this account.";
const NOT_SET_UP: &str = "TOTP has not been set up for this account.";
const NOT_CONFIRMED: &str = "TOTP setup has not been confirmed yet.";
const INVALID_CODE: &str = "Invalid TOTP code or backup code.";

pub fn router() -> Router<App> {
    Router::new()
        .route("/collections/{collection}/totp/setup", post(setup))
        .route("/collections/{collection}/totp/confirm", post(confirm))
        .route("/collections/{collection}/totp/disable", post(disable))
        .route(
            "/collections/{collection}/totp/backup-codes/regenerate",
            post(regenerate_backup_codes),
        )
        .route(
            "/collections/{collection}/auth-with-totp",
            post(auth_with_totp),
        )
}

/// `true` when `(collection_id, record_id)` has a *confirmed* `_totps`
/// row — the one thing [`crate::routes::auth::mfa_gate`] needs to know,
/// independent of the collection's own `authOptions.mfa`.
pub(crate) async fn totp_confirmed_for(
    app: &App,
    collection_id: &str,
    record_id: &str,
) -> ApiResult<bool> {
    Ok(totp_row(app, collection_id, record_id)
        .await?
        .is_some_and(|r| r.get_bool("confirmed")))
}

async fn totp_collection(app: &App) -> cratebase_core::Collection {
    (*app
        .db()
        .collections
        .get("_totps")
        .expect("_totps is a default system collection"))
    .clone()
}

async fn totp_row(app: &App, collection_id: &str, record_id: &str) -> ApiResult<Option<Record>> {
    let totps = app
        .db()
        .collections
        .get("_totps")
        .expect("_totps is a default system collection");
    let mut params = Map::new();
    params.insert("c".into(), Value::String(collection_id.to_string()));
    params.insert("r".into(), Value::String(record_id.to_string()));
    records::find_first_by_filter(
        app.db(),
        &app.db().collections,
        &totps,
        "collectionRef = {:c} && recordRef = {:r}",
        &params,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))
}

fn decrypt_secret(row: &Record) -> String {
    ParamCipher::from_env().decrypt(&row.get_string("secret"))
}

fn backup_hashes(row: &Record) -> Vec<String> {
    row.get_string_list("backupCodes")
}

/// The identity string an authenticator app shows next to the issuer —
/// the auth collection's own `email` field when the record has one
/// (every default auth collection does), falling back to the record id
/// so setup never fails outright for a hand-rolled auth collection with
/// no `email` field.
fn account_label(record: &Record) -> String {
    let email = record.get_string("email");
    if email.is_empty() {
        record.id().to_string()
    } else {
        email
    }
}

async fn setup(
    State(app): State<App>,
    Path(name): Path<String>,
    auth: Auth,
) -> ApiResult<Json<Value>> {
    let collection = common::auth_collection_of(&app, &name)?;
    if auth.collection.id != collection.id {
        return Err(ApiError::forbidden(
            "This token does not belong to this collection.",
        ));
    }
    if totp_confirmed_for(&app, &collection.id, &auth.id).await? {
        return Err(ApiError::bad_request(ALREADY_ENABLED));
    }

    let secret = cratebase_auth::generate_totp_secret();
    let issuer = {
        let name = app.settings().meta.app_name.clone();
        if name.trim().is_empty() {
            "Cratebase".to_string()
        } else {
            name
        }
    };
    let uri = cratebase_auth::otpauth_uri(&secret, &account_label(&auth.record), &issuer);
    let stored_secret = ParamCipher::from_env()
        .encrypt(&secret)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let totps = totp_collection(&app).await;
    let mut row = match totp_row(&app, &collection.id, &auth.id).await? {
        Some(existing) => existing,
        None => Record::new(std::sync::Arc::new(totps)),
    };
    if row.id().is_empty() {
        row.set("collectionRef", Value::String(collection.id.clone()));
        row.set("recordRef", Value::String(auth.id.clone()));
    }
    row.set("secret", Value::String(stored_secret));
    row.set("confirmed", Value::Bool(false));
    row.set("backupCodes", Value::Array(vec![]));
    row.set("lastUsedStep", json!(0));

    if row.id().is_empty() {
        records::create(app.db(), &app.db().collections, &mut row)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    } else {
        records::update(app.db(), &app.db().collections, &mut row)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    }

    Ok(Json(json!({ "secret": secret, "uri": uri })))
}

#[derive(Debug, Default, Deserialize)]
struct CodeBody {
    #[serde(default)]
    code: String,
}

async fn confirm(
    State(app): State<App>,
    Path(name): Path<String>,
    auth: Auth,
    ApiJson(body): ApiJson<CodeBody>,
) -> ApiResult<Json<Value>> {
    let collection = common::auth_collection_of(&app, &name)?;
    if auth.collection.id != collection.id {
        return Err(ApiError::forbidden(
            "This token does not belong to this collection.",
        ));
    }
    let Some(mut row) = totp_row(&app, &collection.id, &auth.id).await? else {
        return Err(ApiError::bad_request(NOT_SET_UP));
    };
    if row.get_bool("confirmed") {
        return Err(ApiError::bad_request(ALREADY_ENABLED));
    }

    let secret = decrypt_secret(&row);
    let now = chrono::Utc::now().timestamp();
    let matched = cratebase_auth::verify_totp(&secret, &body.code, now, 0)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let Some(step) = matched else {
        return Err(ApiError::bad_request(INVALID_CODE));
    };

    let backup_codes = cratebase_auth::generate_backup_codes(10);
    let hashed: Vec<Value> = backup_codes
        .iter()
        .map(|c| Value::String(cratebase_auth::hash_otp(c)))
        .collect();

    row.set("confirmed", Value::Bool(true));
    row.set("backupCodes", Value::Array(hashed));
    row.set("lastUsedStep", json!(step));
    records::update(app.db(), &app.db().collections, &mut row)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(json!({ "backupCodes": backup_codes })))
}

#[derive(Debug, Default, Deserialize)]
struct DisableBody {
    #[serde(default)]
    code: String,
    #[serde(default)]
    password: String,
}

async fn disable(
    State(app): State<App>,
    Path(name): Path<String>,
    auth: Auth,
    ApiJson(body): ApiJson<DisableBody>,
) -> ApiResult<Response> {
    let collection = common::auth_collection_of(&app, &name)?;
    if auth.collection.id != collection.id {
        return Err(ApiError::forbidden(
            "This token does not belong to this collection.",
        ));
    }
    let Some(row) = totp_row(&app, &collection.id, &auth.id).await? else {
        return Err(ApiError::bad_request(NOT_SET_UP));
    };
    if !row.get_bool("confirmed") {
        return Err(ApiError::bad_request(NOT_CONFIRMED));
    }

    let verified = if !body.password.is_empty() {
        cratebase_auth::verify_password_async(&body.password, &auth.record.password_hash()).await
    } else if !body.code.is_empty() {
        verify_code_or_backup(&row, &body.code)?.is_some()
    } else {
        false
    };
    if !verified {
        return Err(ApiError::bad_request(INVALID_CODE));
    }

    records::delete(app.db(), &app.db().collections, &row)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(axum::http::StatusCode::NO_CONTENT.into_response())
}

async fn regenerate_backup_codes(
    State(app): State<App>,
    Path(name): Path<String>,
    auth: Auth,
) -> ApiResult<Json<Value>> {
    let collection = common::auth_collection_of(&app, &name)?;
    if auth.collection.id != collection.id {
        return Err(ApiError::forbidden(
            "This token does not belong to this collection.",
        ));
    }
    let Some(mut row) = totp_row(&app, &collection.id, &auth.id).await? else {
        return Err(ApiError::bad_request(NOT_SET_UP));
    };
    if !row.get_bool("confirmed") {
        return Err(ApiError::bad_request(NOT_CONFIRMED));
    }

    let backup_codes = cratebase_auth::generate_backup_codes(10);
    let hashed: Vec<Value> = backup_codes
        .iter()
        .map(|c| Value::String(cratebase_auth::hash_otp(c)))
        .collect();
    row.set("backupCodes", Value::Array(hashed));
    records::update(app.db(), &app.db().collections, &mut row)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(json!({ "backupCodes": backup_codes })))
}

/// Which credential `code` matched against a `_totps` row.
enum CodeMatch {
    /// A live TOTP code — `.0` is the step it matched, the new
    /// `lastUsedStep` once persisted.
    Totp(i64),
    /// One of the row's remaining backup codes — its hash, so the caller
    /// can remove exactly that entry.
    BackupCode(String),
}

/// Checks `code` against the row's live TOTP secret first, then every
/// remaining backup code hash. Read-only — persisting the replay-
/// protection update this match implies is [`consume`]'s job, so a
/// caller that's about to delete the row outright (`disable`) can skip
/// it.
fn verify_code_or_backup(row: &Record, code: &str) -> ApiResult<Option<CodeMatch>> {
    let secret = decrypt_secret(row);
    let last_used_step = row.get_f64("lastUsedStep") as i64;
    let now = chrono::Utc::now().timestamp();
    let totp_match = cratebase_auth::verify_totp(&secret, code, now, last_used_step)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if let Some(step) = totp_match {
        return Ok(Some(CodeMatch::Totp(step)));
    }
    let normalized = cratebase_auth::normalize_backup_code(code);
    let target_hash = cratebase_auth::hash_otp(&normalized);
    if backup_hashes(row).iter().any(|h| h == &target_hash) {
        return Ok(Some(CodeMatch::BackupCode(target_hash)));
    }
    Ok(None)
}

/// Verifies `code`/a backup code against `row` and, on a match, persists
/// whichever replay-protection update it implies (a new `lastUsedStep`,
/// or the consumed backup code removed from `backupCodes`) before
/// returning whether it matched at all.
async fn verify_and_consume(app: &App, row: &mut Record, code: &str) -> ApiResult<bool> {
    let Some(matched) = verify_code_or_backup(row, code)? else {
        return Ok(false);
    };
    match matched {
        CodeMatch::Totp(step) => row.set("lastUsedStep", json!(step)),
        CodeMatch::BackupCode(hash) => {
            let remaining: Vec<Value> = backup_hashes(row)
                .into_iter()
                .filter(|h| h != &hash)
                .map(Value::String)
                .collect();
            row.set("backupCodes", Value::Array(remaining));
        }
    }
    records::update(app.db(), &app.db().collections, row)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(true)
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthWithTotpBody {
    #[serde(default)]
    mfa_id: String,
    #[serde(default)]
    code: String,
}

/// `POST .../auth-with-totp` completes a pending `_mfas` challenge
/// (opened by whatever first factor just succeeded) with a TOTP code or
/// a backup code — the second half of [`crate::routes::auth::mfa_gate`]'s
/// per-record TOTP path.
async fn auth_with_totp(
    State(app): State<App>,
    Path(name): Path<String>,
    info: RequestInfo,
    headers: axum::http::HeaderMap,
    peer: crate::middleware::client_ip::PeerAddr,
    ApiJson(body): ApiJson<AuthWithTotpBody>,
) -> ApiResult<Response> {
    let collection = common::auth_collection_of(&app, &name)?;
    if body.mfa_id.is_empty() || body.code.is_empty() {
        return Err(ApiError::bad_request("mfaId and code are required."));
    }

    let mfas = app
        .db()
        .collections
        .get("_mfas")
        .expect("_mfas is a default system collection");
    let pending = records::find_by_id_raw(app.db(), &mfas, &body.mfa_id)
        .await
        .ok()
        .filter(|r| r.get_string("collectionRef") == collection.id);
    let Some(pending) = pending else {
        return Err(ApiError::bad_request("Invalid or expired MFA session."));
    };
    let record_id = pending.get_string("recordRef");
    let record = records::find_by_id_raw(app.db(), &collection, &record_id)
        .await
        .map_err(|_| ApiError::bad_request("Invalid or expired MFA session."))?;

    let Some(mut totp) = totp_row(&app, &collection.id, &record_id).await? else {
        return Err(ApiError::bad_request(INVALID_CODE));
    };
    if !totp.get_bool("confirmed") {
        return Err(ApiError::bad_request(INVALID_CODE));
    }
    if !verify_and_consume(&app, &mut totp, &body.code).await? {
        return Err(ApiError::bad_request(INVALID_CODE));
    }

    // The credential just verified above; `mfa_gate` only needs to
    // consume the pending `_mfas` row for a *different* method than
    // whatever the first factor used — always true here, since "totp"
    // never opens a session as a first factor.
    match mfa_gate(&app, &collection, &record, "totp", Some(&body.mfa_id)).await? {
        MfaGate::Pending(mfa_id) => Ok(mfa_pending_response(mfa_id)),
        MfaGate::Passed => {
            let origin = record_login_origin(&app, &collection, &record, &headers, peer).await;
            respond_with_token(
                &app,
                &collection,
                record,
                info,
                Value::Object(Map::new()),
                |hooks| &hooks.on_record_auth_with_totp_request,
                Some(("totp", origin)),
                None,
            )
            .await
        }
    }
}
