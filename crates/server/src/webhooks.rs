//! Outgoing webhooks (`_webhooks`): POST a JSON payload to a configured
//! URL whenever a record event fires on a chosen collection — entirely
//! from the dashboard or the generic Records API, no Rust code, no
//! redeploy.
//!
//! # Trust boundary
//!
//! `_webhooks` is superuser-only end to end (every rule stays at
//! `Collection::new`'s default `None` — see
//! `cratebase_core::Collection::default_system_collections`), exactly
//! like `_cron_jobs`: a webhook's `url`/`secret` are operator-configured
//! integration points, not something a non-superuser record should ever
//! reach.
//!
//! # No in-process registry, unlike `_cron_jobs`
//!
//! [`crate::cron_jobs`] keeps an in-process scheduler ([`crate::cron::CronService`])
//! that has to be told about every `_cron_jobs` row up front and kept in
//! sync as rows change. Webhooks have no equivalent standing state: every
//! dispatch already runs a fresh `SELECT` against `_webhooks` for the
//! collection that just changed, so there is nothing to load at boot and
//! nothing to resync on a dashboard edit — the next write to that
//! collection simply sees the new row. [`bind_hooks`] therefore only
//! registers handlers; there is no `sync_all`.
//!
//! # Why the dispatch hook is untagged
//!
//! Every other hook in this codebase that only cares about one system
//! collection is tagged to it (see `_cron_jobs`'s `with_tags([COLLECTION])`).
//! A webhook, by definition, watches *some other* collection — the one
//! named in its own `collectionRef` — so the reactive
//! `on_record_after_{create,update,delete}_success` handlers below are
//! bound with no tags at all, which makes them run for every collection's
//! writes (`Hook::trigger` only tag-filters handlers that opted in). Each
//! invocation looks up matching `_webhooks` rows by the triggering
//! collection's name/id at dispatch time.
//!
//! # Breaking the recursion
//!
//! A write to `_webhooks` itself must never trigger a webhook dispatch
//! for `_webhooks` — the untagged handler above would otherwise fire
//! on every webhook status write-back, forever. [`dispatch_after_success`]
//! simply skips events whose own collection is `_webhooks`; a `_webhooks`
//! row can never be selected as a dispatch target in the first place
//! (`default_system_collections` never gives it PocketBase's "no
//! equivalent" caveat away — see its own comment).
//!
//! # Why the status write-back bypasses the record API
//!
//! [`deliver`] writes `lastTriggeredAt`/`lastStatus`/`lastMessage` back
//! with a raw `UPDATE`, not `records::update` — same reasoning as
//! `_cron_jobs`'s `run_custom_job`: going through the record API would
//! re-fire the very hook that dispatches webhooks (harmless here since
//! `_webhooks` is never a dispatch target, but still pointless), and it
//! would run full field validation for a write that only ever touches
//! three system-owned columns.
//!
//! # Why delivery is fire-and-forget
//!
//! [`dispatch_after_success`] spawns one `tokio::spawn` per triggering
//! record event, and the lookup+delivery work inside it spawns one more
//! `tokio::spawn` per matching webhook row, each POSTing with a 5 second
//! timeout via `reqwest`. A slow or dead target therefore never blocks
//! the record write's HTTP response, and one slow target never blocks
//! another target's delivery either.
//!
//! # SSRF hardening
//!
//! A webhook's `url` is operator-configured, not something this process
//! chose — but the process is the one making the outbound request, so a
//! superuser (or anyone who can edit a `_webhooks` row through whatever
//! trust boundary is guarding it upstream) could otherwise point `url`
//! at `169.254.169.254` and read a cloud provider's instance-metadata
//! credentials back out through `lastMessage`, or at `127.0.0.1:<port>`
//! to reach a service that only trusts localhost callers. This is a
//! *server-side request forgery* surface independent of the
//! already-documented superuser trust boundary above, so [`deliver`]
//! resolves `url`'s host with [`validate_webhook_url`] and refuses to
//! send if every resolved address isn't a public one, or if the scheme
//! isn't `http`/`https`. The shared `reqwest::Client` also disables
//! redirect-following ([`reqwest::redirect::Policy::none`]): otherwise a
//! validated public URL could 302 to a blocked address and the check
//! above would never see it.

