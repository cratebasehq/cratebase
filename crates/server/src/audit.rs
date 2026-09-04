//! The admin audit log (`_audit_log`): one row per consequential
//! dashboard/API action taken by a superuser — a collection schema
//! change, a settings edit, a superuser account mutation, or an ordinary
//! record deleted by a superuser bypassing that collection's own
//! `deleteRule` — so an operator can answer "who changed what, when"
//! after the fact.
//!
//! # Why the `*Request` hooks, not the inner lifecycle ones
//!
//! `on_collection_create`/`on_record_update`/etc. (`CollectionEvent`,
//! `RecordEvent`) carry the transactional `TxApp` but no [`crate::extract::Auth`]
//! — there is no actor to attribute the row to at that layer. The
//! `*Request` events one level out (`CollectionRequestEvent`,
//! `RecordRequestEvent`, `SettingsUpdateEvent`'s `request`) do carry it,
//! because `crate::routes::collections::apply` and the record routes are
//! handed the caller's [`crate::extract::Auth`] directly. Every handler
//! below therefore follows the finalizer-style pattern
//! `crates/server/src/hooks.rs`'s module doc describes: call `e.next()`
//! first (so a rejection earlier in the chain skips the write and this
//! code never runs), then read the *outcome* the event now carries and
//! write one audit row from it.
//!
//! # What counts as a superuser "bypass" for `record.delete`
//!
//! Logging every record delete would be enormous, pointless noise — an
//! ordinary user deleting their own comment under a normal `deleteRule`
//! is not an audit-worthy event. `crates/server/src/routes/common.rs`'s
//! `record_matches_rule` answers "does this row pass the rule for this
//! caller?" by returning `true` immediately, without evaluating the rule
//! at all, whenever `ctx.is_superuser()` holds — that early return *is*
//! the bypass this module cares about. [`bind_record_delete_bypass`]
//! reuses exactly that definition: a delete is logged as `record.delete`
//! whenever the acting [`crate::extract::Auth`] is a superuser, full
//! stop, regardless of whether the row's own `deleteRule` would also
//! have allowed it for an ordinary caller. `_superusers` deletes are
//! excluded here (they get the more specific `superuser.delete` instead,
//! from [`bind_superuser_lifecycle`]) to avoid double-logging the same
//! action under two different names.
//!
//! # Why `_audit_log` itself can't just use a rule
//!
//! `list_rule`/`view_rule`/`create_rule` staying `None` on
//! `Collection::default_system_collections`' `_audit_log` means
//! "superuser only" — but `None` still lets a superuser *through*, which
//! is exactly wrong for `update`/`delete`: an audit log a superuser can
//! edit or erase after the fact defeats its own purpose. There is no
//! rule string that means "nobody, ever, not even a superuser", so
//! [`bind_append_only_guard`] enforces it as a hook instead: a handler
//! tagged to `_audit_log` on `on_record_update`/`on_record_delete` that
//! returns an error *without* calling `e.next()` — the same "replace the
//! built-in behaviour by not advancing the chain" mechanism
//! `crates/server/src/hooks.rs` documents, just used to permanently
//! refuse rather than conditionally allow.

use serde_json::{json, Map, Value};

use cratebase_core::{AppError, Collection, Record};

use crate::app::App;
use crate::events::{CollectionRequestEvent, RecordEvent, RecordRequestEvent, SettingsUpdateEvent};
use crate::extract::Auth;
use crate::hooks::{Event, Handler};

pub const COLLECTION: &str = "_audit_log";

/// Register every audit-writing and append-only-enforcing hook. Called
/// once from [`crate::app::App::bootstrap`].
pub fn bind_hooks(app: &App) {
    bind_append_only_guard(app);
    bind_collection_lifecycle(app);
    bind_settings_update(app);
    bind_superuser_lifecycle(app);
    bind_record_delete_bypass(app);
}

