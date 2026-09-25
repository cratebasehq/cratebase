//! Resolution order for the six mailable auth-flow templates
//! (verification, password reset, email change, OTP, new-location login
//! alert, magic link), shared by `routes::auth`:
//!
//! 1. The collection's own `authOptions.*Template`/`*.emailTemplate`
//!    field, **if** it has been customized away from
//!    `cratebase_core::collection::AuthOptions::default()`'s value for
//!    that field — rendered with the existing `{PLACEHOLDER}` syntax via
//!    [`cratebase_mailer::render_template`], unchanged from before this
//!    module existed.
//! 2. Otherwise, an `_emailTemplates` row keyed `auth.<kind>` (falling
//!    back from the requested locale to `""` when that locale has no
//!    row) — rendered with the new `{{var}}` syntax via
//!    [`cratebase_mailer::render_email_template`].
//! 3. Otherwise (no such row — e.g. an admin deleted it), the same
//!    built-in default as (1), rendered the same legacy way — so a
//!    database with no customization and an intact `_emailTemplates`
//!    seed, and one with the seed row deleted, both still send the
//!    original PocketBase-style email.
//!
//! This keeps every existing deployment's mail byte-for-byte unchanged
//! unless it opts in, either by editing the collection's own template
//! field or by editing the seeded `_emailTemplates` row.

use std::sync::Arc;

use cratebase_core::collection::AuthOptions;
use cratebase_core::{Collection, EmailTemplate};
use cratebase_db::records;
use serde_json::{Map, Value};

use crate::app::App;

/// The six auth-flow emails resolvable through `_emailTemplates`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMailKind {
    Verification,
    PasswordReset,
    EmailChange,
    Otp,
    LoginAlert,
    MagicLink,
}

impl AuthMailKind {
    /// The `_emailTemplates.key` this kind resolves to.
    pub fn key(self) -> &'static str {
        match self {
            AuthMailKind::Verification => "auth.verification",
            AuthMailKind::PasswordReset => "auth.passwordReset",
            AuthMailKind::EmailChange => "auth.emailChange",
            AuthMailKind::Otp => "auth.otp",
            AuthMailKind::LoginAlert => "auth.loginAlert",
            AuthMailKind::MagicLink => "auth.magic-link",
        }
    }

    /// `(current, default)` for this kind's template field, used to
    /// detect customization — see the module doc's step 1.
    fn templates(self, collection: &Collection) -> (EmailTemplate, EmailTemplate) {
        let default = AuthOptions::default();
        match self {
            AuthMailKind::Verification => (
                collection.auth.verification_template.clone(),
                default.verification_template,
            ),
            AuthMailKind::PasswordReset => (
                collection.auth.reset_password_template.clone(),
                default.reset_password_template,
            ),
            AuthMailKind::EmailChange => (
                collection.auth.confirm_email_change_template.clone(),
                default.confirm_email_change_template,
            ),
            AuthMailKind::Otp => (
                collection.auth.otp.email_template.clone(),
                default.otp.email_template,
            ),
            AuthMailKind::LoginAlert => (
                collection.auth.auth_alert.email_template.clone(),
                default.auth_alert.email_template,
            ),
            AuthMailKind::MagicLink => (
                collection.auth.magic_link.email_template.clone(),
                default.magic_link.email_template,
            ),
        }
    }
}

/// A resolved, rendered auth-flow email.
pub struct ResolvedMail {
    pub subject: String,
    pub html: String,
    /// `Some` only when rendered through the `_emailTemplates`/`{{var}}`
    /// path (step 2) — the legacy `render_template` path never produces
    /// a text alternative.
    pub text: Option<String>,
}

/// Resolves and renders one auth-flow email per the module doc's
/// priority order.
///
/// `legacy_vars` supplies the `{PLACEHOLDER}` values the legacy path
/// needs beyond `{APP_NAME}`/`{APP_URL}` (added automatically) — e.g.
/// `[("TOKEN", tok)]` or `[("OTP", code), ("OTP_ID", id), ...]`.
/// `data` supplies the `{{var}}` values the `_emailTemplates` path needs
/// beyond `{{appName}}`/`{{appUrl}}` (added automatically) — e.g.
/// `json!({ "token": tok, "user": { "name": ... } })`. `locale` is the
/// already-resolved recipient locale (see [`crate::locale::resolve`]),
/// `""` meaning "no preference".
pub async fn resolve_auth_mail(
    app: &App,
    collection: &Collection,
    kind: AuthMailKind,
    locale: &str,
    legacy_vars: &[(&str, &str)],
    data: Value,
) -> ResolvedMail {
    let (current, default) = kind.templates(collection);
    let render_legacy = |template: &EmailTemplate| {
        let settings = app.settings();
        let mut vars: Vec<(&str, &str)> = vec![
            ("APP_NAME", settings.meta.app_name.as_str()),
            ("APP_URL", settings.meta.app_url.as_str()),
        ];
        vars.extend_from_slice(legacy_vars);
        cratebase_mailer::render_template(template, &vars)
    };

    if current != default {
        let (subject, html) = render_legacy(&current);
        return ResolvedMail {
            subject,
            html,
            text: None,
        };
    }

    if let Some(row) = find_email_template(app, kind.key(), locale).await {
        let doc = cratebase_mailer::TemplateDoc {
            subject: row.subject.as_str(),
            html: row.html.as_str(),
            text: row.text.as_str(),
            layout: row.layout,
        };
        let (subject, html, text) =
            cratebase_mailer::render_email_template(&doc, &data, &app.settings().meta);
        return ResolvedMail {
            subject,
            html,
            text: Some(text),
        };
    }

    let (subject, html) = render_legacy(&default);
    ResolvedMail {
        subject,
        html,
        text: None,
    }
}

