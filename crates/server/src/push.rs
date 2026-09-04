//! Push notification delivery: a [`PushProvider`] trait, three backends
//! (Web Push/VAPID, FCM, APNs) selected by a `_push_subscriptions` row's
//! own `platform` field, and [`bind_hooks`] for record-event → push
//! rules configured in `settings.push.triggers`.
//!
//! # Provider selection mirrors `cratebase_mailer`
//!
//! [`PushService::from_settings`] is the `provider_from_settings` /
//! `Mailer::from_settings` shape used everywhere else in this codebase:
//! each backend's own `enabled` flag picks between a real, network-calling
//! implementation and simply not being configured (`Err(NotConfigured)`
//! rather than a fake success) — there is no single echo/log fallback
//! here because, unlike SMTP or an LLM chat, there is no meaningful
//! zero-config default for "deliver a push notification to a real device".
//!
//! # What `_push_subscriptions.token` holds, per platform
//!
//! The system collection (`cratebase_core::Collection::default_system_collections`)
//! has exactly one opaque `token` field for all three platforms, described
//! there as "a Web Push endpoint URL, an FCM registration token, or an
//! APNs device token". That holds for FCM and APNs, where a single string
//! really is the whole address. It does not hold for Web Push: RFC 8291
//! payload encryption needs the subscription's `p256dh` (its ECDH public
//! key) and `auth` (a 16-byte secret) in addition to the endpoint URL —
//! exactly what `PushManager.subscribe()` returns in the browser. Rather
//! than widening the schema (out of this feature's owned files — see the
//! `_push_subscriptions` collection's own doc comment), [`WebPushProvider`]
//! treats `token` as a JSON-encoded [`WebPushSubscriptionToken`] for the
//! `"web"` platform only: `{"endpoint":"...","p256dh":"...","auth":"..."}`.
//! The dashboard/SDK side of this (not built here) is expected to
//! `JSON.stringify` the browser's `PushSubscription.toJSON()` shape when
//! creating the `_push_subscriptions` row.
//!
//! # Why triggers live in `settings.push`, not a new collection
//!
//! See [`cratebase_core::settings::Push::triggers`]'s own doc comment: a
//! `_push_triggers` collection (the assignment's other suggested option)
//! would need edits to `Collection::default_system_collections` and the
//! migration registry, both outside this feature's owned files this
//! session. `settings.push.triggers` gets the same "configure without a
//! redeploy" property through the JSON settings blob other rule lists
//! (`settings.rate_limits.rules`) already use.
//!
//! # Dead-token cleanup
//!
//! Every provider maps an unregistered/invalid-token response — FCM's
//! `UNREGISTERED` error code, APNs' `410 Unregistered` (and
//! `400 BadDeviceToken`, which means the same thing for a token that was
//! never valid), a Web Push `404`/`410` — to [`PushError::InvalidToken`]
//! rather than [`PushError::Delivery`]. Callers ([`crate::routes::push`],
//! [`dispatch_trigger`] below) treat that specific variant as "stop
//! retrying this subscription" and disable the row with a raw `UPDATE`,
//! the same write-back-bypasses-the-record-API pattern
//! `crate::webhooks::deliver` uses for its own status columns.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::sync::RwLock;
use web_push_native::jwt_simple::algorithms::ES256KeyPair;
use web_push_native::{p256, Auth as WebPushAuth, WebPushBuilder};

use cratebase_core::settings::{ApnsConfig, FcmConfig, Push, VapidConfig};
use cratebase_db::engine::{Executor, Sql};

use crate::app::App;
use crate::events::RecordEvent;
use crate::hooks::{Event, Handler};

/// The notification content handed to every provider. `data` is
/// arbitrary caller-supplied JSON, forwarded as best each platform's
/// wire format allows (FCM requires string values; see
/// [`FcmProvider::send`]).
#[derive(Debug, Clone, Default, Serialize)]
pub struct PushPayload {
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub data: Value,
}

/// A provider or transport failure.
#[derive(Debug)]
pub enum PushError {
    /// No backend is configured for the requested platform (its `enabled`
    /// flag is off, or required settings are missing/invalid).
    NotConfigured(String),
    /// The provider rejected the token itself as unregistered/expired —
    /// see the module doc's "Dead-token cleanup" section.
    InvalidToken,
    /// Any other failure: a malformed subscription token, a network
    /// error, a non-2xx response that isn't a dead-token signal.
    Delivery(String),
}

impl std::fmt::Display for PushError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PushError::NotConfigured(msg) => write!(f, "push not configured: {msg}"),
            PushError::InvalidToken => write!(f, "push subscription token is no longer valid"),
            PushError::Delivery(msg) => write!(f, "push delivery failed: {msg}"),
        }
    }
}

