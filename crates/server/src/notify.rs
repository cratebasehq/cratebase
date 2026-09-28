//! `$notify.send` (JS hooks) and `POST /api/notifications/send`'s shared
//! pipeline: fan a notification out to one or more recipients across up
//! to three channels.
//!
//! * **`inapp`** (always available, on by default) — one `_notifications`
//!   row per recipient, created through `cratebase_db::records::create`
//!   directly rather than the HTTP record route, which means this module
//!   has to do what that route would otherwise have done for us:
//!   announce the write over realtime itself (see [`create_inapp`]'s call
//!   to `crate::realtime::publish`). Since `_notifications` is an
//!   ordinary rule-gated system collection, a client's existing realtime
//!   subscription to it (or to `_notifications/{id}`) "just works" — no
//!   `_notifications`-specific wire protocol exists.
//! * **`email`** — delivered through `crate::mails`' full pipeline (the
//!   same one `POST /api/mails/send` uses), rendering the `notification`
//!   `_emailTemplates` seed row (`{ title, body, link }`) unless the
//!   recipient has no `email` value, in which case this channel is
//!   silently skipped for that recipient.
//! * **`push`** — delivered through `crate::push::PushService` to every
//!   `_push_subscriptions` row belonging to the recipient, reusing that
//!   module's own delivery/dead-token-cleanup logic
//!   (`crate::push::deliver_and_cleanup`) rather than a second copy.
//!
//! # Deferred past commit, like `$mails.send`
//!
//! [`prepare`]/[`finish`] split the work the same way
//! `crate::mails::prepare`/`crate::mails::finish` do, for the same reason
//! (see that module's doc comment): a JS hook running inside a still-open
//! record-write transaction cannot safely do the writes/deliveries in
//! [`finish`] inline — on SQLite that would self-deadlock against the
//! writer-lock the enclosing transaction already holds, and on any
//! backend a notification must never go out for a write that ends up
//! rolling back. [`prepare`] only validates and resolves the recipient
//! rows (reads, harmless under an open transaction); `crate::jsvm_host`
//! defers [`finish`] to `TxApp::after_commit` when called from a hook,
//! exactly like `mails_send`.

use std::sync::Arc;

use serde::Serialize;
use serde_json::{json, Value};

use cratebase_core::{AppError, Collection, Record, RecordAction};
use cratebase_db::engine::{Executor, Sql};
use cratebase_db::records;

use crate::app::App;
use crate::mails::{self, Recipient, SendInput};
use crate::push::{self, PushPayload, PushService};

pub const NOTIFICATIONS_COLLECTION: &str = cratebase_core::NOTIFICATIONS_COLLECTION;
/// `_emailTemplates.key` the email channel renders through — seeded by
/// migration 20 (`crates/db/src/migrations.rs`'s
/// `add_notifications_and_channels_up`).
pub const EMAIL_TEMPLATE_KEY: &str = "notification";

pub const CHANNEL_INAPP: &str = "inapp";
pub const CHANNEL_EMAIL: &str = "email";
pub const CHANNEL_PUSH: &str = "push";
const ALL_CHANNELS: &[&str] = &[CHANNEL_INAPP, CHANNEL_EMAIL, CHANNEL_PUSH];

/// A single send may not fan out past this many recipients — generous
/// enough for "notify everyone on this team", not so generous that one
/// call can iterate an entire `users` table.
pub const MAX_RECIPIENTS: usize = 500;

/// `$notify.send`/`POST /api/notifications/send`'s normalized input.
#[derive(Debug, Clone, Default)]
pub struct NotifySendInput {
    /// Recipient record ids, in `collection`.
    pub to: Vec<String>,
    /// The recipient auth collection; defaults to `"users"`.
    pub collection: Option<String>,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub data: Value,
    pub link: Option<String>,
    /// `None` means every channel ([`ALL_CHANNELS`]).
    pub channels: Option<Vec<String>>,
}

fn validate(input: &NotifySendInput) -> Result<(), AppError> {
    if input.to.is_empty() {
        return Err(AppError::bad_request(
            "to must include at least one recipient id.",
        ));
    }
    if input.to.len() > MAX_RECIPIENTS {
        return Err(AppError::bad_request(format!(
            "to may not exceed {MAX_RECIPIENTS} recipients."
        )));
    }
    if input.kind.trim().is_empty() {
        return Err(AppError::bad_request("type is required."));
    }
    if input.title.trim().is_empty() {
        return Err(AppError::bad_request("title is required."));
    }
    if input.body.trim().is_empty() {
        return Err(AppError::bad_request("body is required."));
    }
    if let Some(channels) = &input.channels {
        if channels.is_empty() {
            return Err(AppError::bad_request(
                "channels, when given, must not be empty.",
            ));
        }
        for c in channels {
            if !ALL_CHANNELS.contains(&c.as_str()) {
                return Err(AppError::bad_request(format!(
                    "unknown channel '{c}'; expected one of {ALL_CHANNELS:?}."
                )));
            }
        }
    }
    Ok(())
}

