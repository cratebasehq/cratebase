//! Server-sent events: `GET /api/realtime` and `POST /api/realtime`.
//!
//! # The wire, as PocketBase speaks it
//!
//! A client opens `GET /api/realtime` and the server immediately writes
//! one frame whose SSE `id:` **is** the client id — the JS SDK reads it
//! off `MessageEvent.lastEventId`, not out of the JSON body, so the id
//! has to be in both places:
//!
//! ```text
//! id:2LmCCjhuUes8uv1OBpW2Lp7QTa7ODym75D2bQGG9
//! event:PB_CONNECT
//! data:{"clientId":"2LmCCjhuUes8uv1OBpW2Lp7QTa7ODym75D2bQGG9"}
//! ```
//!
//! The client then POSTs `{clientId, subscriptions}` to the same path.
//! Each subscription is a topic string, optionally carrying its own
//! options:
//!
//! ```text
//! posts
//! posts/RECORD_ID
//! posts?options=%7B%22query%22%3A%7B%22filter%22%3A%22views%20%3E%2010%22%7D%7D
//! ```
//!
//! That whole string, `?options=...` included, is echoed back as the SSE
//! `event:` name, because that is the key the SDK registered its listener
//! under. Treating it as an opaque token rather than re-serializing a
//! parsed form is what keeps the two ends agreeing.
//!
//! # Why access is decided in memory
//!
//! Measured against PocketBase v0.40.2 (see `docs/superpowers/specs`):
//!
//! * the **`listRule`** decides, not the `viewRule` — a collection with
//!   `listRule: ""` and `viewRule: null` delivers to anonymous clients,
//!   and the reverse delivers nothing;
//! * the rule is applied **per record**: with `listRule: 'title =
//!   "visible"'`, creating a "hidden" row sends nothing;
//! * and a `delete` of a rule-matching row **is** delivered, after the
//!   row is gone.
//!
//! That last one rules out the obvious implementation. `SELECT 1 FROM
//! posts WHERE id = ? AND (<rule>)` cannot answer for a record that no
//! longer exists. Instead the record snapshot goes through
//! [`cratebase_db::rules::check_rule_against_row`], the same two-strategy
//! check `createRule` uses for a row that does not exist *yet*: evaluate
//! in process, and when the expression needs the database — relation
//! traversal, `@collection.X` — stand the snapshot up as a one-row
//! derived table and run the compiled SQL against that. A deleted record
//! and an uncommitted one are the same problem.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use cratebase_core::{Collection, Record};
use cratebase_db::context::{CollectionResolver, RequestContext};
use futures::stream::Stream;
use serde::Deserialize;
use serde_json::{Map, Value};
use tokio::sync::mpsc;

use crate::app::App;
use crate::extract::{Auth, MaybeAuth};
use crate::http_error::{ApiError, ApiResult};

pub use cratebase_core::RecordAction;

/// PocketBase's `security.RandomString(40)` client id. The JS SDK's own
/// test asserts `/^[A-Za-z0-9]{40}$/`, so the alphabet is mixed case.
const CLIENT_ID_LEN: usize = 40;
const CLIENT_ID_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// How long a client may sit registered with no SSE reader draining it
/// before we drop its queue. Only reached if a peer stops reading but
/// never closes the socket.
const SEND_QUEUE_LIMIT: usize = 512;

/// `@request.context` during realtime rule evaluation, which rules can
/// branch on exactly as PocketBase's do.
const CONTEXT_REALTIME: &str = "realtime";

// ---------------------------------------------------------------- service

/// One connected SSE client.
struct Client {
    /// Frames queued for this client's stream. Unbounded so a publish
    /// never awaits a slow reader; [`SEND_QUEUE_LIMIT`] bounds it
    /// logically instead, and overflow disconnects rather than blocks.
    tx: mpsc::UnboundedSender<Event>,
    queued: AtomicU64,
    /// Whatever `POST /api/realtime` was authenticated as. Rebound on
    /// every POST, which is how the SDK propagates a login: it resubmits
    /// its subscriptions with the new token.
    auth: parking_lot::RwLock<Option<Auth>>,
    subscriptions: parking_lot::RwLock<Vec<Subscription>>,
}

