//! App settings, shaped like `GET /api/settings` in PocketBase v0.23+ and
//! persisted as JSON in `_params`. Secrets (`smtp.password`, `s3.secret`,
//! `backups.s3.secret`) are accepted on input and stored, but stripped
//! from the public JSON by [`Settings::to_public_json`].

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

impl Default for RateLimitRule {
    fn default() -> Self {
        RateLimitRule {
            label: String::new(),
            audience: String::new(),
            duration: 0,
            max_requests: 0,
        }
    }
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