/// Best-effort insert into `_audit_log`. Never fails the caller — same
/// reasoning as `crate::llm::log_usage`: a logging failure must not fail
/// the action it is auditing.
async fn write(
    app: &App,
    actor: Option<&str>,
    action: &str,
    target: impl Into<String>,
    meta: Value,
) {
    let Some(collection) = app.db().collections.get_by_name(COLLECTION) else {
        tracing::warn!("_audit_log collection missing; dropping audit record");
        return;
    };

    let mut input = Map::new();
    if let Some(actor) = actor {
        input.insert("actor".into(), Value::String(actor.to_string()));
    }
    input.insert("action".into(), Value::String(action.to_string()));
    input.insert("target".into(), Value::String(target.into()));
    input.insert("meta".into(), meta);

    let mut record = cratebase_db::records::from_body(collection.clone(), &input);
    if record.id().is_empty() {
        record.set_id(cratebase_core::record_id());
    }

    let action = action.to_string();
    let result = app
        .run_scoped(true, move |tx| {
            crate::routes::records::write_record(
                tx,
                collection,
                record,
                None,
                crate::routes::records::Write::Create,
                Vec::new(),
            )
        })
        .await;
    if let Err(e) = result {
        tracing::warn!(error = %e, action, "failed to write audit log row");
    }
}

/// `_audit_log` itself: reject every update/delete before `e.next()`
/// ever runs, superuser or not. See this module's doc for why a rule
/// string cannot express this.
fn bind_append_only_guard(app: &App) {
    app.hooks().on_record_update.bind(
        Handler::new(|_: &mut RecordEvent| {
            Box::pin(async move {
                Err(AppError::forbidden(
                    "_audit_log records cannot be modified.",
                ))
            })
        })
        .with_tags([COLLECTION]),
    );
    app.hooks().on_record_delete.bind(
        Handler::new(|_: &mut RecordEvent| {
            Box::pin(
                async move { Err(AppError::forbidden("_audit_log records cannot be deleted.")) },
            )
        })
        .with_tags([COLLECTION]),
    );
}

/// `collection.create`/`collection.update`/`collection.delete`. Bound
/// with no tags, so every collection's lifecycle is audited, not just
/// one — `Hook::trigger` only tag-filters handlers that opt into it
/// (same reasoning `crate::webhooks`'s dispatch hook doc gives for its
/// own untagged handlers).
fn bind_collection_lifecycle(app: &App) {
    app.hooks().on_collection_create_request.bind(Handler::new(
        |e: &mut CollectionRequestEvent| {
            Box::pin(async move {
                e.next().await?;
                if let Some(created) = e.collection.clone() {
                    let actor = e.auth.as_ref().map(|a| a.id.as_str());
                    write(
                        &e.app,
                        actor,
                        "collection.create",
                        created.name.clone(),
                        collection_meta(&created),
                    )
                    .await;
                }
                Ok(())
            })
        },
    ));
    app.hooks().on_collection_update_request.bind(Handler::new(
        |e: &mut CollectionRequestEvent| {
            Box::pin(async move {
                // The store has not been reloaded with the pending write
                // yet — `crate::routes::collections::apply` only does
                // that after this whole hook chain returns — so this is
                // still the collection as it stood before the update.
                let before = e
                    .collection
                    .as_ref()
                    .and_then(|c| e.app.db().collections.get_by_id(&c.id))
                    .map(|c| c.to_json());
                e.next().await?;
                if let (Some(before), Some(after)) = (before, e.collection.clone()) {
                    let actor = e.auth.as_ref().map(|a| a.id.as_str());
                    let meta = json!({ "diff": diff_json(&before, &after.to_json()) });
                    write(&e.app, actor, "collection.update", after.name.clone(), meta).await;
                }
                Ok(())
            })
        },
    ));
    app.hooks().on_collection_delete_request.bind(Handler::new(
        |e: &mut CollectionRequestEvent| {
            Box::pin(async move {
                let deleted = e.collection.clone();
                e.next().await?;
                if let Some(deleted) = deleted {
                    let actor = e.auth.as_ref().map(|a| a.id.as_str());
                    write(
                        &e.app,
                        actor,
                        "collection.delete",
                        deleted.name.clone(),
                        collection_meta(&deleted),
                    )
                    .await;
                }
                Ok(())
            })
        },
    ));
}

