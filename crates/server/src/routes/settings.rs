//! `/api/settings` — superuser only.
//!
//! `PATCH` is a **deep merge** onto the stored settings, not a
//! replacement. That is what makes write-only secrets work: the client
//! never receives `smtp.password`, so it cannot send it back, and an
//! omitted key must therefore keep its stored value rather than being
//! cleared (see [`cratebase_core::Settings::patched`]).
//!
//! A successful `PATCH` persists, swaps the in-memory settings and
//! rebuilds everything derived from them (storage driver, mailer, the
//! `__pbAutoBackup__` cron), so no restart is required.

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use cratebase_core::{codes, AppError, FieldError, Settings};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::app::App;
use crate::extract::{RequestInfo, RequireSuperuser};
use crate::http_error::{ApiError, ApiJson, ApiResult};

/// PocketBase's wrapper message for a rejected settings save.
const SAVE_FAILED: &str = "An error occurred while saving the new settings.";

pub fn router() -> Router<App> {
    Router::new()
        .route("/settings", get(list).patch(update))
        .route("/settings/test/s3", post(test_s3))
        .route("/settings/test/email", post(test_email))
        .route(
            "/settings/apple/generate-client-secret",
            post(apple_client_secret),
        )
}

async fn list(
    State(app): State<App>,
    _su: RequireSuperuser,
    request: RequestInfo,
) -> ApiResult<Json<Value>> {
    let mut event = crate::events::SettingsListEvent::new(app.clone(), request, app.settings());
    app.hooks()
        .on_settings_list_request
        .trigger(&mut event, |_| Box::pin(async { Ok(()) }))
        .await
        .map_err(ApiError)?;
    Ok(Json(event.settings.to_public_json()))
}

async fn update(
    State(app): State<App>,
    _su: RequireSuperuser,
    request: RequestInfo,
    ApiJson(patch): ApiJson<Value>,
) -> ApiResult<Json<Value>> {
    let old = app.settings();
    let merged = old.patched(&patch).map_err(|_| {
        ApiError::bad_request("Failed to load the submitted data due to invalid formatting.")
    })?;
    validate(&merged)?;

    let request = request.with_body(match &patch {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    });
    let mut event = crate::events::SettingsUpdateEvent::new(app.clone(), request, old, merged);
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
        .map_err(ApiError)?;

    Ok(Json(app.settings().to_public_json()))
}

/// Field-level validation, reported as PocketBase's nested tree
/// (`data.meta.appName.code`).
fn validate(s: &Settings) -> Result<(), ApiError> {
    let mut errors: Map<String, Value> = Map::new();
    let mut add = |section: &str, field: &str, code: &str, message: &str| {
        let entry = errors
            .entry(section.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(map) = entry {
            map.insert(
                field.to_string(),
                serde_json::to_value(FieldError::new(code, message)).unwrap_or(Value::Null),
            );
        }
    };

    if s.meta.app_name.trim().is_empty() {
        add("meta", "appName", codes::REQUIRED, "Cannot be blank.");
    }
    if !is_url(&s.meta.app_url) {
        add("meta", "appURL", codes::INVALID_URL, "Must be a valid url.");
    }
    if !s.meta.sender_address.is_empty() && !is_email(&s.meta.sender_address) {
        add(
            "meta",
            "senderAddress",
            codes::INVALID_EMAIL,
            "Must be a valid email address.",
        );
    }
    if s.logs.max_days < 0 {
        add(
            "logs",
            "maxDays",
            "validation_min_greater_equal_than_required",
            "Must be no less than 0.",
        );
    }
    if s.backups.cron_max_keep < 0 {
        add(
            "backups",
            "cronMaxKeep",
            "validation_min_greater_equal_than_required",
            "Must be no less than 0.",
        );
    }
    if !s.backups.cron.trim().is_empty() && s.backups.cron.parse::<croner::Cron>().is_err() {
        add(
            "backups",
            "cron",
            codes::INVALID_FORMAT,
            "Invalid cron expression.",
        );
    }
    if s.batch.max_requests < 0 {
        add(
            "batch",
            "maxRequests",
            "validation_min_greater_equal_than_required",
            "Must be no less than 0.",
        );
    }
    if s.smtp.enabled && s.smtp.host.trim().is_empty() {
        add("smtp", "host", codes::REQUIRED, "Cannot be blank.");
    }
    if s.s3.enabled && s.s3.bucket.trim().is_empty() {
        add("s3", "bucket", codes::REQUIRED, "Cannot be blank.");
    }

    // Rate-limit rules are a list, and PocketBase keys list errors by
    // index (`data.rateLimits.rules["0"]`).
    let mut rule_errors = Map::new();
    for (index, rule) in s.rate_limits.rules.iter().enumerate() {
        let mut per_rule = Map::new();
        if rule.label.trim().is_empty() {
            per_rule.insert(
                "label".into(),
                serde_json::to_value(FieldError::new(codes::REQUIRED, "Cannot be blank.")).unwrap(),
            );
        }
        if rule.max_requests < 0 {
            per_rule.insert(
                "maxRequests".into(),
                serde_json::to_value(FieldError::new(
                    "validation_min_greater_equal_than_required",
                    "Must be no less than 0.",
                ))
                .unwrap(),
            );
        }
        if !per_rule.is_empty() {
            rule_errors.insert(index.to_string(), Value::Object(per_rule));
        }
    }
    if !rule_errors.is_empty() {
        errors.insert(
            "rateLimits".into(),
            json!({ "rules": Value::Object(rule_errors) }),
        );
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ApiError::nested_validation(SAVE_FAILED, errors))
    }
}