/// Validated input plus the recipient rows resolved while they were still
/// safe to read (see the module doc's "Deferred past commit" section). A
/// recipient id that doesn't resolve to a real row is dropped here, with
/// a warning — never an error: a caller batch-notifying ids from an
/// external system shouldn't have the whole call fail because one of them
/// was deleted since.
pub struct PreparedNotify {
    collection: Arc<Collection>,
    recipients: Vec<Record>,
    input: NotifySendInput,
}

impl PreparedNotify {
    /// The recipient ids that actually resolved to a row — what
    /// `crate::jsvm_host`'s deferred `$notify.send` reports back to the
    /// hook synchronously, before [`finish`] (channel delivery) has even
    /// run, same "the request was accepted" contract `mails_send` gives a
    /// hook for its own deferred `_mailLog` id.
    pub fn recipient_ids(&self) -> Vec<String> {
        self.recipients.iter().map(|r| r.id().to_string()).collect()
    }
}

/// `$notify.send`'s result: which recipients actually resolved to a row
/// (every channel is delivered best-effort per recipient after that, so
/// this doesn't distinguish "email bounced" from "email skipped" —
/// `_mailLog`/tracing carry that detail instead, same as any other
/// `crate::mails::send` caller).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendOutcome {
    pub sent: usize,
    pub recipients: Vec<String>,
}

pub async fn prepare(app: &App, input: NotifySendInput) -> Result<PreparedNotify, AppError> {
    prepare_with(app, app.db(), input).await
}

/// [`prepare`], reading the recipient rows through `ex` instead of the
/// plain pool. `crate::jsvm_host` passes the hook's own open transaction
/// here: on SQLite a read through a *different* connection while that
/// transaction holds the writer lock can block until it commits — which
/// it never will, since the commit is waiting on this very hook.
pub async fn prepare_with(
    app: &App,
    ex: &dyn Executor,
    input: NotifySendInput,
) -> Result<PreparedNotify, AppError> {
    validate(&input)?;
    let collection_name = input.collection.clone().unwrap_or_else(|| "users".into());
    let collection = app
        .db()
        .collections
        .get_by_name(&collection_name)
        .ok_or_else(|| AppError::bad_request(format!("no such collection '{collection_name}'.")))?;
    if !collection.is_auth() {
        return Err(AppError::bad_request(format!(
            "'{collection_name}' is not an auth collection."
        )));
    }

    let mut recipients = Vec::with_capacity(input.to.len());
    for id in &input.to {
        match records::find_by_id_raw(ex, &collection, id).await {
            Ok(record) => recipients.push(record),
            Err(_) => {
                tracing::warn!(id = %id, collection = %collection_name, "$notify.send: recipient not found; skipping");
            }
        }
    }

    Ok(PreparedNotify {
        collection,
        recipients,
        input,
    })
}

/// Creates every channel's deliveries for an already-[`prepare`]d
/// notification. See the module doc for what each channel does and the
/// "Deferred past commit" section for why this is a separate step from
/// [`prepare`].
pub async fn finish(app: &App, prepared: PreparedNotify) -> SendOutcome {
    let PreparedNotify {
        collection,
        recipients,
        input,
    } = prepared;
    let channels: Vec<&str> = input
        .channels
        .as_ref()
        .map(|v| v.iter().map(String::as_str).collect())
        .unwrap_or_else(|| ALL_CHANNELS.to_vec());

    let mut sent = Vec::with_capacity(recipients.len());
    for recipient in &recipients {
        sent.push(recipient.id().to_string());
        if channels.contains(&CHANNEL_INAPP) {
            if let Err(e) = create_inapp(app, &collection, recipient.id(), &input).await {
                tracing::warn!(error = %e, recipient = %recipient.id(), "notify: inapp channel failed");
            }
        }
        if channels.contains(&CHANNEL_EMAIL) {
            deliver_email(app, recipient, &input).await;
        }
        if channels.contains(&CHANNEL_PUSH) {
            deliver_push(app, &collection, recipient.id(), &input).await;
        }
    }
    SendOutcome {
        sent: sent.len(),
        recipients: sent,
    }
}

/// [`prepare`] then [`finish`], for a caller with no enclosing
/// transaction to defer past (the HTTP route, or `$notify.send` outside a
/// record-write hook).
pub async fn send(app: &App, input: NotifySendInput) -> Result<SendOutcome, AppError> {
    let prepared = prepare(app, input).await?;
    Ok(finish(app, prepared).await)
}

