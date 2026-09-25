//! No-code email triggers (`_emailTriggers`): send an `_emailTemplates`
//! template through the ordinary pipeline whenever `event` happens on
//! `collection`, entirely from the dashboard/Records API — no Rust code,
//! no redeploy. Structurally this is `crate::webhooks` with a mail send
//! instead of an HTTP POST; see that module's doc for the reasoning
//! repeated here without re-explanation: `_emailTriggers` is
//! superuser-only end to end (same trust tier as `_webhooks`/
//! `_cron_jobs` — an admin-configured integration point), the dispatch
//! hook is deliberately untagged (it has to watch *every* collection's
//! writes, not just `_emailTriggers`'s own), a write to `_emailTriggers`
//! itself never re-triggers a dispatch for `_emailTriggers`, and
//! delivery is fire-and-forget (`tokio::spawn`) so a slow trigger lookup
//! or send never delays the write's HTTP response — which is also what
//! "never failing the originating request" means here: nothing in this
//! module can turn a successful create/update/delete into an error
//! response, because it never runs synchronously with one.
//!
//! # `toField`
//!
//! Resolved against the written record's own JSON first (a dotted path,
//! `crate::mail_templates`-style — e.g. `customer.email`): if that
//! resolves to a non-empty string, it is used as the recipient.
//! Otherwise, `toField` itself is used verbatim as a literal address
//! (e.g. `ops@example.com`, for a trigger that always mails the same
//! inbox regardless of the record). Neither resolves to anything usable
//! → see "Failure handling" below.
//!
//! # `condition`
//!
//! The same filter-expression language as every other API rule, but
//! evaluated with `crates/filter`'s in-process, no-database
//! [`cratebase_filter::evaluate`] directly against the written record's
//! own fields as bare identifiers (`status = "paid"`) — there is no
//! `@request.*` context to speak of, and no `@record.*` macro either:
//! `@record.old.*` (the pre-write values, useful for "only when a field
//! *changed* to X" conditions on `update`) is **not** exposed, because
//! doing so would mean adding a new macro namespace to `crates/filter`
//! for this one caller. A condition that needs it, or anything else
//! `evaluate()` cannot resolve in-process (a relation path,
//! `@collection.X`, a function call), is treated as **not matched** — a
//! `tracing::warn!`, never a panic and never a false "matched".
//!
//! # `dataMap`
//!
//! Template data defaults to `{ "record": <record json> }`. When
//! `dataMap` is a JSON object, its keys are merged on top — added if new,
//! overriding if they collide (including `record` itself, if a trigger
//! deliberately sets one) — so a trigger can hand a template extra
//! literal context (`{ "appEnv": "production" }`) without losing the
//! default shape every other trigger relies on.
//!
//! # Failure handling
//!
//! A `toField` that resolves to nothing usable, or a `template` that
//! resolves to no `_emailTemplates` row, is recorded as a `_mailLog`
//! failure directly (`crate::mails::create_log_row`/`update_log_status`)
//! — there is no recipient/template to hand `crate::mails::send`, so this
//! bypasses it rather than trying to call it with an invalid input.
//! Everything else that goes wrong (a database error loading triggers, a
//! malformed `condition`) is a `tracing::warn!` with no `_mailLog` row,
//! since no send was ever attempted.

use serde_json::{Map, Value};

use cratebase_core::AppError;
use cratebase_db::engine::Sql;
use cratebase_db::{CollectionResolver, Executor, RequestContext};

use crate::app::App;
use crate::events::RecordEvent;
use crate::hooks::{Event, Handler};
use crate::mails::{self, SendInput};

const COLLECTION: &str = "_emailTriggers";
const EVENTS: [&str; 3] = ["create", "update", "delete"];

/// Bind the reactive dispatch hooks. Called once from
/// [`crate::app::App::bootstrap`].
pub fn bind_hooks(app: &App) {
    for event in EVENTS {
        let a = app.clone();
        let hook = match event {
            "create" => &app.hooks().on_record_after_create_success,
            "update" => &app.hooks().on_record_after_update_success,
            _ => &app.hooks().on_record_after_delete_success,
        };
        hook.bind(Handler::new(move |e: &mut RecordEvent| {
            dispatch_after_success(&a, event, e);
            e.next()
        }));
    }
}

/// Snapshot what the hook needs from `e` and spawn the rest of the work
/// — see the module doc's "Failure handling"/fire-and-forget reasoning.
fn dispatch_after_success(app: &App, event: &str, e: &RecordEvent) {
    if e.collection.name == COLLECTION || e.collection.id == COLLECTION {
        return;
    }
    let app = app.clone();
    let event = event.to_string();
    let collection_name = e.collection.name.clone();
    let collection_id = e.collection.id.clone();
    let record_json = e.record.to_json(Default::default());
    tokio::spawn(async move {
        dispatch(app, event, collection_name, collection_id, record_json).await;
    });
}

/// Load every enabled `_emailTriggers` row targeting `collection_name`/
/// `collection_id` for `event`, and fire each match.
async fn dispatch(
    app: App,
    event: String,
    collection_name: String,
    collection_id: String,
    record_json: Value,
) {
    let Some(triggers_collection) = app.db().collections.get_by_name(COLLECTION) else {
        return;
    };
    let rows = match app
        .db()
        .query(
            &format!(
                r#"SELECT * FROM "{COLLECTION}" WHERE "enabled" = 1 AND "event" = $1 AND ("collection" = $2 OR "collection" = $3)"#
            ),
            &[
                Sql::Text(event),
                Sql::Text(collection_name),
                Sql::Text(collection_id),
            ],
        )
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "failed to load _emailTriggers for dispatch");
            return;
        }
    };
    for row in &rows {
        let record = cratebase_db::records::row_to_record(&triggers_collection, row);
        let app = app.clone();
        let record_json = record_json.clone();
        tokio::spawn(async move {
            fire_one(app, record, record_json).await;
        });
    }
}