use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;

use cratebase_core::codes::REQUIRED;
use cratebase_core::{AppError, DateTime, FieldError};
use cratebase_db::engine::{Executor, Sql};

use crate::app::App;
use crate::events::RecordEvent;
use crate::hooks::{Event, Handler};

const COLLECTION: &str = "_webhooks";
const EVENT_KINDS: [&str; 3] = ["create", "update", "delete"];

/// Bind the validating and reactive hooks. Called once from
/// [`crate::app::App::bootstrap`].
pub fn bind_hooks(app: &App) {
    // Reject a malformed `events` value before the row is even written,
    // rather than accepting it and silently never dispatching.
    app.hooks().on_record_create.bind(
        Handler::new(|e: &mut RecordEvent| {
            Box::pin(async move {
                let events = e
                    .record
                    .get("events")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                validate_events(&events)?;
                e.next().await
            })
        })
        .with_tags([COLLECTION]),
    );
    app.hooks().on_record_update.bind(
        Handler::new(|e: &mut RecordEvent| {
            Box::pin(async move {
                let events = e
                    .record
                    .get("events")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                validate_events(&events)?;
                e.next().await
            })
        })
        .with_tags([COLLECTION]),
    );

    // Reactive dispatch. Deliberately untagged — see the module doc
    // comment for why this needs to watch every collection, not just
    // `_webhooks`. Each handler still calls `e.next()`: this is a
    // notification, not a gate, and every other handler that might be
    // registered on the same hook (e.g. `_cron_jobs`'/`_teams`' own
    // reactive sync) must still run after it.
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
/// so a slow `_webhooks` lookup or a slow delivery never delays the
/// record write's response.
fn dispatch_after_success(app: &App, event: &str, e: &RecordEvent) {
    // Never dispatch for writes to `_webhooks` itself — see the module
    // doc comment.
    if e.collection.name == COLLECTION || e.collection.id == COLLECTION {
        return;
    }
    let app = app.clone();
    let event = event.to_string();
    let collection_name = e.collection.name.clone();
    let collection_id = e.collection.id.clone();
    let record = e.record.to_json(Default::default());
    tokio::spawn(async move {
        dispatch(app, event, collection_name, collection_id, record).await;
    });
}

/// Load every enabled `_webhooks` row targeting `collection_name`/
/// `collection_id` and subscribed to `event`, and spawn one delivery per
/// match.
async fn dispatch(
    app: App,
    event: String,
    collection_name: String,
    collection_id: String,
    record: Value,
) {
    let rows = match app
        .db()
        .query(
            &format!(
                r#"SELECT * FROM "{COLLECTION}" WHERE "enabled" = 1 AND ("collectionRef" = $1 OR "collectionRef" = $2)"#
            ),
            &[Sql::from(collection_name.clone()), Sql::from(collection_id)],
        )
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "failed to load webhooks for dispatch");
            return;
        }
    };
    // Note: no shared payload here — each webhook row's `url` may need
    // a different shape (see `format_payload`), so the payload is built
    // per row below, after `url` is known.
    for row in &rows {
        let Some(id) = row.get_str("id") else {
            continue;
        };
        let events = row.get_str("events").unwrap_or_default();
        if !events.split(',').map(str::trim).any(|k| k == event) {
            continue;
        }
        let url = row.get_str("url").unwrap_or_default().to_string();
        if url.is_empty() {
            continue;
        }
        let secret = row
            .get_str("secret")
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let record_id = id.to_string();
        let app = app.clone();
        let payload = format_payload(&url, &event, &collection_name, &record);
        tokio::spawn(deliver(app, record_id, url, secret, payload));
    }
}

