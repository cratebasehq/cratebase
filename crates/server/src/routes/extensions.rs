//! `GET/POST/DELETE /api/db/extensions[/{name}]` — superuser-only
//! management of Postgres extensions (PostGIS, pgvector, pg_trgm, ...).
//!
//! # Postgres only
//!
//! There is no SQLite equivalent of a server-side extension, so every
//! handler here 404s with a clear message on a SQLite database rather
//! than pretending to support a no-op "list" that always comes back
//! empty — a 404 is unambiguous about *why* nothing is there.
//!
//! # Why identifiers are validated, not bound
//!
//! `CREATE EXTENSION`/`DROP EXTENSION` and `SCHEMA` take identifiers, not
//! values — Postgres has no bind-parameter syntax for "the name of the
//! extension to create". [`cratebase_core::is_valid_identifier`] (ASCII
//! letters/digits/underscore, not digit-first) is checked before the name
//! ever reaches a format string, the same way `crates/db/src/schema.rs`
//! validates a collection/field name before building DDL from it — this
//! is a safe *allowlist* on the untrusted string, not string
//! interpolation of arbitrary input.
//!
//! # `cascade`
//!
//! A plain `DROP EXTENSION "x"` is refused by Postgres itself when
//! something depends on `x`; this handler only appends `CASCADE` when the
//! caller explicitly asks for it with `?cascade=true`, so a superuser
//! never cascades a drop by accident just by calling the endpoint the
//! same way as usual.
//!
//! # Audit
//!
//! Every install/drop is written to `_audit_log` via `crate::audit`, same
//! as a collection schema change or a settings edit.

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use cratebase_core::is_valid_identifier;
use cratebase_db::engine::{quote_ident, Executor};
use cratebase_filter::Dialect;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::app::App;
use crate::extract::RequireSuperuser;
use crate::http_error::{ApiError, ApiResult};

pub fn router() -> Router<App> {
    Router::new().route(
        "/db/extensions/{name}",
        axum::routing::post(install_extension).delete(drop_extension),
    )
    .route("/db/extensions", get(list_extensions))
}

fn require_postgres(app: &App) -> ApiResult<()> {
    if app.db().dialect() != Dialect::Postgres {
        return Err(ApiError::not_found(
            "Database extensions are only available on Postgres.",
        ));
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct ExtensionInfo {
    name: String,
    #[serde(rename = "defaultVersion")]
    default_version: Option<String>,
    #[serde(rename = "installedVersion")]
    installed_version: Option<String>,
    installed: bool,
    schema: Option<String>,
    comment: Option<String>,
}

/// `GET /api/db/extensions` — every extension `pg_available_extensions`
/// knows about, left-joined against `pg_extension`/`pg_namespace` so an
/// installed one also reports its version and schema.
async fn list_extensions(
    State(app): State<App>,
    _su: RequireSuperuser,
) -> ApiResult<Json<Value>> {
    require_postgres(&app)?;
    let rows = app
        .db()
        .query(
            r#"SELECT a.name, a.default_version, a.installed_version, a.comment,
                      n.nspname AS schema
               FROM pg_available_extensions a
               LEFT JOIN pg_extension e ON e.extname = a.name
               LEFT JOIN pg_namespace n ON n.oid = e.extnamespace
               ORDER BY a.name"#,
            &[],
        )
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let items: Vec<ExtensionInfo> = rows
        .into_iter()
        .map(|r| {
            let installed_version = r
                .get_str("installed_version")
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            ExtensionInfo {
                name: r.get_str("name").unwrap_or_default().to_string(),
                default_version: r
                    .get_str("default_version")
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
                installed: installed_version.is_some(),
                installed_version,
                schema: r.get_str("schema").map(str::to_string),
                comment: r.get_str("comment").map(str::to_string),
            }
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

#[derive(Debug, Deserialize, Default)]
struct InstallBody {
    schema: Option<String>,
}

/// Body is optional: `POST /api/db/extensions/postgis` with no body (or
/// an empty JSON object) is the common case.
async fn install_extension(
    State(app): State<App>,
    Path(name): Path<String>,
    su: RequireSuperuser,
    body: axum::body::Bytes,
) -> ApiResult<Json<Value>> {
    require_postgres(&app)?;
    if !is_valid_identifier(&name) {
        return Err(ApiError::bad_request(
            "Invalid extension name: expected letters, digits and underscores only.",
        ));
    }
    let parsed: InstallBody = if body.is_empty() {
        InstallBody::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|e| ApiError::bad_request(format!("invalid JSON body: {e}")))?
    };
    let schema = parsed.schema.filter(|s| !s.trim().is_empty());
    if let Some(schema) = &schema {
        if !is_valid_identifier(schema) {
            return Err(ApiError::bad_request(
                "Invalid schema name: expected letters, digits and underscores only.",
            ));
        }
    }

    let mut sql = format!("CREATE EXTENSION IF NOT EXISTS {}", quote_ident(&name));
    if let Some(schema) = &schema {
        sql.push_str(&format!(" SCHEMA {}", quote_ident(schema)));
    }
    app.db()
        .execute(&sql, &[])
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    if name.eq_ignore_ascii_case("postgis") {
        app.set_postgis_available(true);
    }

    crate::audit::write(
        &app,
        Some(&su.0.id),
        "extension.create",
        name.clone(),
        json!({ "schema": schema }),
    )
    .await;

    Ok(Json(json!({ "name": name, "installed": true })))
}

#[derive(Debug, Deserialize, Default)]
struct DropQuery {
    #[serde(default)]
    cascade: bool,
}

/// `DELETE /api/db/extensions/{name}[?cascade=true]` — see the module
/// doc's "`cascade`" section for why `CASCADE` is opt-in.
async fn drop_extension(
    State(app): State<App>,
    Path(name): Path<String>,
    Query(q): Query<DropQuery>,
    su: RequireSuperuser,
) -> ApiResult<Json<Value>> {
    require_postgres(&app)?;
    if !is_valid_identifier(&name) {
        return Err(ApiError::bad_request(
            "Invalid extension name: expected letters, digits and underscores only.",
        ));
    }

    let mut sql = format!("DROP EXTENSION {}", quote_ident(&name));
    if q.cascade {
        sql.push_str(" CASCADE");
    }
    app.db()
        .execute(&sql, &[])
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    if name.eq_ignore_ascii_case("postgis") {
        app.set_postgis_available(false);
    }

    crate::audit::write(
        &app,
        Some(&su.0.id),
        "extension.delete",
        name.clone(),
        json!({ "cascade": q.cascade }),
    )
    .await;

    Ok(Json(json!({ "name": name, "installed": false })))
}