fn collection_meta(c: &Collection) -> Value {
    json!({ "collection": c.to_json() })
}

/// `settings.update`, with a real before/after diff in `meta` —
/// `to_public_json` on both sides so a write-only secret (`smtp.password`)
/// never lands in the log.
fn bind_settings_update(app: &App) {
    app.hooks()
        .on_settings_update_request
        .bind(Handler::new(|e: &mut SettingsUpdateEvent| {
            Box::pin(async move {
                let before = e.old_settings.to_public_json();
                let after = e.new_settings.to_public_json();
                e.next().await?;
                let diff = diff_json(&before, &after);
                if diff.as_object().is_some_and(|m| !m.is_empty()) {
                    let actor = e.request.auth.as_ref().map(|a| a.id.as_str());
                    let target = diff
                        .as_object()
                        .map(|m| m.keys().cloned().collect::<Vec<_>>().join(","))
                        .unwrap_or_default();
                    write(
                        &e.app,
                        actor,
                        "settings.update",
                        target,
                        json!({ "diff": diff }),
                    )
                    .await;
                }
                Ok(())
            })
        }));
}

/// `superuser.create`/`superuser.role_change`/`superuser.delete`: tagged
/// to `_superusers`, since these are specific to that one collection.
fn bind_superuser_lifecycle(app: &App) {
    let tag = cratebase_core::SUPERUSERS_COLLECTION;

    app.hooks().on_record_create_request.bind(
        Handler::new(|e: &mut RecordRequestEvent| {
            Box::pin(async move {
                e.next().await?;
                if let Some(created) = e.record.clone() {
                    let actor = e.auth.as_ref().map(|a| a.id.as_str());
                    let meta = json!({ "email": created.get_string("email") });
                    write(
                        &e.app,
                        actor,
                        "superuser.create",
                        superuser_target(&created),
                        meta,
                    )
                    .await;
                }
                Ok(())
            })
        })
        .with_tags([tag]),
    );

    app.hooks().on_record_update_request.bind(
        Handler::new(|e: &mut RecordRequestEvent| {
            Box::pin(async move {
                // Same reasoning as the collection-update handler above:
                // fetched before `e.next()` runs the actual write, so
                // this is genuinely the stored value, not the pending
                // patch already merged into `e.record`.
                let id = e
                    .record
                    .as_ref()
                    .map(|r| r.id().to_string())
                    .unwrap_or_default();
                let previous_role =
                    cratebase_db::records::find_by_id_raw(e.app.db(), &e.collection, &id)
                        .await
                        .ok()
                        .map(|r| r.get_string("role"));
                e.next().await?;
                if let (Some(previous_role), Some(updated)) = (previous_role, e.record.clone()) {
                    let new_role = updated.get_string("role");
                    if new_role != previous_role {
                        let actor = e.auth.as_ref().map(|a| a.id.as_str());
                        let meta = json!({ "from": previous_role, "to": new_role });
                        write(
                            &e.app,
                            actor,
                            "superuser.role_change",
                            superuser_target(&updated),
                            meta,
                        )
                        .await;
                    }
                }
                Ok(())
            })
        })
        .with_tags([tag]),
    );

    app.hooks().on_record_delete_request.bind(
        Handler::new(|e: &mut RecordRequestEvent| {
            Box::pin(async move {
                e.next().await?;
                if let Some(deleted) = e.record.clone() {
                    let actor = e.auth.as_ref().map(|a| a.id.as_str());
                    let meta = json!({ "email": deleted.get_string("email") });
                    write(
                        &e.app,
                        actor,
                        "superuser.delete",
                        superuser_target(&deleted),
                        meta,
                    )
                    .await;
                }
                Ok(())
            })
        })
        .with_tags([tag]),
    );
}