/// Reshapes the generic `{event, collection, record}` notification into
/// the shape a chat-app incoming webhook expects, detected from `url`'s
/// own host — not a `_webhooks.format` column, so pointing a webhook at
/// Slack or Discord "just works" with no schema change and no extra
/// dashboard field to configure. Anything that doesn't match a known
/// chat-webhook host keeps today's generic shape unchanged, so this is
/// purely additive for every existing webhook.
///
/// Both known targets get the same human-readable text (event, target
/// collection, and the record as pretty JSON in a code fence) under the
/// field name each platform expects: Slack's incoming-webhook payload is
/// `{"text": "..."}`, Discord's is `{"content": "..."}`.
fn format_payload(url: &str, event: &str, collection: &str, record: &Value) -> Value {
    let host = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase));
    let text = || {
        format!(
            "*{event}* on `{collection}`\n```{}```",
            serde_json::to_string_pretty(record).unwrap_or_default()
        )
    };
    match host.as_deref() {
        Some("hooks.slack.com") => json!({ "text": text() }),
        // Discord's webhook path is always `/api/webhooks/<id>/<token>`;
        // gate on that too so a *different* service merely hosted at
        // `discord.com` (unlikely, but this check is nearly free)
        // doesn't get mis-shaped.
        Some("discord.com") if url.contains("/api/webhooks") => json!({ "content": text() }),
        _ => json!({
            "event": event,
            "collection": collection,
            "record": record,
        }),
    }
}

/// POST `payload` to `url`, optionally signing the raw body with
/// HMAC-SHA256 in `X-Cratebase-Signature`, and write the outcome back
/// onto the triggering `_webhooks` row.
async fn deliver(app: App, record_id: String, url: String, secret: Option<String>, payload: Value) {
    static CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
        reqwest::Client::builder()
            // See the module doc's "SSRF hardening" section: without
            // this, a `validate_webhook_url`-approved public URL could
            // 302 to a blocked address and the delivery would follow it
            // there anyway.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("static client config is valid")
    });
    let client = &*CLIENT;

    let (status, message) = match validate_webhook_url(&url).await {
        Err(reason) => ("error".to_string(), reason),
        Ok(()) => {
            let body = serde_json::to_vec(&payload).unwrap_or_default();
            let mut builder = client
                .post(&url)
                .header("Content-Type", "application/json")
                .timeout(Duration::from_secs(5));
            if let Some(secret) = &secret {
                let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
                    .expect("HMAC accepts a key of any length");
                mac.update(&body);
                let signature = mac
                    .finalize()
                    .into_bytes()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>();
                builder = builder.header("X-Cratebase-Signature", signature);
            }
            match builder.body(body).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    (status.as_u16().to_string(), format!("delivered to {url}"))
                }
                Err(e) => ("error".to_string(), e.to_string()),
            }
        }
    };

    let now = DateTime::now().to_pb_string();
    if let Err(e) = app
        .db()
        .execute(
            &format!(
                r#"UPDATE "{COLLECTION}" SET "lastTriggeredAt" = $1, "lastStatus" = $2, "lastMessage" = $3 WHERE "id" = $4"#
            ),
            &[
                Sql::from(now),
                Sql::from(status),
                Sql::from(message),
                Sql::from(record_id.clone()),
            ],
        )
        .await
    {
        tracing::warn!(error = %e, record_id = %record_id, "failed to record a webhook's delivery result");
    }
}

/// A non-empty, comma-separated subset of `create`, `update`, `delete`.
fn validate_events(events: &str) -> Result<(), AppError> {
    let trimmed = events.trim();
    if trimmed.is_empty() {
        return Err(AppError::validation(
            "An error occurred while validating the submitted data.",
            [(
                "events".to_string(),
                FieldError::new(REQUIRED, "Cannot be blank."),
            )],
        ));
    }
    for part in trimmed.split(',') {
        let part = part.trim();
        if !EVENT_KINDS.contains(&part) {
            return Err(AppError::validation(
                "An error occurred while validating the submitted data.",
                [(
                    "events".to_string(),
                    FieldError::new(
                        "validation_invalid_format",
                        format!(
                            "\"{part}\" is not a valid event; must be a comma-separated subset of create, update, delete."
                        ),
                    ),
                )],
            ));
        }
    }
    Ok(())
}

