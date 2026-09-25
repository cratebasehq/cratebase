//! The send pipeline shared by `POST /api/mails/send`
//! (`crate::routes::mails`) and the JS host binding `$mails.send`
//! (`crate::jsvm_host`): resolve a `template` key (through
//! `crate::mail_templates`) or take raw `subject`/`html`/`text`, log the
//! attempt to `_mailLog`, and deliver either inline (with one retry) or
//! through the durable queue (`crate::queue`) when `settings.queue.enabled`.
//!
//! # Delivery and `_mailLog` status
//!
//! A `_mailLog` row is written *before* delivery is attempted
//! (`status: "queued"`) so a send that crashes the process mid-flight
//! still leaves a trace, then updated in place once an attempt resolves.
//! `status` always reflects the *most recent* attempt:
//!
//! * Inline (`settings.queue.enabled == false`): one attempt, then (on
//!   failure) one immediate retry; the row ends at `"sent"` or
//!   `"failed"`.
//! * Queued (`settings.queue.enabled == true`): the row starts and stays
//!   at `"queued"` until the worker's first attempt resolves it to
//!   `"sent"` or `"failed"` — and, if it lands on `"failed"` after a
//!   transient error, a later retry (`crate::queue`'s own exponential
//!   backoff) can still flip it back to `"sent"`. This is a deliberate
//!   simplification over tracking `_queue_jobs.attempts`/`maxAttempts`
//!   in lockstep: the row is always an accurate snapshot of "what
//!   happened last", just not a guarantee that no more retries remain.
//!
//! Either way, `POST /api/mails/send` itself always answers `200` once
//! validation passes — a delivery failure is reported in the response
//! body (`status: "failed"`, `error`), not as an HTTP error, the same
//! "the request to send was accepted" contract `request-otp`/
//! `request-magic-link` give a mail send that might silently no-op.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use cratebase_core::{AppError, DateTime, Record};
use cratebase_db::records;
use cratebase_mailer::{Message, TemplateDoc};

use crate::app::App;
use crate::mail_templates::{find_email_template, resolve_locale};

pub const MAIL_LOG_COLLECTION: &str = "_mailLog";
pub const QUEUE_NAME: &str = "mail:send";
pub const QUEUE_MAX_ATTEMPTS: i64 = 5;

pub const STATUS_QUEUED: &str = "queued";
pub const STATUS_SENT: &str = "sent";
pub const STATUS_FAILED: &str = "failed";

/// `to`/`cc`/`bcc` together may not exceed this many recipients.
pub const MAX_RECIPIENTS: usize = 50;

/// One recipient: a bare address, or `{address, name}` — see
/// [`parse_recipients`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Recipient {
    pub address: String,
    #[serde(default)]
    pub name: String,
}

impl Recipient {
    fn as_address(&self) -> cratebase_mailer::Address {
        (self.address.clone(), self.name.clone())
    }
}

/// Parses a `to`/`cc`/`bcc` JSON value — a bare email string, an
/// `{address, name}` object, or an array of either — into a flat list of
/// [`Recipient`]s.
pub fn parse_recipients(value: &Value) -> Result<Vec<Recipient>, String> {
    match value {
        Value::Null => Ok(vec![]),
        Value::String(s) => Ok(vec![Recipient {
            address: s.clone(),
            name: String::new(),
        }]),
        Value::Object(_) => serde_json::from_value(value.clone())
            .map(|r: Recipient| vec![r])
            .map_err(|_| INVALID_RECIPIENT_SHAPE.to_string()),
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.extend(parse_recipients(item)?);
            }
            Ok(out)
        }
        _ => Err(INVALID_RECIPIENT_SHAPE.to_string()),
    }
}

const INVALID_RECIPIENT_SHAPE: &str =
    "must be an email address, {address, name}, or an array of either.";

/// Same rule `routes::auth`/`routes::settings` use, duplicated rather
/// than shared (three lines).
fn is_email(raw: &str) -> bool {
    let raw = raw.trim();
    match raw.split_once('@') {
        Some((local, domain)) => {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && !raw.contains(' ')
        }
        None => false,
    }
}

/// `POST /api/mails/send`/`$mails.send`'s normalized input, already
/// parsed and validated by the caller's own body shape (JSON for the
/// route, the JS host binding's own argument marshalling) into this
/// common shape.
#[derive(Debug, Clone, Default)]
pub struct SendInput {
    pub to: Vec<Recipient>,
    pub cc: Vec<Recipient>,
    pub bcc: Vec<Recipient>,
    pub template: Option<String>,
    pub locale: Option<String>,
    pub data: Value,
    pub subject: Option<String>,
    pub html: Option<String>,
    pub text: Option<String>,
    pub from: Option<Recipient>,
    pub reply_to: Option<String>,
}

/// What `send`/`preview` render before delivery.
struct Rendered {
    subject: String,
    html: String,
    text: Option<String>,
}