fn is_url(raw: &str) -> bool {
    let raw = raw.trim();
    (raw.starts_with("http://") || raw.starts_with("https://"))
        && raw.len() > 8
        && !raw.contains(' ')
}

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

#[derive(Debug, Deserialize)]
struct TestS3Body {
    #[serde(default)]
    filesystem: String,
}

/// Round-trips a probe object through the selected filesystem.
/// PocketBase reports failures as one multi-line message prefixed with
/// `Failed to test the S3 filesystem.`, which the dashboard shows raw.
async fn test_s3(
    State(app): State<App>,
    _su: RequireSuperuser,
    ApiJson(body): ApiJson<TestS3Body>,
) -> ApiResult<axum::http::StatusCode> {
    let settings = app.settings();
    let (enabled, storage) = match body.filesystem.as_str() {
        "backups" => (
            settings.backups.s3.enabled,
            app.backups_storage().map_err(ApiError),
        ),
        // PocketBase defaults to the record-file store.
        _ => (settings.s3.enabled, Ok((*app.storage()).clone())),
    };
    let label = if body.filesystem == "backups" {
        "backups"
    } else {
        "storage"
    };
    if !enabled {
        return Err(s3_failure(format!("S3 {label} filesystem is not enabled.")));
    }
    let storage = storage?;
    let key = format!("cratebase_probe_{}.txt", cratebase_core::record_id());
    storage
        .put(&key, bytes::Bytes::from_static(b"cratebase"))
        .await
        .map_err(|e| s3_failure(e.to_string()))?;
    let read = storage
        .get(&key)
        .await
        .map_err(|e| s3_failure(e.to_string()));
    let _ = storage.delete(&key).await;
    read?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

fn s3_failure(detail: impl std::fmt::Display) -> ApiError {
    ApiError::bad_request(format!(
        "Failed to test the S3 filesystem.\nRaw error: {detail}"
    ))
}

/// The template names `POST /api/settings/test/email` accepts.
const EMAIL_TEMPLATES: &[&str] = &[
    "verification",
    "password-reset",
    "email-change",
    "otp",
    "login-alert",
];

#[derive(Debug, Deserialize)]
struct TestEmailBody {
    #[serde(default)]
    collection: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    template: String,
}

async fn test_email(
    State(app): State<App>,
    _su: RequireSuperuser,
    ApiJson(body): ApiJson<TestEmailBody>,
) -> ApiResult<axum::http::StatusCode> {
    let mut errors: Map<String, Value> = Map::new();
    if !is_email(&body.email) {
        errors.insert(
            "email".into(),
            serde_json::to_value(FieldError::new(
                codes::INVALID_EMAIL,
                "Must be a valid email address.",
            ))
            .unwrap(),
        );
    }
    if !EMAIL_TEMPLATES.contains(&body.template.as_str()) {
        errors.insert(
            "template".into(),
            serde_json::to_value(FieldError::new(
                codes::NOT_IN_LIST,
                "Must be a valid value.",
            ))
            .unwrap(),
        );
    }
    let collection_name = if body.collection.is_empty() {
        cratebase_core::SUPERUSERS_COLLECTION.to_string()
    } else {
        body.collection.clone()
    };
    let collection = app.db().collections.get(&collection_name);
    match &collection {
        Some(c) if c.is_auth() => {}
        _ => {
            errors.insert(
                "collection".into(),
                serde_json::to_value(FieldError::new(
                    codes::NOT_IN_LIST,
                    "Must be a valid auth collection.",
                ))
                .unwrap(),
            );
        }
    }
    if !errors.is_empty() {
        return Err(ApiError::nested_validation(
            "An error occurred while validating the submitted data.",
            errors,
        ));
    }
    let collection = collection.expect("validated above");

    let settings = app.settings();
    let template = match body.template.as_str() {
        "password-reset" => &collection.auth.reset_password_template,
        "email-change" => &collection.auth.confirm_email_change_template,
        "otp" => &collection.auth.otp.email_template,
        "login-alert" => &collection.auth.auth_alert.email_template,
        _ => &collection.auth.verification_template,
    };
    // A test send has no real record behind it, so the placeholders get
    // obviously-fake values, exactly as PocketBase does.
    let (subject, html) = cratebase_mailer::render_template(
        template,
        &[
            ("APP_NAME", settings.meta.app_name.as_str()),
            ("APP_URL", settings.meta.app_url.as_str()),
            ("TOKEN", "__TEST_TOKEN__"),
            ("OTP", "123456"),
            ("OTP_ID", "__TEST_OTP_ID__"),
        ],
    );

    let message = cratebase_mailer::Message::new(
        (
            settings.meta.sender_address.clone(),
            settings.meta.sender_name.clone(),
        ),
        (body.email.clone(), String::new()),
        subject,
        html,
    );
    let mut event = crate::events::MailerEvent::new(app.clone(), message);
    let app_for_finalizer = app.clone();
    app.hooks()
        .on_mailer_send
        .trigger(&mut event, move |e| {
            let app = app_for_finalizer.clone();
            let message = e.message.clone();
            Box::pin(async move {
                app.mailer()
                    .send(&message)
                    .await
                    .map_err(|err| AppError::bad_request(err.to_string()))
            })
        })
        .await
        .map_err(ApiError)?;

    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppleSecretBody {
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    team_id: String,
    #[serde(default)]
    key_id: String,
    #[serde(default)]
    private_key: String,
    #[serde(default)]
    duration: i64,
}

/// Sign the ES256 JWT Apple wants as an OAuth2 "client secret": issued by
/// the team, subject the service id, audience Apple, `kid` the key id.
async fn apple_client_secret(
    State(_app): State<App>,
    _su: RequireSuperuser,
    ApiJson(body): ApiJson<AppleSecretBody>,
) -> ApiResult<Json<Value>> {
    let mut errors: Map<String, Value> = Map::new();
    let mut require = |field: &str, value: &str| {
        if value.trim().is_empty() {
            errors.insert(
                field.to_string(),
                serde_json::to_value(FieldError::new(codes::REQUIRED, "Cannot be blank.")).unwrap(),
            );
        }
    };
    require("clientId", &body.client_id);
    require("teamId", &body.team_id);
    require("keyId", &body.key_id);
    require("privateKey", &body.private_key);
    if !(0..=15_777_000).contains(&body.duration) || body.duration == 0 {
        errors.insert(
            "duration".into(),
            serde_json::to_value(FieldError::new(
                codes::INVALID_NUMBER,
                "Must be between 1 and 15777000 seconds.",
            ))
            .unwrap(),
        );
    }
    if !errors.is_empty() {
        return Err(ApiError::nested_validation(
            "An error occurred while validating the submitted data.",
            errors,
        ));
    }

    let now = chrono::Utc::now().timestamp();
    let claims = json!({
        "iss": body.team_id,
        "iat": now,
        "exp": now + body.duration,
        "aud": "https://appleid.apple.com",
        "sub": body.client_id,
    });
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
    header.kid = Some(body.key_id.clone());
    let key = jsonwebtoken::EncodingKey::from_ec_pem(body.private_key.as_bytes())
        .map_err(|e| ApiError::bad_request(format!("Invalid private key: {e}")))?;
    let secret = jsonwebtoken::encode(&header, &claims, &key)
        .map_err(|e| ApiError::bad_request(format!("Failed to sign the client secret: {e}")))?;

    Ok(Json(json!({ "secret": secret })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_and_email_checks() {
        assert!(is_url("http://localhost:8090"));
        assert!(is_url("https://example.com/base"));
        assert!(!is_url("not a url"));
        assert!(!is_url("ftp://example.com"));
        assert!(is_email("a@b.co"));
        assert!(!is_email("not-an-email"));
        assert!(!is_email("a@b"));
    }

    #[test]
    fn validation_reports_a_nested_tree() {
        let mut s = Settings::default();
        s.meta.app_name = String::new();
        s.meta.app_url = "not a url".into();
        s.logs.max_days = -1;
        let err = validate(&s).unwrap_err();
        assert_eq!(err.error.status(), 400);
        assert_eq!(err.error.body().message, SAVE_FAILED);
        let data = err.data.unwrap();
        assert_eq!(data["meta"]["appName"]["code"], codes::REQUIRED);
        assert!(data["meta"]["appURL"].is_object());
        assert!(data["logs"]["maxDays"].is_object());
    }

    #[test]
    fn the_defaults_validate() {
        validate(&Settings::default()).unwrap();
    }
}