/// Reject `url` unless its scheme is `http`/`https` and every address it
/// resolves to is a public, routable address — see the module doc's
/// "SSRF hardening" section. Resolution happens here (not left to
/// `reqwest`) specifically so a DNS name that resolves to a private or
/// metadata address is caught before any socket is opened.
async fn validate_webhook_url(url: &str) -> Result<(), String> {
    let parsed =
        reqwest::Url::parse(url).map_err(|e| format!("invalid webhook URL \"{url}\": {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(format!(
                "unsupported webhook URL scheme \"{other}\": only http and https are allowed"
            ))
        }
    }
    // `Url::host_str` keeps the `[...]` brackets an IPv6 literal needs
    // inside a URL (`http://[::1]/hook`); `lookup_host` wants the bare
    // address instead, and stripping brackets is a no-op for a plain
    // domain or an IPv4 literal.
    let host = parsed
        .host_str()
        .ok_or_else(|| format!("webhook URL \"{url}\" has no host"))?;
    let bare_host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    let port = parsed.port_or_known_default().unwrap_or(80);
    let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((bare_host, port))
        .await
        .map_err(|e| format!("failed to resolve webhook host \"{host}\": {e}"))?
        .collect();
    if addrs.is_empty() {
        return Err(format!(
            "webhook host \"{host}\" did not resolve to any address"
        ));
    }
    for addr in &addrs {
        if is_blocked_ip(addr.ip()) {
            return Err(format!(
                "webhook URL \"{url}\" resolves to a blocked address ({}): \
                 private, loopback, link-local, and cloud metadata addresses are not allowed",
                addr.ip()
            ));
        }
    }
    Ok(())
}

/// Loopback (127.0.0.0/8, ::1), RFC1918 private ranges (10/8, 172.16/12,
/// 192.168/16), link-local (169.254.0.0/16 — this is what makes
/// 169.254.169.254, the AWS/GCP/Azure metadata endpoint, unreachable),
/// IPv6 unique-local (fc00::/7) and link-local (fe80::/10), and the
/// unspecified/broadcast/multicast addresses. An IPv4-mapped IPv6
/// address (`::ffff:10.0.0.1`) is unwrapped and checked as its IPv4
/// form, so it cannot be used to smuggle a blocked v4 address past a
/// naive "is this an RFC1918 v4 literal" check.
fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_blocked_v4(mapped);
            }
            let segments = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (segments[0] & 0xfe00) == 0xfc00 // fc00::/7 unique-local
                || (segments[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
        }
    }
}