impl std::error::Error for PushError {}

/// A delivery backend for one push platform.
#[async_trait]
pub trait PushProvider: Send + Sync {
    async fn send(
        &self,
        token: &str,
        platform: &str,
        payload: &PushPayload,
    ) -> Result<(), PushError>;
}

/// The shared outbound HTTP client for every provider in this module —
/// one static client, the same construction convention
/// `crate::webhooks::deliver` uses, rather than a fresh client per call.
/// Redirects are disabled: none of the three provider endpoints below are
/// operator-supplied URLs (unlike a `_webhooks` target), so this isn't
/// the SSRF concern `crate::webhooks` documents, but a provider redirect
/// still has no legitimate reason to be followed silently.
fn http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .expect("static client config is valid")
    });
    &CLIENT
}

// ---------------------------------------------------------------------------
// Web Push / VAPID
// ---------------------------------------------------------------------------

/// The JSON shape `_push_subscriptions.token` holds for `platform = "web"`
/// — see the module doc comment.
#[derive(Debug, Deserialize)]
struct WebPushSubscriptionToken {
    endpoint: String,
    p256dh: String,
    auth: String,
}

pub struct WebPushProvider {
    key_pair: ES256KeyPair,
    subject: String,
}

impl WebPushProvider {
    /// `private_key_b64url` is the raw P-256 scalar as emitted by
    /// `web-push generate-vapid-keys` / any JS web-push library —
    /// unpadded base64url, not PEM.
    pub fn new(private_key_b64url: &str, subject: &str) -> Result<Self, PushError> {
        let bytes = URL_SAFE_NO_PAD
            .decode(private_key_b64url)
            .map_err(|e| PushError::NotConfigured(format!("invalid VAPID private key: {e}")))?;
        let key_pair = ES256KeyPair::from_bytes(&bytes)
            .map_err(|e| PushError::NotConfigured(format!("invalid VAPID private key: {e}")))?;
        Ok(WebPushProvider {
            key_pair,
            subject: subject.to_string(),
        })
    }
}

/// A push-service response that means "this subscription is gone, stop
/// sending to it": RFC 8030 §7 documents both codes push services use for
/// an endpoint that no longer exists.
fn is_web_push_dead(status: reqwest::StatusCode) -> bool {
    status.as_u16() == 404 || status.as_u16() == 410
}