/// Resolves `input`'s `template` (or raw `subject`/`html`/`text`) into a
/// `(subject, html, text)`. Shared by [`send`] and [`preview`].
async fn render(app: &App, input: &SendInput) -> Result<Rendered, AppError> {
    match &input.template {
        Some(key) => {
            let locale = resolve_locale(input.locale.as_deref(), None);
            let row = find_email_template(app, key, &locale)
                .await
                .ok_or_else(|| {
                    AppError::bad_request(format!("no _emailTemplates row for key '{key}'."))
                })?;
            let doc = TemplateDoc {
                subject: &row.subject,
                html: &row.html,
                text: &row.text,
                layout: row.layout,
            };
            let (subject, html, text) =
                cratebase_mailer::render_email_template(&doc, &input.data, &app.settings().meta);
            Ok(Rendered {
                subject,
                html,
                text: Some(text),
            })
        }
        None => Ok(Rendered {
            subject: input.subject.clone().unwrap_or_default(),
            html: input.html.clone().unwrap_or_default(),
            text: input.text.clone(),
        }),
    }
}

/// Validates `input` well enough to attempt a send/preview: at least one
/// `to` recipient (preview tolerates zero — see [`preview`]), the
/// combined recipient cap, every address well-formed, and either
/// `template` or `subject`+(`html`|`text`).
fn validate(input: &SendInput, require_recipients: bool) -> Result<(), AppError> {
    if require_recipients && input.to.is_empty() {
        return Err(AppError::bad_request(
            "to must include at least one recipient.",
        ));
    }
    let total = input.to.len() + input.cc.len() + input.bcc.len();
    if total > MAX_RECIPIENTS {
        return Err(AppError::bad_request(format!(
            "to/cc/bcc together may not exceed {MAX_RECIPIENTS} recipients."
        )));
    }
    for r in input.to.iter().chain(&input.cc).chain(&input.bcc) {
        if !is_email(&r.address) {
            return Err(AppError::bad_request(format!(
                "'{}' is not a valid email address.",
                r.address
            )));
        }
    }
    if input.template.is_none()
        && input.subject.is_none()
        && input.html.is_none()
        && input.text.is_none()
    {
        return Err(AppError::bad_request(
            "either template, or subject and html/text, must be given.",
        ));
    }
    Ok(())
}

/// `POST /api/mails/preview`'s result: the rendered content, no delivery.
pub struct PreviewOutcome {
    pub subject: String,
    pub html: String,
    pub text: Option<String>,
}

/// Renders `input` without sending or logging anything.
pub async fn preview(app: &App, input: &SendInput) -> Result<PreviewOutcome, AppError> {
    validate(input, false)?;
    let rendered = render(app, input).await?;
    Ok(PreviewOutcome {
        subject: rendered.subject,
        html: rendered.html,
        text: rendered.text,
    })
}

/// `POST /api/mails/send`'s result.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendOutcome {
    pub id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Validates, renders, logs and delivers `input` — see the module doc for
/// the delivery/log-status contract.
pub async fn send(app: &App, input: SendInput) -> Result<SendOutcome, AppError> {
    validate(&input, true)?;
    let rendered = render(app, &input).await?;

    let from = input
        .from
        .as_ref()
        .map(Recipient::as_address)
        .unwrap_or_else(|| app.mailer().sender().clone());
    let mut message = Message {
        to: input.to.iter().map(Recipient::as_address).collect(),
        from,
        subject: rendered.subject,
        html: rendered.html,
        text: rendered.text,
        headers: vec![],
        cc: input.cc.iter().map(Recipient::as_address).collect(),
        bcc: input.bcc.iter().map(Recipient::as_address).collect(),
    };
    if let Some(reply_to) = input.reply_to.as_ref().filter(|r| !r.is_empty()) {
        message.headers.push(("Reply-To".into(), reply_to.clone()));
    }

    let log_id = create_log_row(app, &message, input.template.as_deref()).await?;

    if app.settings().queue.enabled {
        let payload = json!({ "logId": log_id, "message": message });
        crate::queue::enqueue_job(
            app,
            QUEUE_NAME,
            payload,
            QUEUE_MAX_ATTEMPTS,
            DateTime::now(),
        )
        .await?;
        return Ok(SendOutcome {
            id: log_id,
            status: STATUS_QUEUED.to_string(),
            error: None,
        });
    }

    // Inline delivery: one attempt, then one immediate retry on failure.
    let outcome = match crate::routes::auth::deliver_mail(app, message.clone()).await {
        Ok(()) => (STATUS_SENT, None),
        Err(_) => match crate::routes::auth::deliver_mail(app, message).await {
            Ok(()) => (STATUS_SENT, None),
            Err(e) => (STATUS_FAILED, Some(e.to_string())),
        },
    };
    update_log_status(app, &log_id, outcome.0, outcome.1.as_deref()).await;
    Ok(SendOutcome {
        id: log_id,
        status: outcome.0.to_string(),
        error: outcome.1,
    })
}

