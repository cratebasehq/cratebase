//! `POST /api/mails/send` and `POST /api/mails/preview` — superuser or
//! API key only (an API key always resolves to a superuser identity, see
//! `crate::extract`'s module doc, so [`RequireSuperuser`] alone already
//! covers both). The actual pipeline (resolve `template` or raw content,
//! log, deliver) lives in `crate::mails`, shared with the JS host binding
//! `$mails.send`.

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::Value;

use crate::app::App;
use crate::extract::RequireSuperuser;
use crate::http_error::{ApiError, ApiJson, ApiResult};
use crate::mails::{self, SendInput};

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
    _su: RequireSuperuser,
    ApiJson(raw): ApiJson<Value>,
) -> ApiResult<Json<mails::SendOutcome>> {
    let body: MailBody = serde_json::from_value(raw)
        .map_err(|_| ApiError::bad_request(cratebase_core::AppError::DEFAULT_BAD_REQUEST))?;
    let input = parse_input(body)?;
    let outcome = mails::send(&app, input).await.map_err(ApiError)?;
    Ok(Json(outcome))
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