#[async_trait]
impl PushProvider for WebPushProvider {
    async fn send(
        &self,
        token: &str,
        _platform: &str,
        payload: &PushPayload,
    ) -> Result<(), PushError> {
        let sub: WebPushSubscriptionToken = serde_json::from_str(token)
            .map_err(|e| PushError::Delivery(format!("malformed web push subscription: {e}")))?;
        let endpoint: http::Uri = sub
            .endpoint
            .parse()
            .map_err(|e| PushError::Delivery(format!("invalid web push endpoint: {e}")))?;
        let p256dh_bytes = URL_SAFE_NO_PAD
            .decode(&sub.p256dh)
            .map_err(|e| PushError::Delivery(format!("invalid p256dh: {e}")))?;
        let public_key = p256::PublicKey::from_sec1_bytes(&p256dh_bytes)
            .map_err(|e| PushError::Delivery(format!("invalid p256dh: {e}")))?;
        let auth_bytes = URL_SAFE_NO_PAD
            .decode(&sub.auth)
            .map_err(|e| PushError::Delivery(format!("invalid auth secret: {e}")))?;
        if auth_bytes.len() != 16 {
            return Err(PushError::Delivery(
                "web push auth secret must be 16 bytes".into(),
            ));
        }
        let auth = WebPushAuth::clone_from_slice(&auth_bytes);

        let builder = WebPushBuilder::new(endpoint, public_key, auth)
            .with_vapid(&self.key_pair, &self.subject);
        let body = serde_json::to_vec(payload)
            .map_err(|e| PushError::Delivery(format!("failed to encode payload: {e}")))?;
        let request = builder
            .build(body)
            .map_err(|e| PushError::Delivery(format!("failed to build web push request: {e}")))?;
        let request = reqwest::Request::try_from(request)
            .map_err(|e| PushError::Delivery(format!("failed to build web push request: {e}")))?;

        let resp = http_client()
            .execute(request)
            .await
            .map_err(|e| PushError::Delivery(e.to_string()))?;
        let status = resp.status();
        if is_web_push_dead(status) {
            return Err(PushError::InvalidToken);
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(PushError::Delivery(format!(
                "web push endpoint returned {status}: {text}"
            )));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// FCM (Firebase Cloud Messaging HTTP v1)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ServiceAccount {
    project_id: String,
    client_email: String,
    private_key: String,
    #[serde(default = "default_token_uri")]
    token_uri: String,
}

fn default_token_uri() -> String {
    "https://oauth2.googleapis.com/token".into()
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    expires_in: u64,
}

pub struct FcmProvider {
    project_id: String,
    client_email: String,
    private_key_pem: String,
    token_uri: String,
    /// Cached OAuth2 bearer token, refreshed a minute before it actually
    /// expires — avoids signing a fresh service-account JWT and hitting
    /// Google's token endpoint on every single push.
    cached_token: RwLock<Option<(String, Instant)>>,
}

impl FcmProvider {
    pub fn new(service_account_json: &str) -> Result<Self, PushError> {
        let sa: ServiceAccount = serde_json::from_str(service_account_json).map_err(|e| {
            PushError::NotConfigured(format!("invalid FCM service account JSON: {e}"))
        })?;
        Ok(FcmProvider {
            project_id: sa.project_id,
            client_email: sa.client_email,
            private_key_pem: sa.private_key,
            token_uri: sa.token_uri,
            cached_token: RwLock::new(None),
        })
    }

    async fn access_token(&self) -> Result<String, PushError> {
        {
            let cached = self.cached_token.read().await;
            if let Some((token, expiry)) = cached.as_ref() {
                if *expiry > Instant::now() {
                    return Ok(token.clone());
                }
            }
        }

        let now = chrono::Utc::now().timestamp();
        let claims = json!({
            "iss": self.client_email,
            "scope": "https://www.googleapis.com/auth/firebase.messaging",
            "aud": self.token_uri,
            "iat": now,
            "exp": now + 3600,
        });
        let key = EncodingKey::from_rsa_pem(self.private_key_pem.as_bytes())
            .map_err(|e| PushError::NotConfigured(format!("invalid FCM private key: {e}")))?;
        let jwt = jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &key)
            .map_err(|e| PushError::Delivery(format!("failed to sign FCM JWT: {e}")))?;

        let resp = http_client()
            .post(&self.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", jwt.as_str()),
            ])
            .send()
            .await
            .map_err(|e| PushError::Delivery(e.to_string()))?;
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(PushError::Delivery(format!(
                "FCM token exchange failed: {text}"
            )));
        }
        let token_resp: OAuthTokenResponse = resp
            .json()
            .await
            .map_err(|e| PushError::Delivery(format!("malformed FCM token response: {e}")))?;
        let expiry = Instant::now() + Duration::from_secs(token_resp.expires_in.saturating_sub(60));
        *self.cached_token.write().await = Some((token_resp.access_token.clone(), expiry));
        Ok(token_resp.access_token)
    }
}

/// FCM's HTTP v1 API answers a dead registration token with 404 and an
/// `errors[].errorCode` / `error.details[].errorCode` of `UNREGISTERED`.
/// Matched on the raw body text rather than fully parsing FCM's error
/// envelope: the field is nested inside a `details` array typed by
/// `@type`, and a substring match is exactly as reliable for a value that
/// can only ever be the literal enum member.
fn is_fcm_dead(status: reqwest::StatusCode, body: &str) -> bool {
    status.as_u16() == 404 || body.contains("UNREGISTERED")
}

#[async_trait]
impl PushProvider for FcmProvider {
    async fn send(
        &self,
        token: &str,
        _platform: &str,
        payload: &PushPayload,
    ) -> Result<(), PushError> {
        let access_token = self.access_token().await?;
        let url = format!(
            "https://fcm.googleapis.com/v1/projects/{}/messages:send",
            self.project_id
        );
        // FCM's `data` payload is a flat map<string, string>; anything
        // that isn't already a JSON string is re-serialized to one so a
        // caller can still pass numbers/objects/booleans in `data`.
        let data: BTreeMap<String, String> = match &payload.data {
            Value::Object(map) => map
                .iter()
                .map(|(k, v)| {
                    let s = match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    (k.clone(), s)
                })
                .collect(),
            _ => BTreeMap::new(),
        };
        let body = json!({
            "message": {
                "token": token,
                "notification": { "title": payload.title, "body": payload.body },
                "data": data,
            }
        });

        let resp = http_client()
            .post(&url)
            .bearer_auth(access_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| PushError::Delivery(e.to_string()))?;
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let text = resp.text().await.unwrap_or_default();
        if is_fcm_dead(status, &text) {
            return Err(PushError::InvalidToken);
        }
        Err(PushError::Delivery(format!(
            "FCM send failed ({status}): {text}"
        )))
    }
}