/// The `inapp` channel: one `_notifications` row, announced over realtime
/// exactly as if the ordinary record-create route had written it (see the
/// module doc for why this call is needed here at all).
async fn create_inapp(
    app: &App,
    recipient_collection: &Arc<Collection>,
    recipient_id: &str,
    input: &NotifySendInput,
) -> Result<(), AppError> {
    let Some(notifications) = app.db().collections.get_by_name(NOTIFICATIONS_COLLECTION) else {
        return Err(AppError::internal("_notifications collection is missing."));
    };
    let mut record = Record::new(notifications.clone());
    record.set(
        "collectionRef",
        Value::String(recipient_collection.id.clone()),
    );
    record.set("recordRef", Value::String(recipient_id.to_string()));
    record.set("type", Value::String(input.kind.clone()));
    record.set("title", Value::String(input.title.clone()));
    record.set("body", Value::String(input.body.clone()));
    record.set("data", input.data.clone());
    record.set(
        "link",
        Value::String(input.link.clone().unwrap_or_default()),
    );
    records::create(app.db(), &app.db().collections, &mut record).await?;
    crate::realtime::publish(app, &notifications, RecordAction::Create, &record);
    Ok(())
}

/// The `email` channel: silently skipped for a recipient with no `email`
/// value (an auth collection with `passwordAuth`/OAuth2-only sign-in and
/// no verified address, say) — this is a best-effort extra channel, not
/// the delivery of record, so a missing address is not an error.
async fn deliver_email(app: &App, recipient: &Record, input: &NotifySendInput) {
    let Some(email) = recipient
        .get("email")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    else {
        return;
    };
    let send_input = SendInput {
        to: vec![Recipient {
            address: email.to_string(),
            name: String::new(),
        }],
        template: Some(EMAIL_TEMPLATE_KEY.to_string()),
        data: json!({
            "title": input.title,
            "body": input.body,
            "link": input.link.clone().unwrap_or_default(),
        }),
        ..Default::default()
    };
    if let Err(e) = mails::send(app, send_input).await {
        tracing::warn!(error = %e, "notify: email channel failed");
    }
}

/// The `push` channel: every enabled `_push_subscriptions` row belonging
/// to this recipient, delivered and cleaned up exactly like a
/// `settings.push.triggers` dispatch (`crate::push::deliver_and_cleanup`).
async fn deliver_push(
    app: &App,
    recipient_collection: &Arc<Collection>,
    recipient_id: &str,
    input: &NotifySendInput,
) {
    let rows = match app
        .db()
        .query(
            r#"SELECT "id", "platform", "token" FROM "_push_subscriptions"
               WHERE "enabled" = 1 AND "collectionRef" = $1 AND "recordRef" = $2"#,
            &[
                Sql::from(recipient_collection.id.clone()),
                Sql::from(recipient_id.to_string()),
            ],
        )
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "notify: push subscription lookup failed");
            return;
        }
    };
    if rows.is_empty() {
        return;
    }

    let service = PushService::from_settings(&app.settings().push);
    let payload = PushPayload {
        title: input.title.clone(),
        body: input.body.clone(),
        data: json!({
            "type": input.kind,
            "link": input.link.clone().unwrap_or_default(),
            "data": input.data,
        }),
    };
    for row in &rows {
        let (Some(id), Some(platform), Some(token)) = (
            row.get_str("id"),
            row.get_str("platform"),
            row.get_str("token"),
        ) else {
            continue;
        };
        tokio::spawn(push::deliver_and_cleanup(
            app.clone(),
            service.clone(),
            id.to_string(),
            platform.to_string(),
            token.to_string(),
            payload.clone(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_requires_recipients_type_title_and_body() {
        let base = NotifySendInput {
            to: vec!["u1".into()],
            kind: "info".into(),
            title: "hi".into(),
            body: "there".into(),
            ..Default::default()
        };
        assert!(validate(&base).is_ok());

        let mut no_to = base.clone();
        no_to.to = vec![];
        assert!(validate(&no_to).is_err());

        let mut no_type = base.clone();
        no_type.kind = String::new();
        assert!(validate(&no_type).is_err());

        let mut no_title = base.clone();
        no_title.title = String::new();
        assert!(validate(&no_title).is_err());

        let mut no_body = base.clone();
        no_body.body = String::new();
        assert!(validate(&no_body).is_err());
    }

    #[test]
    fn validate_rejects_unknown_channels_and_too_many_recipients() {
        let mut input = NotifySendInput {
            to: vec!["u1".into()],
            kind: "info".into(),
            title: "hi".into(),
            body: "there".into(),
            channels: Some(vec!["sms".into()]),
            ..Default::default()
        };
        assert!(validate(&input).is_err());

        input.channels = Some(vec!["email".into()]);
        assert!(validate(&input).is_ok());

        input.channels = None;
        input.to = (0..MAX_RECIPIENTS + 1).map(|i| i.to_string()).collect();
        assert!(validate(&input).is_err());
    }
}