/// Evaluate one `_emailTriggers` row's `condition`, resolve `toField`,
/// build the template data and send — or record why it didn't.
async fn fire_one(app: App, trigger: cratebase_core::Record, record_json: Value) {
    let target_collection_name = trigger.get_string("collection");
    let template = trigger.get_string("template");
    let to_field = trigger.get_string("toField");
    let condition = trigger.get_string("condition");

    if !condition.trim().is_empty() {
        match condition_matches(&app, &target_collection_name, &condition, &record_json) {
            Ok(true) => {}
            Ok(false) => return,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    trigger = trigger.id(),
                    "_emailTriggers condition could not be evaluated; treating as not matched"
                );
                return;
            }
        }
    }

    let Some(to) = resolve_to_field(&record_json, &to_field) else {
        log_trigger_failure(
            &app,
            &template,
            &format!(
                "toField '{to_field}' did not resolve to a record field or a literal address."
            ),
        )
        .await;
        return;
    };

    let mut data = Map::new();
    data.insert("record".into(), record_json);
    if let Some(Value::Object(map)) = trigger.get("dataMap") {
        for (k, v) in map {
            data.insert(k.clone(), v.clone());
        }
    }

    let input = SendInput {
        to: vec![mails::Recipient {
            address: to,
            name: String::new(),
        }],
        template: Some(template.clone()),
        data: Value::Object(data),
        ..Default::default()
    };
    if let Err(e) = mails::send(&app, input).await {
        tracing::warn!(error = %e, trigger = trigger.id(), template, "_emailTriggers send failed");
    }
}

/// `crates/filter`'s in-process `evaluate()` against `record_json`'s own
/// fields, rooted at `target_collection_name` (looked up fresh — cheap,
/// and correct even if the collection was renamed since the trigger was
/// saved, as long as the id still matches; see the module doc for why
/// `@record.old.*` isn't supported and what an unresolvable condition
/// does).
fn condition_matches(
    app: &App,
    target_collection_name: &str,
    condition: &str,
    record_json: &Value,
) -> Result<bool, AppError> {
    let ast =
        cratebase_filter::parse_cached(condition).map_err(|e| AppError::internal(e.to_string()))?;
    let collection = app
        .db()
        .collections
        .get(target_collection_name)
        .ok_or_else(|| AppError::internal("trigger's target collection no longer exists"))?;
    let ctx = RequestContext::superuser();
    let resolver =
        CollectionResolver::new(collection, &app.db().collections, &ctx, app.db().dialect());
    let record_map = record_json.as_object().cloned().unwrap_or_default();
    cratebase_filter::evaluate(&ast, &record_map, &resolver)
        .map_err(|e| AppError::internal(e.to_string()))
}

/// A dotted path into `record_json` (`crate::mail_templates`-style),
/// falling back to `to_field` itself as a literal address — see the
/// module doc's "`toField`" section.
fn resolve_to_field(record_json: &Value, to_field: &str) -> Option<String> {
    if let Some(Value::String(s)) = dig(record_json, to_field) {
        if !s.trim().is_empty() {
            return Some(s.clone());
        }
    }
    if to_field.contains('@') {
        return Some(to_field.to_string());
    }
    None
}

fn dig<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.as_object()?.get(segment)?;
    }
    Some(current)
}

/// Records a `_mailLog` failure with no recipient/message to actually
/// send — see the module doc's "Failure handling".
async fn log_trigger_failure(app: &App, template: &str, error: &str) {
    let message = cratebase_mailer::Message {
        to: vec![],
        from: app.mailer().sender().clone(),
        subject: String::new(),
        html: String::new(),
        text: None,
        headers: vec![],
        cc: vec![],
        bcc: vec![],
    };
    match mails::create_log_row(app, &message, Some(template)).await {
        Ok(log_id) => {
            mails::update_log_status(app, &log_id, mails::STATUS_FAILED, Some(error)).await;
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to write _mailLog row for a failed _emailTriggers dispatch");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use serde_json::json;

    async fn test_app() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::new(Config::memory(dir.path()));
        app.bootstrap().await.expect("bootstrap");
        (app, dir)
    }

    #[test]
    fn resolve_to_field_prefers_the_record_path_then_falls_back_to_a_literal() {
        let record = json!({ "customer": { "email": "a@example.com" } });
        assert_eq!(
            resolve_to_field(&record, "customer.email"),
            Some("a@example.com".to_string())
        );
        assert_eq!(
            resolve_to_field(&record, "ops@example.com"),
            Some("ops@example.com".to_string())
        );
        assert_eq!(resolve_to_field(&record, "missing.path"), None);
        assert_eq!(resolve_to_field(&record, "notAField"), None);
    }

    #[tokio::test]
    async fn condition_matches_evaluates_bare_fields_against_the_record() {
        use cratebase_core::field::{Field, FieldKind};

        let (app, _dir) = test_app().await;
        let mut orders =
            cratebase_core::Collection::new("orders", cratebase_core::CollectionType::Base);
        orders.fields.push(Field::new(
            "status",
            FieldKind::Text {
                min: 0,
                max: 0,
                pattern: String::new(),
                autogenerate_pattern: String::new(),
                primary_key: false,
            },
        ));
        app.db()
            .collections
            .insert(&*app.db().engine, &orders)
            .await
            .expect("create orders collection");
        let record = json!({ "status": "paid" });
        assert!(condition_matches(&app, "orders", "status = \"paid\"", &record).unwrap());
        assert!(!condition_matches(&app, "orders", "status = \"pending\"", &record).unwrap());
    }
}