// ---------------------------------------------------------------------------
// APNs (Apple Push Notification service, HTTP/2 provider API)
// ---------------------------------------------------------------------------

pub struct ApnsProvider {
    key_id: String,
    team_id: String,
    bundle_id: String,
    encoding_key: EncodingKey,
    host: &'static str,
    /// Apple allows reusing one provider JWT for up to an hour; cached the
    /// same way [`FcmProvider::cached_token`] caches its OAuth2 token.
    cached_jwt: RwLock<Option<(String, Instant)>>,
}

impl ApnsProvider {
    /// `p8_key_pem` is the raw contents of the `.p8` file downloaded from
    /// the Apple Developer portal (already PEM, PKCS#8 EC private key).
    pub fn new(
        p8_key_pem: &str,
        key_id: &str,
        team_id: &str,
        bundle_id: &str,
        production: bool,
    ) -> Result<Self, PushError> {
        let encoding_key = EncodingKey::from_ec_pem(p8_key_pem.as_bytes())
            .map_err(|e| PushError::NotConfigured(format!("invalid APNs .p8 key: {e}")))?;
        Ok(ApnsProvider {
            key_id: key_id.to_string(),
            team_id: team_id.to_string(),
            bundle_id: bundle_id.to_string(),
            encoding_key,
            host: if production {
                "api.push.apple.com"
            } else {
                "api.sandbox.push.apple.com"
            },
            cached_jwt: RwLock::new(None),
        })
    }

    async fn provider_token(&self) -> Result<String, PushError> {
        {
            let cached = self.cached_jwt.read().await;
            if let Some((jwt, expiry)) = cached.as_ref() {
                if *expiry > Instant::now() {
                    return Ok(jwt.clone());
                }
            }
        }
        let claims = json!({ "iss": self.team_id, "iat": chrono::Utc::now().timestamp() });
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.key_id.clone());
        let jwt = jsonwebtoken::encode(&header, &claims, &self.encoding_key)
            .map_err(|e| PushError::Delivery(format!("failed to sign APNs JWT: {e}")))?;
        *self.cached_jwt.write().await =
            Some((jwt.clone(), Instant::now() + Duration::from_secs(1800)));
        Ok(jwt)
    }
}

/// APNs reports a dead device token with `410 Unregistered`, and a token
/// that was never valid to begin with with `400 BadDeviceToken` — both
/// mean the same thing to a caller deciding whether to keep retrying.
fn is_apns_dead(status: reqwest::StatusCode, body: &str) -> bool {
    status.as_u16() == 410 || body.contains("Unregistered") || body.contains("BadDeviceToken")
}

#[async_trait]
impl PushProvider for ApnsProvider {
    async fn send(
        &self,
        token: &str,
        _platform: &str,
        payload: &PushPayload,
    ) -> Result<(), PushError> {
        let jwt = self.provider_token().await?;
        let url = format!("https://{}/3/device/{token}", self.host);
        let mut body = match &payload.data {
            Value::Object(map) => map.clone(),
            _ => Map::new(),
        };
        body.insert(
            "aps".into(),
            json!({ "alert": { "title": payload.title, "body": payload.body } }),
        );

        let resp = http_client()
            .post(&url)
            .bearer_auth(jwt)
            .header("apns-topic", &self.bundle_id)
            .header("apns-push-type", "alert")
            .json(&Value::Object(body))
            .send()
            .await
            .map_err(|e| PushError::Delivery(e.to_string()))?;
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let text = resp.text().await.unwrap_or_default();
        if is_apns_dead(status, &text) {
            return Err(PushError::InvalidToken);
        }
        Err(PushError::Delivery(format!(
            "APNs send failed ({status}): {text}"
        )))
    }
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Built once per call from the live `settings.push`, PocketBase/mailer
/// style — see the module doc comment. Each backend is `None` when its
/// `enabled` flag is off or its own settings fail to parse (an invalid
/// FCM service-account JSON, say), so a misconfigured backend degrades to
/// [`PushError::NotConfigured`] rather than panicking the request that
/// happens to hit it.
#[derive(Clone)]
pub struct PushService {
    web: Option<Arc<WebPushProvider>>,
    fcm: Option<Arc<FcmProvider>>,
    apns: Option<Arc<ApnsProvider>>,
}

impl PushService {
    pub fn from_settings(settings: &Push) -> Self {
        let web = configured_web(&settings.vapid);
        let fcm = configured_fcm(&settings.fcm);
        let apns = configured_apns(&settings.apns);
        PushService { web, fcm, apns }
    }