/// One parsed topic. `key` is the client's exact string and the only
/// thing that goes back on the wire.
#[derive(Clone, Debug)]
struct Subscription {
    key: String,
    collection: String,
    record_id: Option<String>,
    filter: Option<String>,
    fields: Option<String>,
    expand: Option<String>,
}

impl Subscription {
    /// `name`, `name/id`, either optionally followed by
    /// `?options=<url-encoded JSON>`.
    fn parse(key: &str) -> Subscription {
        let (topic, options) = match key.split_once("?options=") {
            Some((t, o)) => (t, Some(o)),
            None => (key, None),
        };
        // The JS SDK always sends `collection + "/" + topic`, so a
        // whole-collection subscription arrives as `posts/*`, never bare
        // `posts` — `*` is the wildcard, not a record id. (A bare
        // `posts` is still accepted: that is what a hand-rolled client
        // or another SDK may send.)
        let (collection, record_id) = match topic.split_once('/') {
            Some((c, "*")) => (c.to_string(), None),
            Some((c, id)) if !id.is_empty() => (c.to_string(), Some(id.to_string())),
            _ => (topic.to_string(), None),
        };

        let mut sub = Subscription {
            key: key.to_string(),
            collection,
            record_id,
            filter: None,
            fields: None,
            expand: None,
        };

        // `{"query": {...}, "headers": {...}}`. Only `query` carries
        // anything this server acts on.
        if let Some(raw) = options {
            let decoded = percent_decode(raw);
            if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&decoded) {
                if let Some(Value::Object(q)) = map.get("query") {
                    let take = |k: &str| {
                        q.get(k)
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                    };
                    sub.filter = take("filter");
                    sub.fields = take("fields");
                    sub.expand = take("expand");
                }
            }
        }
        sub
    }

    /// Does this topic name `collection` (by name or id) and, if it is a
    /// single-record topic, this record?
    fn matches(&self, collection: &Collection, record: &Record) -> bool {
        if self.collection != collection.name && self.collection != collection.id {
            return false;
        }
        match &self.record_id {
            Some(id) => id == record.id(),
            None => true,
        }
    }
}

/// The connected clients, and an index from collection to the clients
/// watching it.
#[derive(Default)]
pub struct RealtimeService {
    clients: parking_lot::RwLock<HashMap<String, Arc<Client>>>,
    /// Collection name **and** id both map to the watching client ids, so
    /// a publish is a hash lookup rather than a walk over every client.
    /// Rebuilt on every subscription change, which is rare next to
    /// publishing.
    by_collection: parking_lot::RwLock<HashMap<String, HashSet<String>>>,
}

impl RealtimeService {
    pub fn new() -> RealtimeService {
        RealtimeService::default()
    }

    /// Number of connected clients. Exposed for the dashboard and tests.
    pub fn client_count(&self) -> usize {
        self.clients.read().len()
    }

    fn register(&self, id: String, tx: mpsc::UnboundedSender<Event>) -> Arc<Client> {
        let client = Arc::new(Client {
            tx,
            queued: AtomicU64::new(0),
            auth: parking_lot::RwLock::new(None),
            subscriptions: parking_lot::RwLock::new(Vec::new()),
        });
        self.clients.write().insert(id, client.clone());
        client
    }

    fn unregister(&self, id: &str) {
        self.clients.write().remove(id);
        let mut index = self.by_collection.write();
        for set in index.values_mut() {
            set.remove(id);
        }
        index.retain(|_, set| !set.is_empty());
    }

