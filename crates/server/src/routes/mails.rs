//! `POST /api/mails/send` and `POST /api/mails/preview`.
//!
//! `preview` stays superuser/API-key only (an API key always resolves to
//! a superuser identity, see `crate::extract`'s module doc) — it is a
//! dashboard/tooling affordance, never something a frontend calls
//! directly.
//!
//! `send` is the one PocketBase-shaped endpoint this platform lets a
//! non-superuser caller (an authenticated app user, or even an anonymous
//! one) reach directly, so a frontend can send transactional mail
//! (invites, receipts, notifications) with no backend of its own. That
//! only works because of `_emailTemplates.sendRule`
//! (`crate::mail_templates::eval_send_rule`):
//!
//! * A superuser or API key may always send anything: a `template`, or
//!   raw `subject`/`html`/`text`, with any `from`/`cc`/`bcc`/`replyTo`
//!   override. Unchanged from before `sendRule` existed.
//! * Anyone else must name a `template` (raw content and every override
//!   are refused outright — those stay an operator-only affordance), to
//!   at most [`MAX_NON_SUPERUSER_RECIPIENTS`] `to` addresses, and that
//!   template's `sendRule` must be non-`null` and evaluate `true` for
//!   *every* `to` address (see [`check_send_permission`]). A denial is
//!   `403`, indistinguishable whether the template doesn't exist, has no
//!   `sendRule`, or the rule itself rejected the caller — nothing here
//!   should leak which.
//!
//! The actual pipeline (resolve `template` or raw content, log, deliver)
//! lives in `crate::mails`, shared with the JS host binding `$mails.send`
//! — which, like a superuser HTTP caller, is never subject to this gate.

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use cratebase_core::AppError;
use serde::Deserialize;
use serde_json::Value;

use crate::app::App;
use crate::extract::{Auth, MaybeAuth, RequireSuperuser};
use crate::http_error::{ApiError, ApiJson, ApiResult};
use crate::mail_templates;
use crate::mails::{self, SendInput};

/// Non-superuser callers may not fan a single send out past this many
/// `to` addresses — `cc`/`bcc` are refused outright for them (see the
/// module doc), so this is the whole recipient count.
pub const MAX_NON_SUPERUSER_RECIPIENTS: usize = 5;