    /// Send to one subscription's `token`, dispatching on `platform`
    /// (`"web"` / `"android"` / `"ios"` — `_push_subscriptions.platform`).
    pub async fn send(
        &self,
        platform: &str,
        token: &str,
        payload: &PushPayload,
    ) -> Result<(), PushError> {
        match platform {
            "web" => match &self.web {
                Some(p) => p.send(token, platform, payload).await,
                None => Err(PushError::NotConfigured(
                    "Web Push (VAPID) is not configured".into(),
                )),
            },
            "android" => match &self.fcm {
                Some(p) => p.send(token, platform, payload).await,
                None => Err(PushError::NotConfigured("FCM is not configured".into())),
            },
            "ios" => match &self.apns {
                Some(p) => p.send(token, platform, payload).await,
                None => Err(PushError::NotConfigured("APNs is not configured".into())),
            },
            other => Err(PushError::NotConfigured(format!(
                "unknown push platform '{other}'"
            ))),
        }
    }
}

fn configured_web(vapid: &VapidConfig) -> Option<Arc<WebPushProvider>> {
    if !vapid.enabled || vapid.private_key.is_empty() || vapid.subject.is_empty() {
        return None;
    }
    match WebPushProvider::new(&vapid.private_key, &vapid.subject) {
        Ok(p) => Some(Arc::new(p)),
        Err(e) => {
            tracing::warn!(error = %e, "invalid VAPID settings, Web Push disabled");
            None
        }
    }
}

fn configured_fcm(fcm: &FcmConfig) -> Option<Arc<FcmProvider>> {
    if !fcm.enabled || fcm.service_account_json.is_empty() {
        return None;
    }
    match FcmProvider::new(&fcm.service_account_json) {
        Ok(p) => Some(Arc::new(p)),
        Err(e) => {
            tracing::warn!(error = %e, "invalid FCM settings, FCM disabled");
            None
        }
    }
}

fn configured_apns(apns: &ApnsConfig) -> Option<Arc<ApnsProvider>> {
    if !apns.enabled || apns.key.is_empty() {
        return None;
    }
    match ApnsProvider::new(
        &apns.key,
        &apns.key_id,
        &apns.team_id,
        &apns.bundle_id,
        apns.production,
    ) {
        Ok(p) => Some(Arc::new(p)),
        Err(e) => {
            tracing::warn!(error = %e, "invalid APNs settings, APNs disabled");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Record-event triggers (`settings.push.triggers`)
// ---------------------------------------------------------------------------

/// Bind the reactive trigger dispatch. Called once from
/// [`crate::app::App::bootstrap`]. Deliberately untagged, same reasoning
/// as `crate::webhooks::bind_hooks`: a trigger, by definition, watches
/// *some other* collection (the one named in its own `collection` field),
/// so this has to see every collection's writes, not just one system
/// collection's.
pub fn bind_hooks(app: &App) {
    let a = app.clone();
    app.hooks()
        .on_record_after_create_success
        .bind(Handler::new(move |e: &mut RecordEvent| {
            dispatch_after_success(&a, "create", e);
            e.next()
        }));
    let a = app.clone();
    app.hooks()
        .on_record_after_update_success
        .bind(Handler::new(move |e: &mut RecordEvent| {
            dispatch_after_success(&a, "update", e);
            e.next()
        }));
    let a = app.clone();
    app.hooks()
        .on_record_after_delete_success
        .bind(Handler::new(move |e: &mut RecordEvent| {
            dispatch_after_success(&a, "delete", e);
            e.next()
        }));
}

/// Snapshot what the hook needs from `e` and spawn the rest of the work,
/// same "never delay the record write's response" reasoning as
/// `crate::webhooks::dispatch_after_success`.
fn dispatch_after_success(app: &App, event: &str, e: &RecordEvent) {
    // `_push_subscriptions` writes themselves never fire a push — nothing
    // subscribes to changes in the subscription list itself, and it would
    // be a pointless self-referential dispatch if a trigger ever did name
    // it.
    if e.collection.name == "_push_subscriptions" || e.collection.id == "_push_subscriptions" {
        return;
    }
    let app = app.clone();
    let event = event.to_string();
    let collection_name = e.collection.name.clone();
    let record = e.record.to_json(Default::default());
    tokio::spawn(async move {
        dispatch_trigger(app, event, collection_name, record).await;
    });
}

async fn dispatch_trigger(app: App, event: String, collection_name: String, record: Value) {
    let triggers: Vec<_> = app
        .settings()
        .push
        .triggers
        .iter()
        .filter(|t| {
            t.enabled
                && t.collection == collection_name
                && t.events.split(',').map(str::trim).any(|k| k == event)
        })
        .cloned()
        .collect();
    if triggers.is_empty() {
        return;
    }

    let service = PushService::from_settings(&app.settings().push);
    for trigger in triggers {
        let title = render_template(&trigger.title, &record);
        let body = render_template(&trigger.body, &record);
        let payload = PushPayload {
            title,
            body,
            data: json!({ "collection": collection_name, "event": event }),
        };

        let rows = if trigger.target_field.is_empty() {
            app.db()
                .query(
                    r#"SELECT "id", "platform", "token" FROM "_push_subscriptions" WHERE "enabled" = 1"#,
                    &[],
                )
                .await
        } else {
            let Some(target) = record.get(&trigger.target_field).and_then(Value::as_str) else {
                continue;
            };
            app.db()
                .query(
                    r#"SELECT "id", "platform", "token" FROM "_push_subscriptions" WHERE "enabled" = 1 AND "recordRef" = $1"#,
                    &[Sql::from(target.to_string())],
                )
                .await
        };
        let rows = match rows {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = %e, "failed to load push subscriptions for trigger dispatch");
                continue;
            }
        };
        for row in &rows {
            let (Some(id), Some(platform), Some(token)) = (
                row.get_str("id"),
                row.get_str("platform"),
                row.get_str("token"),
            ) else {
                continue;
            };
            // Spawned per subscription so one slow provider call never
            // blocks the next subscription's delivery — same fan-out
            // shape as `crate::webhooks::dispatch`.
            tokio::spawn(deliver_and_cleanup(
                app.clone(),
                service.clone(),
                id.to_string(),
                platform.to_string(),
                token.to_string(),
                payload.clone(),
            ));
        }
    }
}

async fn deliver_and_cleanup(
    app: App,
    service: PushService,
    id: String,
    platform: String,
    token: String,
    payload: PushPayload,
) {
    match service.send(&platform, &token, &payload).await {
        Ok(()) => {}
        Err(PushError::InvalidToken) => disable_subscription(&app, &id).await,
        Err(e) => tracing::warn!(error = %e, id = %id, "push trigger delivery failed"),
    }
}

/// Disable a dead subscription with a raw `UPDATE`, not the record API —
/// same reasoning as `crate::webhooks::deliver`'s status write-back:
/// going through `records::update` would run full field validation and
/// re-fire hooks for a write that only ever touches one system column.
pub async fn disable_subscription(app: &App, id: &str) {
    if let Err(e) = app
        .db()
        .execute(
            r#"UPDATE "_push_subscriptions" SET "enabled" = 0 WHERE "id" = $1"#,
            &[Sql::from(id.to_string())],
        )
        .await
    {
        tracing::warn!(error = %e, id = %id, "failed to disable dead push subscription");
    }
}

/// Minimal `{{field}}` substitution against a record's JSON. Not a real
/// template engine (no conditionals/loops) — `_webhooks` doesn't have
/// templates at all (it just forwards the raw record), and a push
/// notification's title/body need *some* per-record interpolation to be
/// useful at all, so this is the smallest thing that provides it. An
/// unresolvable `{{field}}` (missing, or not a string/number/bool)
/// renders as an empty string rather than leaving the literal
/// placeholder in a notification a real user sees.
fn render_template(template: &str, record: &Value) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str("{{");
            rest = after;
            break;
        };
        let field = after[..end].trim();
        let value = match record.get(field) {
            Some(Value::String(s)) => s.clone(),
            Some(v @ (Value::Number(_) | Value::Bool(_))) => v.to_string(),
            _ => String::new(),
        };
        out.push_str(&value);
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cratebase_core::settings::PushTrigger;

