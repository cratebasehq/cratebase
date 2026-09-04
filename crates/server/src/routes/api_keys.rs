//! `POST /api/api-keys` — mint a new `_api_keys` row and hand back the
//! raw key exactly once.
//!
//! Every other operation on `_api_keys` (list/view/update/delete) goes
//! through the ordinary generic Records API at
//! `/api/collections/_api_keys/records`, already superuser-gated end to
//! end by the collection's own rules (see
//! `cratebase_core::Collection::default_system_collections`'s doc
//! comment on `_api_keys`). This route exists only because that API can
//! never round-trip `key` — it's `hidden`, and the stored value is a
//! one-way hash anyway, so there is nothing to return from a normal
//! create. Minting therefore needs its own endpoint that generates the
//! raw key, hashes it once, and returns the only copy the caller will
//! ever see; see `crate::api_keys` for the generation and hashing.

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use cratebase_core::Record;
use cratebase_db::records;

use crate::app::App;
use crate::extract::RequireSuperuser;
use crate::http_error::{ApiError, ApiJson, ApiResult};

pub fn router() -> Router<App> {
    Router::new().route("/api-keys", post(create))
}

#[derive(Debug, Default, Deserialize)]
struct CreateBody {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Serialize)]
struct CreateResponse {
    id: String,
    name: String,
    prefix: String,
    /// The only time this ever leaves the server — see the module doc.
    key: String,
}

async fn create(
    State(app): State<App>,
    _su: RequireSuperuser,
    ApiJson(body): ApiJson<CreateBody>,
) -> ApiResult<Json<CreateResponse>> {
    let collection = app
        .db()
        .collections
        .get_by_name(crate::api_keys::COLLECTION)
        .ok_or_else(|| ApiError::internal(format!("{} is missing", crate::api_keys::COLLECTION)))?;

    let generated = crate::api_keys::generate();
    let hash = cratebase_auth::hash_password_async(&generated.raw)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let mut record = Record::new(collection.clone());
    record.set("name", Value::String(body.name.clone()));
    record.set("key", Value::String(hash));
    record.set("prefix", Value::String(generated.prefix.clone()));
    record.set("enabled", Value::Bool(true));
    records::create(app.db(), &app.db().collections, &mut record).await?;

    Ok(Json(CreateResponse {
        id: record.id().to_string(),
        name: body.name,
        prefix: generated.prefix,
        key: generated.raw,
    }))
}