pub fn router() -> Router<App> {
    Router::new()
        .route("/mails/send", post(send))
        .route("/mails/preview", post(preview))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MailBody {
    #[serde(default)]
    to: Value,
    #[serde(default)]
    cc: Value,
    #[serde(default)]
    bcc: Value,
    #[serde(default)]
    template: Option<String>,
    #[serde(default)]
    locale: Option<String>,
    #[serde(default)]
    data: Value,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    html: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    from: Option<Value>,
    #[serde(default, rename = "replyTo")]
    reply_to: Option<String>,
}

fn parse_input(body: MailBody) -> Result<SendInput, ApiError> {
    let to =
        mails::parse_recipients(&body.to).map_err(|e| ApiError::bad_request(format!("to {e}")))?;
    let cc =
        mails::parse_recipients(&body.cc).map_err(|e| ApiError::bad_request(format!("cc {e}")))?;
    let bcc = mails::parse_recipients(&body.bcc)
        .map_err(|e| ApiError::bad_request(format!("bcc {e}")))?;
    let from = match body.from {
        Some(v) => Some(
            mails::parse_recipients(&v)
                .map_err(|e| ApiError::bad_request(format!("from {e}")))?
                .into_iter()
                .next()
                .ok_or_else(|| ApiError::bad_request("from must not be empty."))?,
        ),
        None => None,
    };
    Ok(SendInput {
        to,
        cc,
        bcc,
        template: body.template,
        locale: body.locale,
        data: body.data,
        subject: body.subject,
        html: body.html,
        text: body.text,
        from,
        reply_to: body.reply_to,
    })
}

async fn send(
    State(app): State<App>,
    MaybeAuth(auth): MaybeAuth,
    ApiJson(raw): ApiJson<Value>,
) -> ApiResult<Json<mails::SendOutcome>> {
    let body: MailBody = serde_json::from_value(raw)
        .map_err(|_| ApiError::bad_request(cratebase_core::AppError::DEFAULT_BAD_REQUEST))?;
    if !auth.as_ref().is_some_and(|a| a.is_superuser) {
        check_send_permission(&app, auth.as_ref(), &body).await?;
    }
    let input = parse_input(body)?;
    let outcome = mails::send(&app, input).await.map_err(ApiError)?;
    Ok(Json(outcome))
}

/// `403`, with no detail about *why* — a template that doesn't exist, has
/// no `sendRule`, or a rule that rejected the caller/recipient are all
/// answered identically, so this endpoint never confirms which templates
/// exist or leaks a rule's logic to a probing caller.
fn send_denied() -> ApiError {
    ApiError(AppError::forbidden(
        "This template does not allow this caller to send it.",
    ))
}

/// Gates a non-superuser, non-API-key caller — see the module doc.
/// Superuser/API-key callers never reach this (`send` checks
/// `is_superuser` first).
async fn check_send_permission(
    app: &App,
    auth: Option<&Auth>,
    body: &MailBody,
) -> Result<(), ApiError> {
    let Some(key) = body.template.as_deref() else {
        // No `template`: either a raw send, or a malformed request that
        // `parse_input`/`mails::validate` will reject on its own merits
        // once this caller is a superuser — for anyone else, refusing it
        // here (rather than letting it fall through to a `400`) keeps
        // "raw content is operator-only" a `403`, matching every other
        // denial in this gate.
        return Err(send_denied());
    };
    if body.subject.is_some() || body.html.is_some() || body.text.is_some() {
        return Err(send_denied());
    }
    if body.from.is_some() || body.reply_to.as_ref().is_some_and(|r| !r.is_empty()) {
        return Err(send_denied());
    }
    let cc =
        mails::parse_recipients(&body.cc).map_err(|e| ApiError::bad_request(format!("cc {e}")))?;
    let bcc = mails::parse_recipients(&body.bcc)
        .map_err(|e| ApiError::bad_request(format!("bcc {e}")))?;
    if !cc.is_empty() || !bcc.is_empty() {
        return Err(send_denied());
    }

    let to =
        mails::parse_recipients(&body.to).map_err(|e| ApiError::bad_request(format!("to {e}")))?;
    if to.is_empty() {
        return Err(ApiError::bad_request(
            "to must include at least one recipient.",
        ));
    }
    if to.len() > MAX_NON_SUPERUSER_RECIPIENTS {
        return Err(send_denied());
    }

    let locale = mail_templates::resolve_locale(body.locale.as_deref(), None);
    let template = mail_templates::find_email_template(app, key, &locale)
        .await
        .ok_or_else(send_denied)?;
    let Some(rule) = template.send_rule.as_deref() else {
        return Err(send_denied());
    };

    let auth_ctx = auth.map(Auth::to_auth_context);
    for recipient in &to {
        let allowed = mail_templates::eval_send_rule(
            app,
            rule,
            auth_ctx.as_ref(),
            &recipient.address,
            &body.data,
            &locale,
        )
        .await;
        if !allowed {
            return Err(send_denied());
        }
    }
    Ok(())
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewResponse {
    subject: String,
    html: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
}

async fn preview(
    State(app): State<App>,
    _su: RequireSuperuser,
    ApiJson(raw): ApiJson<Value>,
) -> ApiResult<Json<PreviewResponse>> {
    let body: MailBody = serde_json::from_value(raw)
        .map_err(|_| ApiError::bad_request(cratebase_core::AppError::DEFAULT_BAD_REQUEST))?;
    // Preview never sends, so an empty/absent `to` is fine — `parse_input`
    // still validates it isn't malformed, `mails::preview` doesn't
    // require it to be non-empty.
    let mut body = body;
    if matches!(body.to, Value::Null) {
        body.to = Value::Array(vec![]);
    }
    let input = parse_input(body)?;
    let outcome = mails::preview(&app, &input).await.map_err(ApiError)?;
    Ok(Json(PreviewResponse {
        subject: outcome.subject,
        html: outcome.html,
        text: outcome.text,
    }))
}