/// Inserts a `_mailLog` row, `status: "queued"`, before any delivery is
/// attempted. Returns the new row's id.
pub(crate) async fn create_log_row(
    app: &App,
    message: &Message,
    template: Option<&str>,
) -> Result<String, AppError> {
    let Some(collection) = app.db().collections.get_by_name(MAIL_LOG_COLLECTION) else {
        return Err(AppError::internal("_mailLog collection is missing."));
    };
    let to: Vec<Value> = message
        .to
        .iter()
        .map(|(address, name)| json!({ "address": address, "name": name }))
        .collect();
    let mut record = Record::new(collection);
    record.set("to", Value::Array(to));
    record.set("subject", Value::String(message.subject.clone()));
    record.set(
        "template",
        Value::String(template.unwrap_or_default().to_string()),
    );
    record.set("status", Value::String(STATUS_QUEUED.to_string()));
    records::create(app.db(), &app.db().collections, &mut record).await?;
    Ok(record.id().to_string())
}

/// Updates a `_mailLog` row's `status`/`error` after a delivery attempt
/// resolves. Best-effort: a failure to write the log never fails the
/// caller's send — the mail either went out or didn't regardless of
/// whether this bookkeeping succeeds.
pub(crate) async fn update_log_status(app: &App, log_id: &str, status: &str, error: Option<&str>) {
    let Some(collection) = app.db().collections.get_by_name(MAIL_LOG_COLLECTION) else {
        return;
    };
    let Ok(mut record) = records::find_by_id_raw(app.db(), &collection, log_id).await else {
        return;
    };
    record.set("status", Value::String(status.to_string()));
    record.set(
        "error",
        Value::String(error.unwrap_or_default().to_string()),
    );
    if let Err(e) = records::update(app.db(), &app.db().collections, &mut record).await {
        tracing::warn!(error = %e, log_id, "failed to update _mailLog row");
    }
}

/// The `mail:send` queue job payload: a fully-rendered [`Message`] plus
/// the `_mailLog` row id to update once this attempt resolves.
#[derive(Debug, Deserialize)]
struct QueuedMail {
    log_id: String,
    message: Message,
}

/// Registers the `mail:send` handler on `handle` — call this before
/// `App::bootstrap` registers the [`crate::queue::QueuePlugin`] itself,
/// same as any other queue consumer. A no-op unless `settings.queue.enabled`
/// (nothing enqueues under that name otherwise).
pub fn register_queue_handler(app: &App, handle: crate::queue::QueueHandle) {
    let app = app.clone();
    handle.register_handler(QUEUE_NAME, move |payload: Value| {
        let app = app.clone();
        async move {
            let queued: QueuedMail = serde_json::from_value(payload)
                .map_err(|e| format!("invalid mail:send payload: {e}"))?;
            match crate::routes::auth::deliver_mail(&app, queued.message).await {
                Ok(()) => {
                    update_log_status(&app, &queued.log_id, STATUS_SENT, None).await;
                    Ok(())
                }
                Err(e) => {
                    let msg = e.to_string();
                    update_log_status(&app, &queued.log_id, STATUS_FAILED, Some(&msg)).await;
                    Err(msg)
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_recipients_accepts_string_object_and_array() {
        assert_eq!(
            parse_recipients(&json!("a@example.com")).unwrap(),
            vec![Recipient {
                address: "a@example.com".into(),
                name: String::new()
            }]
        );
        assert_eq!(
            parse_recipients(&json!({ "address": "a@example.com", "name": "A" })).unwrap(),
            vec![Recipient {
                address: "a@example.com".into(),
                name: "A".into()
            }]
        );
        assert_eq!(
            parse_recipients(&json!(["a@example.com", { "address": "b@example.com" }]))
                .unwrap()
                .len(),
            2
        );
        assert_eq!(parse_recipients(&Value::Null).unwrap(), vec![]);
    }

    #[test]
    fn parse_recipients_rejects_bad_shapes() {
        assert!(parse_recipients(&json!(42)).is_err());
        assert!(parse_recipients(&json!({ "foo": "bar" })).is_err());
    }

    #[test]
    fn validate_requires_a_recipient_unless_told_not_to() {
        let mut input = SendInput {
            subject: Some("s".into()),
            html: Some("h".into()),
            ..Default::default()
        };
        assert!(validate(&input, true).is_err());
        assert!(validate(&input, false).is_ok());
        input.to = vec![Recipient {
            address: "a@example.com".into(),
            name: String::new(),
        }];
        assert!(validate(&input, true).is_ok());
    }

    #[test]
    fn validate_rejects_bad_addresses_and_too_many_recipients() {
        let mut input = SendInput {
            subject: Some("s".into()),
            html: Some("h".into()),
            to: vec![Recipient {
                address: "not-an-email".into(),
                name: String::new(),
            }],
            ..Default::default()
        };
        assert!(validate(&input, true).is_err());
        input.to = (0..MAX_RECIPIENTS + 1)
            .map(|i| Recipient {
                address: format!("user{i}@example.com"),
                name: String::new(),
            })
            .collect();
        assert!(validate(&input, true).is_err());
    }

    #[test]
    fn validate_requires_template_or_raw_content() {
        let input = SendInput {
            to: vec![Recipient {
                address: "a@example.com".into(),
                name: String::new(),
            }],
            ..Default::default()
        };
        assert!(validate(&input, true).is_err());
    }
}