    /// Replace a client's subscriptions and reindex it.
    fn set_subscriptions(&self, id: &str, subs: Vec<Subscription>, auth: Option<Auth>) -> bool {
        let Some(client) = self.clients.read().get(id).cloned() else {
            return false;
        };
        {
            let mut index = self.by_collection.write();
            for set in index.values_mut() {
                set.remove(id);
            }
            for sub in &subs {
                index
                    .entry(sub.collection.clone())
                    .or_default()
                    .insert(id.to_string());
            }
            index.retain(|_, set| !set.is_empty());
        }
        *client.auth.write() = auth;
        *client.subscriptions.write() = subs;
        true
    }

    /// Clients that named this collection, by either name or id.
    fn watchers(&self, collection: &Collection) -> Vec<(String, Arc<Client>)> {
        let index = self.by_collection.read();
        let mut ids: HashSet<&String> = HashSet::new();
        if let Some(set) = index.get(&collection.name) {
            ids.extend(set);
        }
        if let Some(set) = index.get(&collection.id) {
            ids.extend(set);
        }
        if ids.is_empty() {
            return Vec::new();
        }
        let clients = self.clients.read();
        ids.into_iter()
            .filter_map(|id| clients.get(id).map(|c| (id.clone(), c.clone())))
            .collect()
    }

    /// Push one addressed frame straight to a connected client, by its
    /// `GET /api/realtime` client id — the same registry `publish`/
    /// `fan_out` above use for collection-scoped record events, reused
    /// here for `crate::llm`'s point-to-point chat chunks rather than
    /// standing up a second SSE connection/registration/queueing
    /// implementation.
    ///
    /// Unlike a record event, this has no topic to match against a
    /// subscription list: the caller already knows exactly which client
    /// it means to reach. Returns `false` if that client id isn't
    /// connected (it never subscribed, or already disconnected) — the
    /// caller treats that as "nobody is listening", not an error.
    pub fn send(&self, client_id: &str, event: &str, data: Value) -> bool {
        let Some(client) = self.clients.read().get(client_id).cloned() else {
            return false;
        };
        let frame = Event::default().event(event).data(data.to_string());
        if client.queued.fetch_add(1, Ordering::Relaxed) as usize >= SEND_QUEUE_LIMIT {
            tracing::debug!(client = %client_id, "realtime client too far behind; dropping");
            self.unregister(client_id);
            return false;
        }
        if client.tx.send(frame).is_err() {
            self.unregister(client_id);
            return false;
        }
        true
    }
}

/// Percent-decoding for the `?options=` blob. Small and local: the value
/// is a JSON string the SDK encoded with `encodeURIComponent`, so only
/// `%XX` needs undoing (`+` is *not* a space here — this is a path-ish
/// token, not a form body).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ----------------------------------------------------------------- routes

pub fn router() -> Router<App> {
    Router::new().route("/realtime", get(connect).post(submit))
}

/// `GET /api/realtime` — open the stream and hand back a client id.
async fn connect(State(app): State<App>) -> impl IntoResponse {
    let id = cratebase_core::ids::random_string(CLIENT_ID_LEN, CLIENT_ID_ALPHABET);
    let (tx, rx) = mpsc::unbounded_channel();

    // The id travels as the SSE `id:` field, which is what the JS SDK
    // reads (`lastEventId`); the JSON body carries it too, for clients
    // that look there instead.
    let hello = Event::default()
        .id(&id)
        .event("PB_CONNECT")
        .data(serde_json::json!({ "clientId": &id }).to_string());
    let _ = tx.send(hello);

    app.realtime().register(id.clone(), tx);

    let stream = ClientStream {
        app: app.clone(),
        id,
        rx,
    };

    Sse::new(stream).keep_alive(
        // Proxies commonly close an idle stream at 60s; PocketBase's own
        // client has no timeout of its own, so the comment ping is purely
        // to keep intermediaries from hanging up.
        KeepAlive::new().interval(Duration::from_secs(30)),
    )
}

