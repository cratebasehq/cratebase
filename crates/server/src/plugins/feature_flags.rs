//! Feature-flag plugin: the smallest possible extension-point exercise that
//! also needs its own collection, per `ROADMAP.md`. No lifecycle hooks, no
//! new expression language — a flag is a record in an ordinary `Base`
//! collection (`_feature_flags`), evaluated with the exact same rule
//! engine that already backs `listRule`/`viewRule`/`createRule`.
//!
//! - `enabled` is the on/off switch.
//! - `rule` is an optional extra filter expression, evaluated against the
//!   requester's `@request.auth.*` context (and the flag's own fields, via
//!   `data`) — e.g. `@request.auth.record.plan = "pro"` to target only
//!   pro-plan users. Empty/absent `rule` means "everyone `enabled` covers."
//! - Superusers always see a flag as enabled, same bypass every other rule
//!   in Cratebase gives them.
//!
//! SDK: `cb.featureFlags.isEnabled("key")` — see `sdk/js/src/feature-flags-service.ts`.

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use cratebase_core::field::{Field, FieldOptions, FieldType};
use cratebase_core::{new_id, now, AuthOptions, Collection, CollectionType};
use cratebase_db::records::{self, ListParams};
use cratebase_db::resolver::{evaluate_record_rule, RequestContext};
use cratebase_db::{collections, DbError, DbResult, Db};
use serde_json::{json, Value};

use crate::extract::CurrentAuth;
use crate::plugin::Plugin;
use crate::state::AppState;

const COLLECTION_NAME: &str = "_feature_flags";

fn feature_flags_collection() -> Collection {
    let ts = now();
    Collection {
        id: new_id(),
        name: COLLECTION_NAME.to_string(),
        collection_type: CollectionType::Base,
        schema: vec![
            Field {
                id: new_id(),
                name: "key".into(),
                field_type: FieldType::Text,
                required: true,
                unique: true,
                options: FieldOptions::default(),
            },
            Field {
                id: new_id(),
                name: "enabled".into(),
                field_type: FieldType::Bool,
                required: true,
                unique: false,
                options: FieldOptions::default(),
            },
            Field {
                id: new_id(),
                name: "rule".into(),
                field_type: FieldType::Text,
                required: false,
                unique: false,
                options: FieldOptions::default(),
            },
        ],
        // Flags are cheap, non-sensitive booleans — public read so a
        // client can check them before authenticating. Writes are
        // superuser-only (`None` == admin-only, same convention every
        // other collection uses).
        list_rule: Some(String::new()),
        view_rule: Some(String::new()),
        create_rule: None,
        update_rule: None,
        delete_rule: None,
        auth_options: AuthOptions::default(),
        view_query: None,
        created: ts.clone(),
        updated: ts,
    }
}

/// Seed the `_feature_flags` collection on first boot, mirroring
/// `cratebase_db::system::ensure_default_collections`. No-op once it
/// exists.
pub async fn ensure_feature_flags_collection(db: &Db) -> DbResult<()> {
    match collections::get_collection_by_name(db, COLLECTION_NAME).await {
        Ok(_) => Ok(()),
        Err(DbError::NotFound) => {
            collections::create_collection(db, &feature_flags_collection()).await
        }
        Err(e) => Err(e),
    }
}

/// `GET /api/plugins/feature-flags/{key}` → `{"key": ..., "enabled": bool}`.
/// An unknown key resolves to `enabled: false` rather than 404 — a typo'd
/// flag key should degrade to "off," not break the caller.
async fn check_flag(
    State(state): State<AppState>,
    Path(key): Path<String>,
    CurrentAuth(auth): CurrentAuth,
) -> Json<Value> {
    let off = || Json(json!({ "key": key, "enabled": false }));

    let Ok(collection) = collections::get_collection_by_name(&state.db, COLLECTION_NAME).await
    else {
        return off();
    };

    let ctx = RequestContext { auth, data: None };
    let Ok(result) = records::list_records(
        &state.db,
        &collection,
        &ctx,
        None,
        ListParams {
            filter: None,
            sort: None,
            page: 1,
            per_page: 500,
        },
    )
    .await
    else {
        return off();
    };

    let Some(flag) = result
        .items
        .iter()
        .find(|r| r["key"].as_str() == Some(key.as_str()))
    else {
        return off();
    };

    if !flag["enabled"].as_bool().unwrap_or(false) {
        return off();
    }

    let rule = flag["rule"].as_str().filter(|s| !s.is_empty());
    let enabled = match rule {
        None => true,
        Some(expr) => {
            let flag_data = flag.as_object().cloned().unwrap_or_default();
            let eval_ctx = RequestContext {
                auth: ctx.auth.clone(),
                data: Some(flag_data),
            };
            evaluate_record_rule(&state.db, &Some(expr.to_string()), &collection, &eval_ctx)
                .await
                .unwrap_or(false)
        }
    };

    Json(json!({ "key": key, "enabled": enabled }))
}

pub struct FeatureFlagsPlugin;

impl Plugin for FeatureFlagsPlugin {
    fn name(&self) -> &'static str {
        "feature-flags"
    }

    fn setup<'a>(
        &'a self,
        db: &'a Db,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            ensure_feature_flags_collection(db).await?;
            Ok(())
        })
    }

    fn routes(&self) -> Option<Router<AppState>> {
        Some(Router::new().route("/{key}", get(check_flag)))
    }
}