/// A superuser row identified by email when it has one, falling back to
/// its id — matches how `crates/server/src/routes/logs.rs` labels a
/// caller in `_request_logs`.
fn superuser_target(record: &Record) -> String {
    let email = record.get_string("email");
    if email.is_empty() {
        record.id().to_string()
    } else {
        email
    }
}

/// `record.delete`: untagged (applies to every collection), logged only
/// for a genuine superuser bypass — see this module's doc for the exact
/// definition. `_superusers` is excluded: that case gets the more
/// specific `superuser.delete` from [`bind_superuser_lifecycle`] instead,
/// bound on the same hook, so logging it here too would double it.
/// `_audit_log` is excluded too, though [`bind_append_only_guard`]
/// already makes that delete fail before this ever has anything to log.
fn bind_record_delete_bypass(app: &App) {
    app.hooks()
        .on_record_delete_request
        .bind(Handler::new(|e: &mut RecordRequestEvent| {
            Box::pin(async move {
                let collection_name = e.collection.name.clone();
                let bypassed = is_superuser(e.auth.as_ref());
                e.next().await?;
                if bypassed
                    && collection_name != cratebase_core::SUPERUSERS_COLLECTION
                    && collection_name != COLLECTION
                {
                    if let Some(deleted) = e.record.clone() {
                        let actor = e.auth.as_ref().map(|a| a.id.as_str());
                        let target = format!("{collection_name}/{}", deleted.id());
                        write(&e.app, actor, "record.delete", target, json!({})).await;
                    }
                }
                Ok(())
            })
        }));
}

/// The same definition `crates/server/src/routes/common.rs`'s
/// `record_matches_rule` uses: a superuser's own status is what grants
/// access, independent of whatever the collection's own rule says.
fn is_superuser(auth: Option<&Auth>) -> bool {
    auth.is_some_and(|a| a.is_superuser)
}