    fn payload() -> PushPayload {
        PushPayload {
            title: "hi".into(),
            body: "there".into(),
            data: json!({}),
        }
    }

    // ---- VAPID / Web Push construction -----------------------------------

    /// A freshly generated P-256 key pair, raw base64url — the same shape
    /// `web-push generate-vapid-keys` emits and [`VapidConfig::private_key`]
    /// stores.
    fn test_vapid_key() -> String {
        let kp = ES256KeyPair::generate();
        URL_SAFE_NO_PAD.encode(kp.to_bytes())
    }

    #[test]
    fn web_push_provider_accepts_a_valid_vapid_key() {
        let key = test_vapid_key();
        assert!(WebPushProvider::new(&key, "mailto:ops@example.com").is_ok());
    }

    #[test]
    fn web_push_provider_rejects_a_malformed_vapid_key() {
        let result = WebPushProvider::new("not-base64!!", "mailto:ops@example.com");
        assert!(matches!(result, Err(PushError::NotConfigured(_))));
    }

    #[tokio::test]
    async fn web_push_send_builds_a_valid_encrypted_request_from_a_json_token() {
        // Exercises the full construction path (JSON token parsing, ECDH
        // public key + auth secret decoding, RFC 8291 payload encryption,
        // RFC 8292 VAPID JWT signing, http::Request -> reqwest::Request)
        // against a subscriber key pair generated in-test, stopping right
        // before the network call — proof the whole non-network half of
        // the Web Push path is correct without depending on a live push
        // service.
        let vapid_key = test_vapid_key();
        let provider = WebPushProvider::new(&vapid_key, "mailto:ops@example.com").unwrap();

        let ua_secret = p256::SecretKey::random(&mut rand::rngs::OsRng);
        let ua_public = ua_secret.public_key();
        let p256dh = URL_SAFE_NO_PAD.encode(ua_public.to_sec1_bytes());
        let auth_bytes: [u8; 16] = rand::random();
        let auth = URL_SAFE_NO_PAD.encode(auth_bytes);

        let token = serde_json::to_string(&json!({
            "endpoint": "https://push.example.com/abc123",
            "p256dh": p256dh,
            "auth": auth,
        }))
        .unwrap();

        // `send` will fail on the network call itself (no live endpoint in
        // a unit test) — what this test asserts is that it gets *past*
        // every construction step first, i.e. the failure is a transport
        // error, never a malformed-subscription/encoding error.
        let err = provider.send(&token, "web", &payload()).await.unwrap_err();
        match err {
            PushError::Delivery(msg) => {
                assert!(
                    !msg.contains("malformed") && !msg.contains("invalid"),
                    "expected a transport failure past construction, got: {msg}"
                );
            }
            other => panic!("expected a transport-stage Delivery error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn web_push_send_rejects_a_malformed_token() {
        let vapid_key = test_vapid_key();
        let provider = WebPushProvider::new(&vapid_key, "mailto:ops@example.com").unwrap();
        let err = provider
            .send("not json", "web", &payload())
            .await
            .unwrap_err();
        assert!(matches!(err, PushError::Delivery(_)));
    }

    // ---- Dead-token detection ----------------------------------------

    #[test]
    fn web_push_dead_token_statuses() {
        assert!(is_web_push_dead(reqwest::StatusCode::NOT_FOUND));
        assert!(is_web_push_dead(reqwest::StatusCode::GONE));
        assert!(!is_web_push_dead(reqwest::StatusCode::OK));
        assert!(!is_web_push_dead(reqwest::StatusCode::BAD_REQUEST));
    }

    #[test]
    fn fcm_dead_token_detection() {
        assert!(is_fcm_dead(
            reqwest::StatusCode::NOT_FOUND,
            r#"{"error":{"status":"NOT_FOUND"}}"#
        ));
        assert!(is_fcm_dead(
            reqwest::StatusCode::BAD_REQUEST,
            r#"{"error":{"details":[{"errorCode":"UNREGISTERED"}]}}"#
        ));
        assert!(!is_fcm_dead(
            reqwest::StatusCode::BAD_REQUEST,
            r#"{"error":{"details":[{"errorCode":"INVALID_ARGUMENT"}]}}"#
        ));
        assert!(!is_fcm_dead(reqwest::StatusCode::OK, "{}"));
    }

    #[test]
    fn apns_dead_token_detection() {
        assert!(is_apns_dead(
            reqwest::StatusCode::GONE,
            r#"{"reason":"Unregistered"}"#
        ));
        assert!(is_apns_dead(
            reqwest::StatusCode::BAD_REQUEST,
            r#"{"reason":"BadDeviceToken"}"#
        ));
        assert!(!is_apns_dead(
            reqwest::StatusCode::BAD_REQUEST,
            r#"{"reason":"BadTopic"}"#
        ));
        assert!(!is_apns_dead(reqwest::StatusCode::OK, "{}"));
    }

    // ---- FCM / APNs JWT construction ------------------------------------

    const TEST_RSA_KEY: &str = include_str!("../testdata/test_rsa_key.pem");
    const TEST_EC_KEY: &str = include_str!("../testdata/test_ec_key.pem");

    #[test]
    fn fcm_provider_parses_a_service_account_and_signs_a_jwt() {
        let sa_json = json!({
            "project_id": "demo-project",
            "client_email": "svc@demo-project.iam.gserviceaccount.com",
            "private_key": TEST_RSA_KEY,
        })
        .to_string();
        let provider = FcmProvider::new(&sa_json).unwrap();
        assert_eq!(provider.project_id, "demo-project");
        assert_eq!(provider.token_uri, "https://oauth2.googleapis.com/token");

        // Sign the same OAuth2 JWT `access_token` would, proving the RSA
        // key decodes and jsonwebtoken accepts it, without an actual
        // network round trip to Google's token endpoint.
        let claims = json!({
            "iss": provider.client_email,
            "scope": "https://www.googleapis.com/auth/firebase.messaging",
            "aud": provider.token_uri,
            "iat": 0,
            "exp": 3600,
        });
        let key = EncodingKey::from_rsa_pem(provider.private_key_pem.as_bytes()).unwrap();
        let jwt = jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &key).unwrap();
        assert_eq!(jwt.split('.').count(), 3);
    }

    #[test]
    fn fcm_provider_rejects_invalid_service_account_json() {
        let result = FcmProvider::new("not json");
        assert!(matches!(result, Err(PushError::NotConfigured(_))));
    }

    #[tokio::test]
    async fn apns_provider_signs_a_provider_jwt_and_caches_it() {
        let provider = ApnsProvider::new(
            TEST_EC_KEY,
            "ABC123DEFG",
            "TEAM123456",
            "com.example.app",
            false,
        )
        .unwrap();
        assert_eq!(provider.host, "api.sandbox.push.apple.com");

        let jwt1 = provider.provider_token().await.unwrap();
        assert_eq!(jwt1.split('.').count(), 3);
        // Cached: a second call within the cache window returns the exact
        // same token rather than signing a new one.
        let jwt2 = provider.provider_token().await.unwrap();
        assert_eq!(jwt1, jwt2);
    }

    #[test]
    fn apns_provider_uses_production_host_when_configured() {
        let provider = ApnsProvider::new(
            TEST_EC_KEY,
            "ABC123DEFG",
            "TEAM123456",
            "com.example.app",
            true,
        )
        .unwrap();
        assert_eq!(provider.host, "api.push.apple.com");
    }

    #[test]
    fn apns_provider_rejects_an_invalid_key() {
        let result = ApnsProvider::new("not a key", "kid", "team", "bundle", false);
        assert!(matches!(result, Err(PushError::NotConfigured(_))));
    }

    // ---- Trigger template rendering --------------------------------------

    #[test]
    fn render_template_substitutes_known_fields() {
        let record = json!({ "title": "Widget", "count": 3, "archived": false });
        assert_eq!(
            render_template("New: {{title}} ({{count}})", &record),
            "New: Widget (3)"
        );
        assert_eq!(
            render_template("archived={{archived}}", &record),
            "archived=false"
        );
    }

    #[test]
    fn render_template_blanks_unknown_or_non_scalar_fields() {
        let record = json!({ "nested": { "a": 1 } });
        assert_eq!(
            render_template("[{{missing}}][{{nested}}]", &record),
            "[][]"
        );
    }

    // ---- PushService platform selection ----------------------------------

    #[tokio::test]
    async fn push_service_reports_not_configured_when_no_backend_is_enabled() {
        let service = PushService::from_settings(&Push::default());
        for platform in ["web", "android", "ios"] {
            let err = service
                .send(platform, "token", &payload())
                .await
                .unwrap_err();
            assert!(matches!(err, PushError::NotConfigured(_)));
        }
    }

    #[tokio::test]
    async fn push_service_reports_not_configured_for_an_unknown_platform() {
        let service = PushService::from_settings(&Push::default());
        let err = service
            .send("windows-phone", "token", &payload())
            .await
            .unwrap_err();
        assert!(matches!(err, PushError::NotConfigured(_)));
    }

    #[test]
    fn push_service_configures_web_push_when_vapid_is_enabled() {
        let mut settings = Push::default();
        settings.vapid.enabled = true;
        settings.vapid.private_key = test_vapid_key();
        settings.vapid.subject = "mailto:ops@example.com".into();
        let service = PushService::from_settings(&settings);
        assert!(service.web.is_some());
        assert!(service.fcm.is_none());
        assert!(service.apns.is_none());
    }

    #[test]
    fn trigger_with_empty_events_never_matches() {
        let trigger = PushTrigger {
            enabled: true,
            collection: "posts".into(),
            events: "".into(),
            title: "{{title}}".into(),
            body: "".into(),
            target_field: "".into(),
        };
        assert!(!trigger
            .events
            .split(',')
            .map(str::trim)
            .any(|k| k == "create"));
    }
}