/// One `_emailTemplates` row's renderable content.
pub struct EmailTemplateRow {
    pub subject: String,
    pub html: String,
    pub text: String,
    pub layout: bool,
}

/// Looks up `_emailTemplates` by `key`, trying `locale` first (when
/// non-empty) and falling back to the `""` locale.
pub async fn find_email_template(app: &App, key: &str, locale: &str) -> Option<EmailTemplateRow> {
    let collection = app.db().collections.get_by_name("_emailTemplates")?;
    if !locale.is_empty() {
        if let Some(row) = find_email_template_row(app, &collection, key, locale).await {
            return Some(row);
        }
    }
    find_email_template_row(app, &collection, key, "").await
}

async fn find_email_template_row(
    app: &App,
    collection: &Arc<Collection>,
    key: &str,
    locale: &str,
) -> Option<EmailTemplateRow> {
    let mut params = Map::new();
    params.insert("key".into(), Value::String(key.to_string()));
    params.insert("locale".into(), Value::String(locale.to_string()));
    let record = records::find_first_by_filter(
        app.db(),
        &app.db().collections,
        collection,
        "key = {:key} && locale = {:locale}",
        &params,
    )
    .await
    .ok()??;
    Some(EmailTemplateRow {
        subject: record.get_string("subject"),
        html: record.get_string("html"),
        text: record.get_string("text"),
        layout: record.get_bool("layout"),
    })
}

/// The recipient locale to render with: an explicit `locale` argument
/// wins; otherwise a `lang`/`locale` field on `record` (in that order),
/// when present and non-empty; otherwise `""` (no preference — the
/// `_emailTemplates` lookup above falls back to the `""`-locale row).
pub fn resolve_locale(explicit: Option<&str>, record: Option<&cratebase_core::Record>) -> String {
    if let Some(l) = explicit {
        if !l.is_empty() {
            return l.to_string();
        }
    }
    if let Some(record) = record {
        for field in ["locale", "lang"] {
            let v = record.get_string(field);
            if !v.is_empty() {
                return v;
            }
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use cratebase_db::Executor;

    async fn test_app() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::new(Config::memory(dir.path()));
        app.bootstrap().await.expect("bootstrap");
        (app, dir)
    }

    #[test]
    fn resolve_locale_prefers_explicit_then_record_locale_then_lang() {
        use cratebase_core::field::{Field, FieldKind};
        let mut collection = Collection::new("users", cratebase_core::CollectionType::Auth);
        let text = |name: &str| {
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
        };
        collection.fields.push(text("lang"));
        collection.fields.push(text("locale"));
        let mut record = cratebase_core::Record::new(std::sync::Arc::new(collection));
        record.set("lang", Value::String("fr".into()));
        assert_eq!(resolve_locale(Some("de"), Some(&record)), "de");
        assert_eq!(resolve_locale(None, Some(&record)), "fr");
        record.set("locale", Value::String("es".into()));
        assert_eq!(
            resolve_locale(None, Some(&record)),
            "es",
            "locale beats lang"
        );
        assert_eq!(resolve_locale(None, None), "");
        assert_eq!(
            resolve_locale(Some(""), Some(&record)),
            "es",
            "empty explicit is ignored"
        );
    }

    #[tokio::test]
    async fn default_template_falls_through_to_email_templates_row() {
        let (app, _dir) = test_app().await;
        let collection = app.db().collections.get("users").unwrap();
        let resolved = resolve_auth_mail(
            &app,
            &collection,
            AuthMailKind::Verification,
            "",
            &[("TOKEN", "tok")],
            serde_json::json!({ "token": "tok" }),
        )
        .await;
        // The seeded `_emailTemplates` row renders through the new
        // engine, so it comes back with a derived text alternative —
        // the legacy path never produces one.
        assert!(resolved.text.is_some());
        assert!(resolved.html.contains("tok"));
        assert_eq!(resolved.subject, "Verify your Acme email");
    }

    #[tokio::test]
    async fn customized_collection_template_wins_over_email_templates_row() {
        let (app, _dir) = test_app().await;
        let mut collection = (*app.db().collections.get("users").unwrap()).clone();
        collection.auth.verification_template = EmailTemplate {
            subject: "Custom subject {APP_NAME}".into(),
            body: "<p>Custom body {TOKEN}</p>".into(),
        };
        let resolved = resolve_auth_mail(
            &app,
            &collection,
            AuthMailKind::Verification,
            "",
            &[("TOKEN", "tok")],
            serde_json::json!({ "token": "tok" }),
        )
        .await;
        assert_eq!(resolved.subject, "Custom subject Acme");
        assert!(resolved.html.contains("Custom body tok"));
        assert!(resolved.text.is_none(), "legacy path has no text alt");
    }

    #[tokio::test]
    async fn missing_email_templates_row_falls_back_to_builtin_default() {
        let (app, _dir) = test_app().await;
        let collection = app.db().collections.get("users").unwrap();
        app.db()
            .execute(
                r#"DELETE FROM "_emailTemplates" WHERE "key" = 'auth.verification'"#,
                &[],
            )
            .await
            .expect("delete seeded row");
        let resolved = resolve_auth_mail(
            &app,
            &collection,
            AuthMailKind::Verification,
            "",
            &[("TOKEN", "tok")],
            serde_json::json!({ "token": "tok" }),
        )
        .await;
        assert_eq!(resolved.subject, "Verify your Acme email");
        assert!(resolved.html.contains("tok"));
        assert!(resolved.text.is_none(), "builtin fallback has no text alt");
    }
}