/// The SSE body. Dropping it — which axum does as soon as the client
/// disconnects — is what unregisters the client, so a closed browser tab
/// cleans itself up without any liveness probe.
struct ClientStream {
    app: App,
    id: String,
    rx: mpsc::UnboundedReceiver<Event>,
}

impl Stream for ClientStream {
    type Item = Result<Event, std::convert::Infallible>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx).map(|opt| opt.map(Ok))
    }
}

impl Drop for ClientStream {
    fn drop(&mut self) {
        self.app.realtime().unregister(&self.id);
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitBody {
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    subscriptions: Vec<String>,
}

/// `POST /api/realtime` — set this client's subscriptions.
///
/// The request's own `Authorization` header is what the subscriptions are
/// evaluated as; re-POSTing after a login is how the SDK upgrades an
/// anonymous stream.
async fn submit(
    State(app): State<App>,
    MaybeAuth(auth): MaybeAuth,
    Json(body): Json<SubmitBody>,
) -> ApiResult<impl IntoResponse> {
    if body.client_id.is_empty() {
        return Err(ApiError(cratebase_core::AppError::bad_request(
            "Missing or invalid client id.",
        )));
    }

    let subs = body
        .subscriptions
        .iter()
        .filter(|s| !s.trim().is_empty())
        .map(|s| Subscription::parse(s))
        .collect::<Vec<_>>();

    if !app
        .realtime()
        .set_subscriptions(&body.client_id, subs, auth)
    {
        // PocketBase answers a stale/unknown client id with 404 rather
        // than silently accepting subscriptions nobody will ever read.
        return Err(ApiError(cratebase_core::AppError::not_found(
            "Missing or invalid client id.",
        )));
    }

    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- publish

/// Announce a committed record mutation to realtime subscribers.
///
/// Called **after** the transaction commits, with the post-write record
/// (for a delete, the row as it was). Returns immediately: the fan-out,
/// including any `expand` queries, happens on a spawned task so a slow
/// subscriber can never slow the writer down.
pub fn publish(
    app: &crate::app::App,
    collection: &Arc<Collection>,
    action: RecordAction,
    record: &Record,
) {
    let watchers = app.realtime().watchers(collection);
    if watchers.is_empty() {
        return;
    }

    let app = app.clone();
    let collection = collection.clone();
    let record = record.clone();
    tokio::spawn(async move {
        fan_out(app, collection, action, record, watchers).await;
    });
}

async fn fan_out(
    app: App,
    collection: Arc<Collection>,
    action: RecordAction,
    record: Record,
    watchers: Vec<(String, Arc<Client>)>,
) {
    // The record as the rules see it: PocketBase-shaped JSON of the
    // post-write state. Built once, shared by every subscriber's rule and
    // filter evaluation.
    //
    // `with_hidden` so a rule mentioning a hidden field sees a value
    // rather than a hole; `CollectionResolver` is what decides a hidden
    // field resolves to null for `@request.auth.*`, and that is a
    // separate concern from the record being matched.
    let snapshot = match record.to_json(cratebase_core::SerializeOptions {
        with_hidden: true,
        show_email: true,
        with_custom_data: false,
    }) {
        Value::Object(map) => map,
        _ => Map::new(),
    };

    // Serializing is the expensive half (it can run `expand` queries), so
    // subscribers asking for the same shape share one result.
    let mut rendered: HashMap<(Option<String>, Option<String>, bool), Value> = HashMap::new();

    for (client_id, client) in watchers {
        let auth = client.auth.read().clone();
        let subs = client.subscriptions.read().clone();

        for sub in subs {
            if !sub.matches(&collection, &record) {
                continue;
            }
            if !deliver_decision(&app, &collection, &snapshot, &sub, auth.as_ref()).await {
                continue;
            }

            let show_email = auth.as_ref().is_some_and(|a| a.is_superuser);
            let shape = (sub.fields.clone(), sub.expand.clone(), show_email);
            let payload = match rendered.get(&shape) {
                Some(v) => v.clone(),
                None => {
                    let Some(v) = render(&app, &collection, &record, &sub, auth.as_ref()).await
                    else {
                        continue;
                    };
                    rendered.insert(shape, v.clone());
                    v
                }
            };

            let frame = Event::default().event(&sub.key).data(
                serde_json::json!({ "action": action.as_str(), "record": payload }).to_string(),
            );

            if client.queued.fetch_add(1, Ordering::Relaxed) as usize >= SEND_QUEUE_LIMIT {
                // A peer that stopped reading but never closed. Dropping
                // it is better than growing this queue without bound.
                tracing::debug!(client = %client_id, "realtime client too far behind; dropping");
                app.realtime().unregister(&client_id);
                break;
            }
            if client.tx.send(frame).is_err() {
                app.realtime().unregister(&client_id);
                break;
            }
        }
    }
}

/// Whether this subscriber may see this record: the collection's
/// `listRule`, and then the topic's own `?filter=`.
async fn deliver_decision(
    app: &App,
    collection: &Arc<Collection>,
    snapshot: &Map<String, Value>,
    sub: &Subscription,
    auth: Option<&Auth>,
) -> bool {
    let superuser = auth.is_some_and(|a| a.is_superuser);
    let ctx = rule_context(auth, superuser);
    let resolver = CollectionResolver::new(
        collection.clone(),
        &app.db().collections,
        &ctx,
        app.db().dialect(),
    );

    // `None` (superusers only), `Some("")` (public) and the expression
    // case are all handled by the shared checker.
    if !check(app, &resolver, &collection.list_rule, snapshot, collection).await {
        return false;
    }

    match &sub.filter {
        None => true,
        // A topic filter applies to superusers too: it is the client's
        // own narrowing, not an access rule.
        Some(filter) => {
            let as_rule = Some(filter.clone());
            // Evaluated with `superuser: false` so the checker actually
            // runs the expression instead of short-circuiting.
            let ctx = rule_context(auth, false);
            let resolver = CollectionResolver::new(
                collection.clone(),
                &app.db().collections,
                &ctx,
                app.db().dialect(),
            );
            check(app, &resolver, &as_rule, snapshot, collection).await
        }
    }
}

fn rule_context(auth: Option<&Auth>, superuser: bool) -> RequestContext {
    RequestContext {
        auth: auth.map(Auth::to_auth_context),
        body: Map::new(),
        query: Map::new(),
        headers: Map::new(),
        method: "GET".to_string(),
        context: CONTEXT_REALTIME.to_string(),
        superuser,
    }
}

/// One rule/filter decision, denying on error.
///
/// An evaluation that fails outright (an unparsable expression, a
/// database error during the fallback) must not deliver: guessing "yes"
/// on an access check leaks a record.
async fn check(
    app: &App,
    resolver: &CollectionResolver<'_>,
    rule: &Option<String>,
    snapshot: &Map<String, Value>,
    collection: &Arc<Collection>,
) -> bool {
    match cratebase_db::rules::check_rule_against_row(app.db(), resolver, rule, snapshot).await {
        Ok(passed) => passed,
        Err(err) => {
            tracing::warn!(
                %err,
                collection = %collection.name,
                rule = ?rule,
                "realtime: rule check failed; not delivering"
            );
            false
        }
    }
}

/// The record as this subscription asked for it: `expand` resolved, then
/// `fields` projected.
async fn render(
    app: &App,
    collection: &Arc<Collection>,
    record: &Record,
    sub: &Subscription,
    auth: Option<&Auth>,
) -> Option<Value> {
    let mut record = record.clone();

    if let Some(spec) = sub.expand.as_deref() {
        let ctx = RequestContext {
            auth: auth.map(Auth::to_auth_context),
            body: Map::new(),
            query: Map::new(),
            headers: Map::new(),
            method: "GET".to_string(),
            context: CONTEXT_REALTIME.to_string(),
            superuser: auth.is_some_and(|a| a.is_superuser),
        };
        let mut items = vec![record];
        if let Err(err) = cratebase_db::expand::resolve(
            app.db(),
            &app.db().collections,
            &ctx,
            &mut items,
            spec,
            0,
        )
        .await
        {
            tracing::debug!(%err, "realtime: expand failed; sending unexpanded");
        }
        record = items.remove(0);
    }

    let show_email = auth.is_some_and(|a| a.is_superuser);
    let mut value = crate::routes::common::enrich_and_serialize(
        app,
        collection,
        record,
        auth.cloned(),
        show_email,
    )
    .await
    .ok()?;
    crate::routes::common::project(&mut value, sub.fields.as_deref());
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_bare_collection_topic() {
        let s = Subscription::parse("posts");
        assert_eq!(s.collection, "posts");
        assert_eq!(s.record_id, None);
        assert_eq!(s.key, "posts");
    }

    #[test]
    fn a_star_topic_is_the_whole_collection_not_a_record_id() {
        // What `collection("posts").subscribe("*", cb)` actually puts on
        // the wire: RecordService always prefixes the collection, so the
        // wildcard arrives as a path segment.
        let s = Subscription::parse("posts/*");
        assert_eq!(s.collection, "posts");
        assert_eq!(s.record_id, None);
    }

    #[test]
    fn parses_a_single_record_topic() {
        let s = Subscription::parse("posts/abc123");
        assert_eq!(s.collection, "posts");
        assert_eq!(s.record_id.as_deref(), Some("abc123"));
    }

    #[test]
    fn parses_the_options_blob_the_js_sdk_sends() {
        // Exactly what `subscribe(topic, cb, {filter, fields, expand})`
        // produces: `?options=` + encodeURIComponent(JSON).
        let raw = serde_json::json!({
            "query": { "filter": "views > 10", "fields": "id,title", "expand": "author" }
        })
        .to_string();
        let encoded = raw
            .chars()
            .map(|c| match c {
                'A'..='Z'
                | 'a'..='z'
                | '0'..='9'
                | '-'
                | '_'
                | '.'
                | '!'
                | '~'
                | '*'
                | '\''
                | '('
                | ')' => c.to_string(),
                other => other
                    .to_string()
                    .bytes()
                    .map(|b| format!("%{b:02X}"))
                    .collect(),
            })
            .collect::<String>();

        let s = Subscription::parse(&format!("posts?options={encoded}"));
        assert_eq!(s.collection, "posts");
        assert_eq!(s.filter.as_deref(), Some("views > 10"));
        assert_eq!(s.fields.as_deref(), Some("id,title"));
        assert_eq!(s.expand.as_deref(), Some("author"));
        // The key keeps the suffix: it is the SSE event name the client
        // registered its listener under.
        assert!(s.key.contains("?options="));
    }

    #[test]
    fn a_record_topic_with_options_still_splits_the_id() {
        let s = Subscription::parse("posts/rec1?options=%7B%7D");
        assert_eq!(s.collection, "posts");
        assert_eq!(s.record_id.as_deref(), Some("rec1"));
    }

    #[test]
    fn empty_option_values_are_treated_as_absent() {
        let s =
            Subscription::parse(r#"posts?options=%7B%22query%22%3A%7B%22filter%22%3A%22%22%7D%7D"#);
        assert_eq!(s.filter, None);
    }

    #[test]
    fn a_malformed_options_blob_degrades_to_a_plain_topic() {
        let s = Subscription::parse("posts?options=not-json");
        assert_eq!(s.collection, "posts");
        assert_eq!(s.filter, None);
    }

    #[test]
    fn percent_decoding_leaves_plus_alone() {
        // `+` means a literal plus in a URI component; only a form body
        // would read it as a space. A filter like `a+b` must survive.
        assert_eq!(percent_decode("a+b%20c"), "a+b c");
    }
}