/// A shallow diff of two JSON objects: one entry per top-level key whose
/// value differs, each `{from, to}`. Good enough for `_audit_log.meta`
/// without a generic recursive-diff dependency — a settings or collection
/// payload is a handful of top-level sections, and which whole section
/// changed is what an operator actually wants to see first.
fn diff_json(old: &Value, new: &Value) -> Value {
    let (Value::Object(old_map), Value::Object(new_map)) = (old, new) else {
        return json!({ "from": old, "to": new });
    };
    let mut keys: Vec<&String> = old_map.keys().chain(new_map.keys()).collect();
    keys.sort();
    keys.dedup();
    let mut changed = Map::new();
    for key in keys {
        let a = old_map.get(key).unwrap_or(&Value::Null);
        let b = new_map.get(key).unwrap_or(&Value::Null);
        if a != b {
            changed.insert(key.clone(), json!({ "from": a, "to": b }));
        }
    }
    Value::Object(changed)
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::extract::{Path, Request, State};
    use cratebase_core::{CollectionType, Field, FieldKind, SerializeOptions};
    use cratebase_db::engine::Executor;
    use serde_json::json;

    use super::*;
    use crate::config::Config;
    use crate::extract::RequestInfo;
    use crate::http_error::ApiQuery;
    use crate::routes::collections;
    use crate::routes::records::{create_record, delete_record, update_record, SingleQuery};

    async fn test_app() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::new(Config::memory(dir.path()));
        app.bootstrap().await.expect("bootstrap");
        (app, dir)
    }

    fn text_field(name: &str) -> Field {
        Field::new(
            name,
            FieldKind::Text {
                min: 0,
                max: 0,
                pattern: String::new(),
                autogenerate_pattern: String::new(),
                primary_key: false,
            },
        )
    }

    /// A superuser `Auth`, backed by a real `_superusers` row —
    /// `actor` is a genuine relation field, so a row referencing an id
    /// that does not exist would fail create validation. `label` only
    /// seeds the email; the id is a real generated one (record ids have
    /// a minimum length the collection enforces).
    async fn superuser_auth(app: &App, label: &str, role: &str) -> Auth {
        let collection = app.db().collections.get_by_name("_superusers").unwrap();
        let mut record = Record::new(collection.clone());
        record.set_id(cratebase_core::record_id());
        record.set("email", Value::String(format!("{label}@example.com")));
        record.set(
            "password",
            Value::String("hashed-password-placeholder".into()),
        );
        record.set(
            "tokenKey",
            Value::String(format!(
                "tok-{}-0000000000000000",
                cratebase_core::record_id()
            )),
        );
        record.set("role", Value::String(role.to_string()));
        cratebase_db::records::create(app.db(), &app.db().collections, &mut record)
            .await
            .expect("insert superuser fixture");
        let id = record.id().to_string();
        Auth {
            id,
            collection_id: collection.id.clone(),
            collection_name: collection.name.clone(),
            is_superuser: true,
            collection,
            record,
        }
    }

    async fn audit_rows(app: &App) -> Vec<Value> {
        let collection = app.db().collections.get_by_name(COLLECTION).unwrap();
        let rows = app
            .db()
            .query(&format!(r#"SELECT * FROM "{COLLECTION}""#), &[])
            .await
            .unwrap();
        rows.into_iter()
            .map(|row| {
                cratebase_db::records::row_to_record(&collection, &row)
                    .to_json(SerializeOptions::default())
            })
            .collect()
    }

    #[tokio::test]
    async fn collection_create_update_delete_each_log_one_row() {
        let (app, _dir) = test_app().await;
        let info = RequestInfo::default();
        let actor = superuser_auth(&app, "su1", "owner").await;
        let actor_id = actor.id.clone();

        let mut next = Collection::new("widgets", CollectionType::Base);
        next.fields = vec![text_field("title")];
        collections::prepare_new(&mut next);
        collections::validate(&app, &next, None, "test").unwrap();
        let created = collections::apply(
            &app,
            next,
            None,
            collections::Change::Create,
            &info,
            Some(actor.clone()),
        )
        .await
        .unwrap();

        let mut updated = (*created).clone();
        updated.fields.push(text_field("subtitle"));
        collections::validate(&app, &updated, Some(&created), "test").unwrap();
        collections::apply(
            &app,
            updated,
            Some((*created).clone()),
            collections::Change::Update,
            &info,
            Some(actor.clone()),
        )
        .await
        .unwrap();

        let existing = app.db().collections.get_by_id(&created.id).unwrap();
        collections::apply(
            &app,
            (*existing).clone(),
            Some((*existing).clone()),
            collections::Change::Delete,
            &info,
            Some(actor),
        )
        .await
        .unwrap();

        let rows = audit_rows(&app).await;
        let creates: Vec<&Value> = rows
            .iter()
            .filter(|r| r["action"] == "collection.create")
            .collect();
        let updates: Vec<&Value> = rows
            .iter()
            .filter(|r| r["action"] == "collection.update")
            .collect();
        let deletes: Vec<&Value> = rows
            .iter()
            .filter(|r| r["action"] == "collection.delete")
            .collect();
        assert_eq!(creates.len(), 1, "{rows:#?}");
        assert_eq!(updates.len(), 1, "{rows:#?}");
        assert_eq!(deletes.len(), 1, "{rows:#?}");
        assert_eq!(creates[0]["target"], "widgets");
        assert_eq!(creates[0]["actor"], actor_id);
        assert!(creates[0]["meta"]["collection"]["name"] == "widgets");
        assert!(
            updates[0]["meta"]["diff"]["fields"].is_object(),
            "expected a real diff for the added field: {:#?}",
            updates[0]
        );
    }

    #[tokio::test]
    async fn settings_update_logs_one_row_with_a_real_diff() {
        let (app, _dir) = test_app().await;
        let actor = superuser_auth(&app, "su1", "owner").await;
        let actor_id = actor.id.clone();
        let request = RequestInfo {
            auth: Some(actor),
            ..Default::default()
        };

        let old = app.settings();
        let mut merged = (*old).clone();
        merged.meta.app_name = "Renamed App".to_string();
        let mut event = SettingsUpdateEvent::new(app.clone(), request, old, merged);
        let app_for_finalizer = app.clone();
        app.hooks()
            .on_settings_update_request
            .trigger(&mut event, move |e| {
                let app = app_for_finalizer.clone();
                let next = e.new_settings.clone();
                Box::pin(async move {
                    app.set_settings(next)
                        .await
                        .map_err(|err| AppError::internal(err.to_string()))
                })
            })
            .await
            .unwrap();

        let rows = audit_rows(&app).await;
        let updates: Vec<&Value> = rows
            .iter()
            .filter(|r| r["action"] == "settings.update")
            .collect();
        assert_eq!(updates.len(), 1, "{rows:#?}");
        assert_eq!(updates[0]["actor"], actor_id);
        assert_eq!(
            updates[0]["meta"]["diff"]["meta"]["to"]["appName"],
            "Renamed App"
        );
    }

    #[tokio::test]
    async fn superuser_create_and_role_change_and_delete_each_log_one_row() {
        let (app, _dir) = test_app().await;
        let owner = superuser_auth(&app, "owner1", "owner").await;

        // Create a second superuser.
        let info = RequestInfo {
            auth: Some(owner.clone()),
            ..Default::default()
        };
        let body = json!({
            "email": "new-admin@example.com",
            "password": "supersecretpw",
            "passwordConfirm": "supersecretpw",
            "role": "admin",
        });
        let request = Request::new(Body::from(serde_json::to_vec(&body).unwrap()));
        let response = create_record(
            State(app.clone()),
            Path("_superusers".to_string()),
            ApiQuery(SingleQuery::default()),
            info.clone(),
            request,
        )
        .await
        .expect("create superuser");
        let new_id = response.0["id"].as_str().unwrap().to_string();

        // Change its role.
        let patch = json!({ "role": "owner" });
        let request = Request::new(Body::from(serde_json::to_vec(&patch).unwrap()));
        let _ = update_record(
            State(app.clone()),
            Path(("_superusers".to_string(), new_id.clone())),
            ApiQuery(SingleQuery::default()),
            info.clone(),
            request,
        )
        .await
        .expect("promote superuser");

        // Delete it.
        delete_record(
            State(app.clone()),
            Path(("_superusers".to_string(), new_id.clone())),
            info,
        )
        .await
        .expect("delete superuser");

        let rows = audit_rows(&app).await;
        let creates: Vec<&Value> = rows
            .iter()
            .filter(|r| r["action"] == "superuser.create")
            .collect();
        let role_changes: Vec<&Value> = rows
            .iter()
            .filter(|r| r["action"] == "superuser.role_change")
            .collect();
        let deletes: Vec<&Value> = rows
            .iter()
            .filter(|r| r["action"] == "superuser.delete")
            .collect();
        assert_eq!(creates.len(), 1, "{rows:#?}");
        assert_eq!(role_changes.len(), 1, "{rows:#?}");
        assert_eq!(deletes.len(), 1, "{rows:#?}");
        assert_eq!(creates[0]["target"], "new-admin@example.com");
        assert_eq!(role_changes[0]["meta"]["from"], "admin");
        assert_eq!(role_changes[0]["meta"]["to"], "owner");
    }

    #[tokio::test]
    async fn superuser_bypass_delete_logs_record_delete_but_ordinary_delete_does_not() {
        let (app, _dir) = test_app().await;
        let owner = superuser_auth(&app, "owner1", "owner").await;
        let owner_id = owner.id.clone();

        // A collection only its own author may delete.
        let mut next = Collection::new("notes", CollectionType::Base);
        next.fields = vec![text_field("authorId"), text_field("body")];
        next.delete_rule = Some("authorId = @request.auth.id".to_string());
        collections::prepare_new(&mut next);
        collections::validate(&app, &next, None, "test").unwrap();
        collections::apply(
            &app,
            next,
            None,
            collections::Change::Create,
            &RequestInfo::default(),
            None,
        )
        .await
        .unwrap();

        // An ordinary `users` record that owns one of the two rows.
        let users = app.db().collections.get_by_name("users").unwrap();
        let mut author = Record::new(users.clone());
        author.set_id(cratebase_core::record_id());
        author.set("email", Value::String("author@example.com".into()));
        author.set(
            "password",
            Value::String("hashed-password-placeholder".into()),
        );
        author.set(
            "tokenKey",
            Value::String(format!(
                "tok-{}-0000000000000000",
                cratebase_core::record_id()
            )),
        );
        cratebase_db::records::create(app.db(), &app.db().collections, &mut author)
            .await
            .unwrap();
        let author_id = author.id().to_string();
        let author_auth = Auth {
            id: author_id.clone(),
            collection_id: users.id.clone(),
            collection_name: users.name.clone(),
            is_superuser: false,
            collection: users,
            record: author,
        };

        let notes = app.db().collections.get_by_name("notes").unwrap();
        let mut owned_note = Record::new(notes.clone());
        owned_note.set("authorId", Value::String(author_id));
        owned_note.set("body", Value::String("mine".into()));
        cratebase_db::records::create(app.db(), &app.db().collections, &mut owned_note)
            .await
            .unwrap();
        let mut other_note = Record::new(notes.clone());
        other_note.set("authorId", Value::String("someone-else".into()));
        other_note.set("body", Value::String("not mine".into()));
        cratebase_db::records::create(app.db(), &app.db().collections, &mut other_note)
            .await
            .unwrap();

        // The author deleting their own row: ordinary, rule-permitted —
        // must NOT produce a `record.delete` audit row.
        let author_info = RequestInfo {
            auth: Some(author_auth),
            ..Default::default()
        };
        delete_record(
            State(app.clone()),
            Path(("notes".to_string(), owned_note.id().to_string())),
            author_info,
        )
        .await
        .expect("author deletes their own note");

        // A superuser deleting someone else's row: only a superuser
        // bypass gets it past `deleteRule` — must produce exactly one.
        let superuser_info = RequestInfo {
            auth: Some(owner),
            ..Default::default()
        };
        delete_record(
            State(app.clone()),
            Path(("notes".to_string(), other_note.id().to_string())),
            superuser_info,
        )
        .await
        .expect("superuser bypasses the rule");

        let rows = audit_rows(&app).await;
        let record_deletes: Vec<&Value> = rows
            .iter()
            .filter(|r| r["action"] == "record.delete")
            .collect();
        assert_eq!(record_deletes.len(), 1, "{rows:#?}");
        assert_eq!(
            record_deletes[0]["target"],
            format!("notes/{}", other_note.id())
        );
        assert_eq!(record_deletes[0]["actor"], owner_id);
    }

    #[tokio::test]
    async fn audit_log_rejects_update_and_delete_even_from_a_superuser() {
        let (app, _dir) = test_app().await;
        let owner = superuser_auth(&app, "owner1", "owner").await;

        // Produce one real row via a collection create, so there is
        // something to try to tamper with.
        let mut next = Collection::new("widgets", CollectionType::Base);
        next.fields = vec![text_field("title")];
        collections::prepare_new(&mut next);
        collections::validate(&app, &next, None, "test").unwrap();
        collections::apply(
            &app,
            next,
            None,
            collections::Change::Create,
            &RequestInfo::default(),
            Some(owner.clone()),
        )
        .await
        .unwrap();

        let row = audit_rows(&app)
            .await
            .into_iter()
            .next()
            .expect("one audit row");
        let id = row["id"].as_str().unwrap().to_string();

        let info = RequestInfo {
            auth: Some(owner),
            ..Default::default()
        };

        let patch = json!({ "target": "tampered" });
        let request = Request::new(Body::from(serde_json::to_vec(&patch).unwrap()));
        let update_result = update_record(
            State(app.clone()),
            Path((COLLECTION.to_string(), id.clone())),
            ApiQuery(SingleQuery::default()),
            info.clone(),
            request,
        )
        .await;
        assert!(update_result.is_err(), "update must be rejected");

        let delete_result =
            delete_record(State(app.clone()), Path((COLLECTION.to_string(), id)), info).await;
        assert!(delete_result.is_err(), "delete must be rejected");
    }
}
