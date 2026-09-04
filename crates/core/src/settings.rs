//! App settings, shaped like `GET /api/settings` in PocketBase v0.23+ and
//! persisted as JSON in `_params`. Secrets (`smtp.password`, `s3.secret`,
//! `backups.s3.secret`, `llm.apiKey`, `sms.authToken`,
//! `push.vapid.privateKey`, `push.fcm.serviceAccountJson`, `push.apns.key`)
//! are accepted on input and stored, but stripped from the public JSON by
//! [`Settings::to_public_json`].

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Meta {
    pub app_name: String,
    #[serde(rename = "appURL")]
    pub app_url: String,
    pub sender_name: String,
    pub sender_address: String,
    pub hide_controls: bool,
    pub accent_color: String,
}

impl Default for Meta {
    fn default() -> Self {
        Meta {
            app_name: "Acme".into(),
            app_url: "http://localhost:8090".into(),
            sender_name: "Support".into(),
            sender_address: "support@example.com".into(),
            hide_controls: false,
            accent_color: "#1055c9".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Smtp {
    pub enabled: bool,
    pub port: u16,
    pub host: String,
    pub username: String,
    pub password: String,
    pub auth_method: String,
    pub tls: bool,
    pub local_name: String,
}

impl Default for Smtp {
    fn default() -> Self {
        Smtp {
            enabled: false,
            port: 587,
            host: "smtp.example.com".into(),
            username: String::new(),
            password: String::new(),
            auth_method: String::new(),
            tls: false,
            local_name: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct S3 {
    pub enabled: bool,
    pub bucket: String,
    pub region: String,
    pub endpoint: String,
    pub access_key: String,
    pub secret: String,
    pub force_path_style: bool,
}

/// LLM provider config for `POST /api/llm/chat` (`crates/server/src/llm.rs`).
/// Mirrors [`Smtp`]'s shape: `enabled` picks between the configured
/// provider and the zero-config fallback (an echo provider that needs no
/// network access), exactly like `smtp.enabled` picks between
/// `SmtpBackend` and `LogBackend` in `cratebase_mailer`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Llm {
    pub enabled: bool,
    /// `"openai"` (or any OpenAI-compatible `/chat/completions` API, e.g.
    /// a local Ollama instance) is the only real provider today;
    /// anything else falls back to the echo provider.
    pub provider: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl Default for Llm {
    fn default() -> Self {
        Llm {
            enabled: false,
            provider: "openai".into(),
            base_url: "https://api.openai.com/v1".into(),
            api_key: String::new(),
            model: "gpt-4o-mini".into(),
        }
    }
}

/// SMS provider config for `cratebase_mailer::sms` (Twilio-compatible REST
/// API). Mirrors [`Smtp`]'s shape: `enabled` picks between the configured
/// Twilio backend and the zero-config log fallback, exactly like
/// `smtp.enabled` picks between `SmtpBackend` and `LogBackend`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Sms {
    pub enabled: bool,
    pub account_sid: String,
    pub auth_token: String,
    pub from_number: String,
}

/// Push notification provider config for `POST /api/push/send`
/// (`crates/server/src/push.rs`). Unlike [`Smtp`]/[`Llm`]/[`Sms`], there is
/// no single "provider" choice: a caller targets a `_push_subscriptions`
/// row whose own `platform` field (`web`/`android`/`ios`) picks one of
/// three independent backends, so each backend gets its own `enabled` flag
/// rather than sharing one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Push {
    pub vapid: VapidConfig,
    pub fcm: FcmConfig,
    pub apns: ApnsConfig,
    /// Record-event → push rules, evaluated the same way
    /// `cratebase_server::webhooks` evaluates `_webhooks` rows, but kept
    /// here in settings rather than as a new system collection: adding one
    /// would mean editing `cratebase_core::Collection::default_system_collections`
    /// and the migration registry, both outside this feature's owned
    /// files, for a config shape (`collection`/`events`/two templates)
    /// settings' existing JSON-blob-of-rules pattern (see
    /// [`RateLimits::rules`]) already covers with zero schema surface.
    pub triggers: Vec<PushTrigger>,
}

/// Web Push / VAPID (RFC 8292) — works from any browser with no
/// per-platform app registration. `private_key`/`public_key` are the raw
/// P-256 key pair as used by the `web-push generate-vapid-keys` CLI and
/// every JS web-push library: unpadded base64url, not PEM. `subject` is
/// the contact URI RFC 8292 requires in every VAPID JWT (`mailto:...` or
/// `https://...`), so a push service can reach the operator about a
/// misbehaving sender.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct VapidConfig {
    pub enabled: bool,
    pub public_key: String,
    pub private_key: String,
    pub subject: String,
}

/// Firebase Cloud Messaging HTTP v1 API, for Android (and web, though this
/// server only ever selects it for `platform = "android"` —
/// see `crate::push`). `service_account_json` is the raw contents of the
/// Firebase service-account key file downloaded from the Firebase console;
/// the project id, client email and private key used to sign the OAuth2
/// bearer JWT are all parsed out of it, so there is nothing else to
/// configure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct FcmConfig {
    pub enabled: bool,
    pub service_account_json: String,
}

/// Apple Push Notification service, HTTP/2 provider API, token-based
/// (`.p8`) auth — no expiring certificate to renew. `key` is the raw `.p8`
/// file contents (PEM), `key_id`/`team_id` come from the Apple Developer
/// portal page the key was created on, `bundle_id` is the app's bundle
/// identifier (sent as `apns-topic`), and `production` picks
/// `api.push.apple.com` over the `api.sandbox.push.apple.com` used by
/// development-signed builds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ApnsConfig {
    pub enabled: bool,
    pub key: String,
    pub key_id: String,
    pub team_id: String,
    pub bundle_id: String,
    pub production: bool,
}

/// One record-event → push rule. `events` is a comma-separated subset of
/// `create`/`update`/`delete`, same grammar as `_webhooks.events`.
/// `title`/`body` are `{{field}}`-templated against the triggering
/// record's JSON. `target_field` picks who receives it: empty broadcasts
/// to every enabled `_push_subscriptions` row; non-empty names a field on
/// the triggering record whose value must equal a subscription's
/// `recordRef` (e.g. a `userRef` field on a `comments` collection, so only
/// that comment's addressee is notified).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct PushTrigger {
    pub enabled: bool,
    pub collection: String,
    pub events: String,
    pub title: String,
    pub body: String,
    pub target_field: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Backups {
    pub cron: String,
    pub cron_max_keep: i64,
    pub s3: S3,
}

impl Default for Backups {
    fn default() -> Self {
        Backups {
            cron: String::new(),
            cron_max_keep: 3,
            s3: S3::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RateLimitRule {
    /// A path prefix (`/api/`), an exact path, or a tag (`*:auth`,
    /// `*:create`, `posts:list`, ...).
    pub label: String,
    /// `""` (all), `@guest`, or `@auth`.
    pub audience: String,
    pub duration: i64,
    pub max_requests: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RateLimits {
    pub rules: Vec<RateLimitRule>,
    #[serde(rename = "excludedIPs")]
    pub excluded_ips: Vec<String>,
    pub enabled: bool,
}

impl Default for RateLimits {
    fn default() -> Self {
        RateLimits {
            rules: vec![
                RateLimitRule {
                    label: "*:auth".into(),
                    audience: String::new(),
                    duration: 3,
                    max_requests: 2,
                },
                RateLimitRule {
                    label: "*:create".into(),
                    audience: String::new(),
                    duration: 5,
                    max_requests: 20,
                },
                RateLimitRule {
                    label: "/api/batch".into(),
                    audience: String::new(),
                    duration: 1,
                    max_requests: 3,
                },
                RateLimitRule {
                    label: "/api/".into(),
                    audience: String::new(),
                    duration: 10,
                    max_requests: 300,
                },
            ],
            excluded_ips: vec![],
            enabled: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TrustedProxy {
    pub headers: Vec<String>,
    #[serde(rename = "useLeftmostIP")]
    pub use_leftmost_ip: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Batch {
    pub enabled: bool,
    pub max_requests: i64,
    pub timeout: i64,
    pub max_body_size: i64,
}

impl Default for Batch {
    fn default() -> Self {
        Batch {
            enabled: false,
            max_requests: 50,
            timeout: 3,
            max_body_size: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Logs {
    pub max_days: i64,
    pub min_level: i64,
    #[serde(rename = "logIP")]
    pub log_ip: bool,
    pub log_auth_id: bool,
    pub max_data_size: i64,
}

impl Default for Logs {
    fn default() -> Self {
        Logs {
            max_days: 5,
            min_level: 0,
            log_ip: true,
            log_auth_id: false,
            max_data_size: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub meta: Meta,
    pub smtp: Smtp,
    pub s3: S3,
    pub backups: Backups,
    pub rate_limits: RateLimits,
    pub trusted_proxy: TrustedProxy,
    pub batch: Batch,
    pub logs: Logs,
    pub llm: Llm,
    pub sms: Sms,
    pub push: Push,
    #[serde(rename = "superuserIPs")]
    pub superuser_ips: Vec<String>,
}

impl Settings {
    /// JSON for `GET /api/settings`: everything except write-only secrets.
    pub fn to_public_json(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Some(smtp) = v.get_mut("smtp").and_then(Value::as_object_mut) {
            smtp.remove("password");
        }
        if let Some(s3) = v.get_mut("s3").and_then(Value::as_object_mut) {
            s3.remove("secret");
        }
        if let Some(llm) = v.get_mut("llm").and_then(Value::as_object_mut) {
            llm.remove("apiKey");
        }
        if let Some(sms) = v.get_mut("sms").and_then(Value::as_object_mut) {
            sms.remove("authToken");
        }
        if let Some(vapid) = v
            .get_mut("push")
            .and_then(|p| p.get_mut("vapid"))
            .and_then(Value::as_object_mut)
        {
            vapid.remove("privateKey");
        }
        if let Some(fcm) = v
            .get_mut("push")
            .and_then(|p| p.get_mut("fcm"))
            .and_then(Value::as_object_mut)
        {
            fcm.remove("serviceAccountJson");
        }
        if let Some(apns) = v
            .get_mut("push")
            .and_then(|p| p.get_mut("apns"))
            .and_then(Value::as_object_mut)
        {
            apns.remove("key");
        }
        if let Some(s3) = v
            .get_mut("backups")
            .and_then(|b| b.get_mut("s3"))
            .and_then(Value::as_object_mut)
        {
            s3.remove("secret");
        }
        v
    }

    /// Apply a partial update: `patch` is deep-merged onto the current
    /// JSON, so omitted secrets keep their stored values.
    pub fn patched(&self, patch: &Value) -> Result<Settings, serde_json::Error> {
        let mut current = serde_json::to_value(self)?;
        deep_merge(&mut current, patch);
        serde_json::from_value(current)
    }
}

fn deep_merge(target: &mut Value, patch: &Value) {
    match (target, patch) {
        (Value::Object(t), Value::Object(p)) => {
            for (k, v) in p {
                match t.get_mut(k) {
                    Some(existing) if existing.is_object() && v.is_object() => {
                        deep_merge(existing, v)
                    }
                    _ => {
                        t.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (t, p) => *t = p.clone(),
    }
}

/// Helper for callers that want a `Map` rather than a `Value`.
pub fn as_map(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_pocketbase_fixture() {
        let s = Settings::default();
        let v = s.to_public_json();
        assert_eq!(v["meta"]["appName"], "Acme");
        assert_eq!(v["meta"]["accentColor"], "#1055c9");
        assert_eq!(v["smtp"]["port"], 587);
        assert!(v["smtp"].get("password").is_none());
        assert!(v["s3"].get("secret").is_none());
        assert!(v["backups"]["s3"].get("secret").is_none());
        assert!(v["sms"].get("authToken").is_none());
        assert_eq!(v["rateLimits"]["rules"][0]["label"], "*:auth");
        assert_eq!(v["batch"]["maxRequests"], 50);
        assert_eq!(v["logs"]["maxDays"], 5);
        assert_eq!(v["superuserIPs"], serde_json::json!([]));
    }

    #[test]
    fn patch_keeps_secrets_when_omitted() {
        let mut s = Settings::default();
        s.smtp.password = "hunter2".into();
        let patched = s
            .patched(&serde_json::json!({"smtp": {"host": "mail.example.com"}}))
            .unwrap();
        assert_eq!(patched.smtp.host, "mail.example.com");
        assert_eq!(patched.smtp.password, "hunter2");
        assert_eq!(patched.smtp.port, 587);
    }
}