fn is_blocked_v4(v4: Ipv4Addr) -> bool {
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local() // 169.254.0.0/16, includes 169.254.169.254
        || v4.is_unspecified()
        || v4.is_broadcast()
        || v4.is_multicast()
        || v4.is_documentation()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slack_url_gets_slack_shaped_payload() {
        let record = json!({"id": "abc123", "title": "hi"});
        let payload = format_payload(
            "https://hooks.slack.com/services/T000/B000/XXXX",
            "create",
            "posts",
            &record,
        );
        let text = payload["text"].as_str().expect("text field");
        assert!(text.contains("create"), "{text}");
        assert!(text.contains("posts"), "{text}");
        assert!(text.contains("abc123"), "{text}");
        assert!(payload.get("event").is_none(), "not the generic shape");
        assert!(payload.get("content").is_none(), "not the discord shape");
    }

    #[test]
    fn discord_webhook_url_gets_discord_shaped_payload() {
        let record = json!({"id": "abc123", "title": "hi"});
        let payload = format_payload(
            "https://discord.com/api/webhooks/123456/token-abc",
            "update",
            "comments",
            &record,
        );
        let content = payload["content"].as_str().expect("content field");
        assert!(content.contains("update"), "{content}");
        assert!(content.contains("comments"), "{content}");
        assert!(content.contains("abc123"), "{content}");
        assert!(payload.get("event").is_none(), "not the generic shape");
        assert!(payload.get("text").is_none(), "not the slack shape");
    }

    #[test]
    fn discord_host_without_webhook_path_keeps_generic_shape() {
        // Guards the path check in `format_payload`: merely being on
        // `discord.com` isn't enough, it has to be the webhooks path.
        let record = json!({"id": "abc123"});
        let payload = format_payload(
            "https://discord.com/some/other/route",
            "create",
            "posts",
            &record,
        );
        assert_eq!(payload["event"], "create");
        assert_eq!(payload["collection"], "posts");
        assert_eq!(payload["record"], record);
    }

    #[test]
    fn other_urls_keep_generic_shape() {
        let record = json!({"id": "abc123", "title": "hi"});
        let payload = format_payload("https://example.com/hook", "delete", "posts", &record);
        assert_eq!(payload["event"], "delete");
        assert_eq!(payload["collection"], "posts");
        assert_eq!(payload["record"], record);
        assert!(payload.get("text").is_none());
        assert!(payload.get("content").is_none());
    }

    #[tokio::test]
    async fn public_url_allowed() {
        // A real, stable public DNS name: Cloudflare's resolver, plain
        // HTTPS, no private/metadata address anywhere in its A/AAAA set.
        assert!(validate_webhook_url("https://one.one.one.one/hook")
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn localhost_rejected() {
        let err = validate_webhook_url("http://localhost:8080/hook")
            .await
            .unwrap_err();
        assert!(err.contains("blocked address"), "{err}");
    }

    #[tokio::test]
    async fn loopback_literal_rejected() {
        let err = validate_webhook_url("http://127.0.0.1/hook")
            .await
            .unwrap_err();
        assert!(err.contains("blocked address"), "{err}");
    }

    #[tokio::test]
    async fn cloud_metadata_endpoint_rejected() {
        let err = validate_webhook_url("http://169.254.169.254/latest/meta-data/")
            .await
            .unwrap_err();
        assert!(err.contains("blocked address"), "{err}");
    }

    #[tokio::test]
    async fn link_local_range_rejected() {
        let err = validate_webhook_url("http://169.254.1.1/hook")
            .await
            .unwrap_err();
        assert!(err.contains("blocked address"), "{err}");
    }

    #[tokio::test]
    async fn rfc1918_10_rejected() {
        let err = validate_webhook_url("http://10.0.0.5/hook")
            .await
            .unwrap_err();
        assert!(err.contains("blocked address"), "{err}");
    }

    #[tokio::test]
    async fn rfc1918_172_16_rejected() {
        let err = validate_webhook_url("http://172.16.5.5/hook")
            .await
            .unwrap_err();
        assert!(err.contains("blocked address"), "{err}");
    }

    #[tokio::test]
    async fn rfc1918_192_168_rejected() {
        let err = validate_webhook_url("http://192.168.1.1/hook")
            .await
            .unwrap_err();
        assert!(err.contains("blocked address"), "{err}");
    }

    #[tokio::test]
    async fn ipv6_loopback_rejected() {
        let err = validate_webhook_url("http://[::1]/hook").await.unwrap_err();
        assert!(err.contains("blocked address"), "{err}");
    }

    #[tokio::test]
    async fn ipv6_unique_local_rejected() {
        let err = validate_webhook_url("http://[fc00::1]/hook")
            .await
            .unwrap_err();
        assert!(err.contains("blocked address"), "{err}");
    }

    #[tokio::test]
    async fn file_scheme_rejected() {
        let err = validate_webhook_url("file:///etc/passwd")
            .await
            .unwrap_err();
        assert!(err.contains("scheme"), "{err}");
    }

    #[tokio::test]
    async fn gopher_scheme_rejected() {
        let err = validate_webhook_url("gopher://127.0.0.1:70/_x")
            .await
            .unwrap_err();
        assert!(err.contains("scheme"), "{err}");
    }

    #[test]
    fn blocked_ip_ranges() {
        let blocked: &[&str] = &[
            "127.0.0.1",
            "169.254.169.254",
            "10.1.2.3",
            "172.31.255.255",
            "192.168.0.1",
            "0.0.0.0",
            "::1",
            "fc00::1",
            "fe80::1",
        ];
        for ip in blocked {
            assert!(is_blocked_ip(ip.parse().unwrap()), "{ip} should be blocked");
        }
        let allowed: &[&str] = &["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"];
        for ip in allowed {
            assert!(
                !is_blocked_ip(ip.parse().unwrap()),
                "{ip} should be allowed"
            );
        }
    }
}
