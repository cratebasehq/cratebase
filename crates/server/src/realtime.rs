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
//!
//! # Cross-node fan-out (Postgres only)
//!
//! Everything above happens against *this* process's in-memory client
//! registry, which only ever sees writes this process itself performed.
//! Behind a load balancer with several app instances sharing one
//! Postgres database, a client parked on instance A never hears about a
//! write instance B just committed — unless something bridges them.
//!
//! That bridge is `pg_notify`/`LISTEN`. [`publish`] below, after doing
//! its normal local fan-out, always also calls
//! [`cratebase_db::Engine::notify_realtime`] with a small JSON payload
//! (collection id, action, record id — never the full record: Postgres
//! caps a `NOTIFY` payload at 8000 bytes). `App::bootstrap` starts
//! [`start_cross_node_listener`] once per process, which
//! `subscribe_realtime`s for every process's payloads on that channel,
//! including its own (skipped by an `origin` field — this process's own
//! local subscribers were already reached synchronously). For anyone
//! else's payload, [`receive_cross_node`] re-fetches the record with
//! *this* process's own executor and hands it to the same [`fan_out`]
//! a local write uses, so rule evaluation runs against this process's
//! own current settings and rule text, never anything serialized by the
//! writer. A `delete` is the one exception: the row is gone everywhere
//! by the time this arrives, so the writer's pre-delete snapshot (hidden
//! fields stripped — this rides through Postgres's own query logs)
//! rides along in the payload instead; when that snapshot alone would
//! push the payload over the 8000-byte cap, [`notify_cross_node`] falls
//! back to an id-only snapshot instead of dropping the notify entirely,
//! since a delete only needs `record.id` for a client to drop the row.
//!
//! SQLite is single-node by definition (one file, one process), so
//! `notify_realtime`/`subscribe_realtime` are no-ops there — see their
//! doc comments on [`cratebase_db::Engine`] — and this file's behavior
//! on SQLite is unchanged from before this section existed.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use cratebase_core::{AppError, Collection, Record};
use cratebase_db::context::{AuthContext, CollectionResolver, RequestContext};
use cratebase_db::engine::Executor;
use cratebase_db::records::find_by_id_raw;
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
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

/// A member's presence entry is dropped this long after its most recent
/// heartbeat/update — see the module doc's "Channels and presence"
/// section. Deliberately a few multiples of a reasonable client
/// heartbeat cadence (the SDK's `presence.track` re-POSTs roughly every
/// `PRESENCE_TTL / 3`), so one missed beat from a slow network never
/// flaps a member's join/leave state.
const PRESENCE_TTL: Duration = Duration::from_secs(45);
/// How often [`App::bootstrap`]'s sweep task calls [`RealtimeService::sweep_presence`].
const PRESENCE_SWEEP_INTERVAL: Duration = Duration::from_secs(15);
/// Channel names are a topic segment (`channel:<name>`), never a full
/// path: no `/` (would collide with `Subscription::parse`'s record-id
/// split), no whitespace, and bounded length. `*` is reserved for a
/// `_channels` config row's own prefix-pattern suffix, not something a
/// caller ever passes.
const MAX_CHANNEL_NAME_LEN: usize = 150;

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
pub struct RealtimeService {
    clients: parking_lot::RwLock<HashMap<String, Arc<Client>>>,
    /// Collection name **and** id both map to the watching client ids, so
    /// a publish is a hash lookup rather than a walk over every client.
    /// Rebuilt on every subscription change, which is rare next to
    /// publishing.
    by_collection: parking_lot::RwLock<HashMap<String, HashSet<String>>>,
    /// Random id identifying *this* `App`'s realtime instance in every
    /// cross-node payload it sends (Postgres only — see the module
    /// doc's "Cross-node fan-out" section). Generated once per
    /// `RealtimeService`, not once per process: one OS process is
    /// expected to run exactly one `App`, but nothing enforces that
    /// (tests routinely run several `App`s against one shared database
    /// in-process, and nor does anything rule it out for an embedder),
    /// and a process-wide id would make a second in-process instance
    /// mistake every other instance's writes for its own echo and
    /// silently drop them.
    origin: String,
    /// Channel presence: `channel -> client_id -> member`. A member whose
    /// [`PresenceMember::origin`] is this instance's own [`Self::origin`]
    /// is one of *this* node's connected clients (removed the moment its
    /// SSE stream disconnects, via [`disconnect_client`]); any other
    /// origin is a replica of another node's member, kept fresh by that
    /// node's own heartbeats/updates riding the cross-node `"presence"`
    /// payload and expired locally by [`Self::sweep_presence`] if that
    /// node goes quiet (crashes, network partition, ...) — see the
    /// module doc's "Channels and presence" section.
    presence: parking_lot::RwLock<HashMap<String, HashMap<String, PresenceMember>>>,
    /// Reverse index of *this node's own* presence members, so a
    /// disconnecting client's channels can be found without scanning
    /// every channel in [`Self::presence`]. Never holds a replica entry.
    presence_by_client: parking_lot::RwLock<HashMap<String, HashSet<String>>>,
}

/// One channel's presence entry for one client. See [`RealtimeService::presence`].
#[derive(Clone)]
struct PresenceMember {
    state: Value,
    /// `{id, collectionName}` snapshot, or `Value::Null` when anonymous —
    /// never the full auth record (no reason for every subscriber to see
    /// more than that).
    auth: Value,
    updated_at: Instant,
}

impl RealtimeService {
    pub fn new() -> RealtimeService {
        RealtimeService {
            clients: parking_lot::RwLock::new(HashMap::new()),
            by_collection: parking_lot::RwLock::new(HashMap::new()),
            origin: cratebase_core::ids::random_string(20, CLIENT_ID_ALPHABET),
            presence: parking_lot::RwLock::new(HashMap::new()),
            presence_by_client: parking_lot::RwLock::new(HashMap::new()),
        }
    }

    /// This instance's [`Self::origin`] field; see its doc comment.
    fn origin(&self) -> &str {
        &self.origin
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

    /// Whether `id` is a currently-connected `GET /api/realtime` client —
    /// used by `channel_presence_track` to require a live SSE stream
    /// before accepting a presence heartbeat for it (see that handler's
    /// doc comment).
    fn is_connected(&self, id: &str) -> bool {
        self.clients.read().contains_key(id)
    }

    /// Clients subscribed to exactly this topic string — the channel
    /// equivalent of [`Self::watchers`], which is keyed by a
    /// [`Collection`]'s name/id instead. `key` is `"channel:<name>"`; see
    /// the module doc's "Channels and presence" section for why a bare
    /// `channel:<name>` topic (no `?options=`) reuses [`Self::by_collection`]
    /// unmodified rather than needing a second index.
    fn watchers_by_key(&self, key: &str) -> Vec<(String, Arc<Client>)> {
        let index = self.by_collection.read();
        let Some(set) = index.get(key) else {
            return Vec::new();
        };
        let clients = self.clients.read();
        set.iter()
            .filter_map(|id| clients.get(id).map(|c| (id.clone(), c.clone())))
            .collect()
    }

    /// Insert/refresh one channel's presence member. Returns `true` when
    /// this is a brand-new member (a "join"), `false` for a heartbeat/
    /// state update on an already-present one.
    fn presence_upsert(
        &self,
        channel: &str,
        client_id: &str,
        origin: &str,
        state: Value,
        auth: Value,
    ) -> bool {
        let is_own = origin == self.origin;
        let mut presence = self.presence.write();
        let members = presence.entry(channel.to_string()).or_default();
        let is_new = !members.contains_key(client_id);
        members.insert(
            client_id.to_string(),
            PresenceMember {
                state,
                auth,
                updated_at: Instant::now(),
            },
        );
        drop(presence);
        if is_own {
            self.presence_by_client
                .write()
                .entry(client_id.to_string())
                .or_default()
                .insert(channel.to_string());
        }
        is_new
    }

    /// Remove one channel's presence member. Returns `true` if it existed.
    fn presence_remove(&self, channel: &str, client_id: &str) -> bool {
        let mut presence = self.presence.write();
        let existed = presence
            .get_mut(channel)
            .map(|members| members.remove(client_id).is_some())
            .unwrap_or(false);
        presence.retain(|_, members| !members.is_empty());
        drop(presence);
        if let Some(set) = self.presence_by_client.write().get_mut(client_id) {
            set.remove(channel);
        }
        existed
    }

    /// Every channel this node's *own* client is currently tracked in
    /// (never a replica — see [`Self::presence`]'s doc comment), removing
    /// the reverse-index entry as it goes. Called once, from
    /// [`disconnect_client`].
    fn presence_take_client_channels(&self, client_id: &str) -> Vec<String> {
        self.presence_by_client
            .write()
            .remove(client_id)
            .map(|set| set.into_iter().collect())
            .unwrap_or_default()
    }

    /// `GET .../presence`'s member list: `{clientId, state, auth}` per
    /// current member, oldest first. Does not itself expire anything —
    /// [`Self::sweep_presence`] is what removes a stale replica, on its
    /// own schedule — so two calls a few seconds apart, both before a
    /// sweep runs, agree with each other.
    fn presence_list(&self, channel: &str) -> Vec<(String, Value, Value)> {
        let presence = self.presence.read();
        let Some(members) = presence.get(channel) else {
            return Vec::new();
        };
        let mut out: Vec<(String, Value, Value, Instant)> = members
            .iter()
            .map(|(id, m)| (id.clone(), m.state.clone(), m.auth.clone(), m.updated_at))
            .collect();
        out.sort_by_key(|(_, _, _, at)| *at);
        out.into_iter().map(|(id, s, a, _)| (id, s, a)).collect()
    }

    /// Drop every *replica* presence member (an [`PresenceMember::origin`]
    /// other than this node's own) whose last heartbeat/update is older
    /// than [`PRESENCE_TTL`] — the mechanism that makes a crashed node's
    /// members eventually disappear everywhere else, since a dead node
    /// sends no more `"presence"` cross-node payloads to refresh them.
    /// This node's *own* members are governed by real disconnects
    /// ([`disconnect_client`]), not this sweep, though a stale one here
    /// (a `Drop` that somehow ran without reaching
    /// [`disconnect_client`]) is still cleaned up defensively. Returns
    /// the `(channel, client_id)` pairs removed, so the caller can
    /// broadcast a `presence.leave` for each.
    fn sweep_presence(&self) -> Vec<(String, String)> {
        let now = Instant::now();
        let mut removed = Vec::new();
        let mut presence = self.presence.write();
        presence.retain(|channel, members| {
            members.retain(|client_id, member| {
                let stale = now.duration_since(member.updated_at) > PRESENCE_TTL;
                if stale {
                    removed.push((channel.clone(), client_id.clone()));
                }
                !stale
            });
            !members.is_empty()
        });
        drop(presence);
        if !removed.is_empty() {
            let mut by_client = self.presence_by_client.write();
            for (channel, client_id) in &removed {
                if let Some(set) = by_client.get_mut(client_id) {
                    set.remove(channel);
                }
            }
        }
        removed
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

impl Default for RealtimeService {
    fn default() -> Self {
        Self::new()
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

pub fn router() -> Router<App> {
    Router::new()
        .route("/realtime", get(connect).post(submit))
        .route("/realtime/channels/{name}/publish", post(channel_publish))
        .route(
            "/realtime/channels/{name}/presence",
            get(channel_presence_list).post(channel_presence_track),
        )
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
        // Also leaves every channel this client had presence in — see
        // `disconnect_client`'s doc comment.
        disconnect_client(&self.app, &self.id);
    }
}

/// Unregister an SSE client and broadcast `presence.leave` for every
/// channel it was tracked in — the "automatic leave on SSE disconnect"
/// half of the module doc's "Channels and presence" section. Spawns the
/// actual broadcast rather than doing it inline, since [`Drop`] cannot be
/// `async` and this runs from [`ClientStream`]'s `Drop` impl.
fn disconnect_client(app: &App, client_id: &str) {
    let channels = app.realtime().presence_take_client_channels(client_id);
    app.realtime().unregister(client_id);
    if channels.is_empty() {
        return;
    }
    let app = app.clone();
    let client_id = client_id.to_string();
    tokio::spawn(async move {
        for channel in channels {
            broadcast_presence(
                &app,
                &channel,
                "presence.leave",
                &client_id,
                &Value::Null,
                &Value::Null,
            )
            .await;
        }
    });
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

/// Announce a committed record mutation to realtime subscribers.
///
/// Called **after** the transaction commits, with the post-write record
/// (for a delete, the row as it was). Returns immediately: both the
/// local fan-out and the cross-node notify below (including any
/// `expand` queries the former runs) happen on spawned tasks so a slow
/// subscriber, or a slow Postgres round trip, can never slow the writer
/// down.
pub fn publish(
    app: &crate::app::App,
    collection: &Arc<Collection>,
    action: RecordAction,
    record: &Record,
) {
    // Cross-node first, and unconditionally: this process has no way to
    // know whether some *other* process sharing the database has
    // watchers for this collection, only whether it does itself. See the
    // module doc's "Cross-node fan-out" section. Skipped entirely on a
    // backend that doesn't support it (SQLite): `notify_realtime` there
    // is a no-op anyway, so the clone/spawn/serialize below would just
    // be wasted work on every single write.
    if app.db().engine.supports_cross_node() {
        let app = app.clone();
        let collection = collection.clone();
        let record = record.clone();
        tokio::spawn(async move {
            notify_cross_node(&app, &collection, action, &record).await;
        });
    }

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

fn parse_action(s: &str) -> Option<RecordAction> {
    match s {
        "create" => Some(RecordAction::Create),
        "update" => Some(RecordAction::Update),
        "delete" => Some(RecordAction::Delete),
        _ => None,
    }
}

/// Tell every other process sharing this database about a write (see the
/// module doc's "Cross-node fan-out" section). SQLite: `notify_realtime`
/// is a no-op there, so this call costs one no-op `await` and nothing
/// else.
async fn notify_cross_node(
    app: &App,
    collection: &Arc<Collection>,
    action: RecordAction,
    record: &Record,
) {
    let mut payload = serde_json::json!({
        // Explicit for symmetry with the `"channel"`/`"presence"` kinds
        // below `dispatch_cross_node` also handles; a payload with no
        // `kind` at all (predating this field) is still read as `"record"`
        // — see `dispatch_cross_node`'s doc comment.
        "kind": "record",
        "origin": app.realtime().origin(),
        "collection": collection.id,
        "action": action.as_str(),
        "id": record.id(),
    });
    if action == RecordAction::Delete {
        // The row won't exist for the receiving process to re-fetch, so
        // the pre-delete snapshot has to ride along; every other action
        // sends none and lets the receiver re-`SELECT` its own current
        // copy instead (see `receive_cross_node`).
        //
        // `with_hidden: false`: Postgres logs bound statement parameters,
        // and every other `LISTEN cratebase_realtime` client on this
        // database receives this payload verbatim — a password hash or
        // auth token key riding along here would leak far more widely
        // than the row itself ever would.
        let mut candidate = payload.clone();
        candidate["snapshot"] = record.to_json(cratebase_core::SerializeOptions {
            with_hidden: false,
            show_email: true,
            with_custom_data: false,
        });
        // A large editor/json/text field can push the snapshot over
        // Postgres's `NOTIFY` payload cap; `>=` to match Postgres's own
        // `strlen(payload) >= NOTIFY_PAYLOAD_MAX_LENGTH` rejection.
        // Falling back to an id-only snapshot rather than just dropping
        // the notify entirely (the old behavior) still lets
        // `receive_cross_node` emit `{"action":"delete","record":{"id":...}}`
        // — PocketBase clients key a delete on `record.id` alone, so
        // this is enough for every other node's subscribers to drop the
        // row from their local state, even though this particular
        // record's snapshot is too big to ride along.
        if candidate.to_string().len() >= cratebase_db::postgres::NOTIFY_PAYLOAD_LIMIT {
            payload["snapshot"] = serde_json::json!({ "id": record.id() });
        } else {
            payload = candidate;
        }
    }
    let payload = payload.to_string();
    if let Err(e) = app.db().engine.notify_realtime(&payload).await {
        tracing::warn!(
            error = %e,
            collection = %collection.name,
            action = action.as_str(),
            "cross-node realtime notify failed; other instances won't see this write"
        );
    }
}

/// Handle one payload from [`start_cross_node_listener`]: parse it,
/// figure out who is even watching this collection *on this process*
/// (cheap, and skips the rest entirely when nobody is), get a current
/// copy of the record, and run it through the exact same [`fan_out`] a
/// local write uses — so rule evaluation happens fresh, here, against
/// this process's own settings and rule text, never anything the writer
/// serialized (see the module doc).
async fn receive_cross_node_record(app: &App, msg: &Map<String, Value>) {
    if msg.get("origin").and_then(Value::as_str) == Some(app.realtime().origin()) {
        return;
    }
    let (Some(collection_id), Some(action), Some(id)) = (
        msg.get("collection").and_then(Value::as_str),
        msg.get("action")
            .and_then(Value::as_str)
            .and_then(parse_action),
        msg.get("id").and_then(Value::as_str),
    ) else {
        tracing::warn!("cross-node realtime payload missing collection/action/id; dropping");
        return;
    };
    let Some(collection) = app.db().collections.get(collection_id) else {
        // A schema change hasn't propagated to this process yet, or the
        // collection was deleted moments after the write. Either way
        // there is nothing to deliver.
        return;
    };

    let watchers = app.realtime().watchers(&collection);
    if watchers.is_empty() {
        return;
    }

    let record = match action {
        RecordAction::Delete => {
            let Some(Value::Object(snapshot)) = msg.get("snapshot").cloned() else {
                tracing::warn!("cross-node delete payload missing its record snapshot; dropping");
                return;
            };
            Record::from_loaded(collection.clone(), snapshot)
        }
        RecordAction::Create | RecordAction::Update => {
            match find_by_id_raw(app.db(), &collection, id).await {
                Ok(record) => record,
                // Already gone again, or replicated late enough that a
                // later write already superseded it — nothing to show.
                Err(_) => return,
            }
        }
    };

    fan_out(app.clone(), collection, action, record, watchers).await;
}

/// Start this process's half of cross-node fan-out: subscribe to every
/// other process's [`notify_realtime`] calls against the same database
/// and hand each one to [`receive_cross_node`]. Called once, from
/// `App::bootstrap`. A no-op on SQLite — see
/// [`cratebase_db::Engine::subscribe_realtime`]'s doc comment — so a
/// SQLite deployment's realtime behavior is unaffected by this existing
/// at all.
pub fn start_cross_node_listener(app: &App) {
    let engine = app.db().engine.clone();
    let app2 = app.clone();
    engine.subscribe_realtime(Arc::new(move |payload: String| {
        let app = app2.clone();
        tokio::spawn(async move {
            dispatch_cross_node(&app, &payload).await;
        });
    }));

    // The presence TTL sweep (see `RealtimeService::sweep_presence`'s doc
    // comment): runs on every node regardless of backend, not just
    // Postgres — a node's own disconnects are handled synchronously by
    // `disconnect_client`, but this is the safety net for a replica whose
    // origin node went away without a clean `"presence.leave"` ever
    // arriving (a crash, a network partition). Started once per process,
    // right alongside the cross-node listener rather than needing its own
    // call from `App::bootstrap`.
    let app = app.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(PRESENCE_SWEEP_INTERVAL);
        loop {
            ticker.tick().await;
            for (channel, client_id) in app.realtime().sweep_presence() {
                broadcast_presence(
                    &app,
                    &channel,
                    "presence.leave",
                    &client_id,
                    &Value::Null,
                    &Value::Null,
                )
                .await;
            }
        }
    });
}

/// One payload from [`start_cross_node_listener`]'s `LISTEN` subscription,
/// routed by its `"kind"` discriminator (see the module doc's "Cross-node
/// fan-out" section for why this exists at all: `notify_realtime`/
/// `subscribe_realtime` carry one untyped string channel, shared by every
/// kind of realtime event so a single `LISTEN cratebase_realtime` covers
/// all of them). A payload with no `kind` field predates this
/// discriminator — every payload `notify_cross_node` ever sent before
/// channels/presence existed — and is read as `"record"` for backward
/// compatibility with a rolling upgrade where an old node is still
/// sending the original shape.
async fn dispatch_cross_node(app: &App, payload: &str) {
    let Ok(Value::Object(msg)) = serde_json::from_str::<Value>(payload) else {
        tracing::warn!("cross-node realtime payload was not a JSON object; dropping");
        return;
    };
    match msg.get("kind").and_then(Value::as_str) {
        Some("channel") => receive_cross_node_channel(app, &msg).await,
        Some("presence") => receive_cross_node_presence(app, &msg).await,
        None | Some("record") => receive_cross_node_record(app, &msg).await,
        Some(other) => {
            tracing::warn!(kind = %other, "unknown cross-node realtime payload kind; dropping")
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
        // Rule re-evaluation here runs in-process against an in-memory
        // row snapshot (see `check`/`check_via_sql`'s fallback), never a
        // fresh geo query a GiST index could speed up, so there is
        // nothing this would ever accelerate.
        postgis_available: false,
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
            postgis_available: false,
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

// ---------------------------------------------------------------------
// Realtime channels + presence
// ---------------------------------------------------------------------
//
// `channel:<name>` is an ordinary topic string with no `/`, so
// `Subscription::parse`/`RealtimeService::by_collection` already index
// and match it correctly with no changes — see `watchers_by_key`. What's
// genuinely new here is: the `_channels` authorization lookup (no
// `Collection`/rule-on-a-row concept applies, since a channel isn't a
// record), the publish/presence HTTP endpoints, and presence bookkeeping.
// Both publish and presence events ride the exact same cross-node
// `LISTEN`/`NOTIFY` path record writes use (see [`dispatch_cross_node`]),
// distinguished by their `"kind"` field.

/// `Value::to_string()` of a publish's `data` plus its `event` name may
/// not exceed this many bytes — see [`channel_publish`]'s doc comment:
/// left as headroom under Postgres's own `NOTIFY` payload cap
/// (`cratebase_db::postgres::NOTIFY_PAYLOAD_LIMIT`, 8000 bytes) for the
/// envelope fields (`kind`/`origin`/`channel`/`event`) wrapped around it.
const MAX_PUBLISH_PAYLOAD: usize = cratebase_db::postgres::NOTIFY_PAYLOAD_LIMIT - 500;

/// A channel name is a topic segment, not a path: no `/` (would collide
/// with `Subscription::parse`'s record-id split on a bare `channel:foo/bar`
/// topic), no `*` (reserved for a `_channels` row's own prefix pattern,
/// never something a caller passes), no whitespace, and bounded length.
pub(crate) fn is_valid_channel_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_CHANNEL_NAME_LEN
        && name
            .chars()
            .all(|c| c.is_ascii_graphic() && c != '/' && c != '*')
}

/// One `_channels` row's authorization for a given channel name — see
/// `Collection::default_system_collections`'s comment on `channels` for
/// the null/""/expr convention `subscribe_rule`/`publish_rule` follow.
struct ChannelConfig {
    subscribe_rule: Option<String>,
    publish_rule: Option<String>,
    /// The part of the channel name past a prefix pattern's `*` (`""` for
    /// an exact-name match) — exposed to rules as `@request.data.suffix`.
    suffix: String,
}

/// Resolves `channel` against every `_channels` row: an exact `name`
/// match wins outright; otherwise the *longest* `"prefix*"` row whose
/// `prefix` is a prefix of `channel` (so `"room:vip:*"` outranks a
/// broader `"room:*"` for `"room:vip:1"`). `None` means no row configures
/// this channel at all — the secure default, channel disabled.
///
/// One query covering every row rather than one filtered by `name`:
/// `_channels` is operator-configured (expected to be small — tens, not
/// millions, of rows) and prefix matching can't be expressed as a single
/// indexed lookup anyway, so this trades a full-table scan of a tiny
/// table for not having to maintain a second, denormalized lookup path.
async fn resolve_channel_config(app: &App, channel: &str) -> Option<ChannelConfig> {
    let rows = app
        .db()
        .query(
            r#"SELECT "name", "subscribeRule", "publishRule" FROM "_channels""#,
            &[],
        )
        .await
        .map_err(|e| tracing::warn!(error = %e, "failed to load _channels config"))
        .ok()?;

    let mut best: Option<(usize, ChannelConfig)> = None;
    for row in &rows {
        let Some(name) = row.get_str("name").filter(|n| !n.is_empty()) else {
            continue;
        };
        let candidate = if name == channel {
            Some((usize::MAX, String::new()))
        } else if let Some(prefix) = name.strip_suffix('*') {
            channel
                .starts_with(prefix)
                .then(|| (prefix.len(), channel[prefix.len()..].to_string()))
        } else {
            None
        };
        let Some((specificity, suffix)) = candidate else {
            continue;
        };
        if best.as_ref().is_some_and(|(s, _)| *s >= specificity) {
            continue;
        }
        best = Some((
            specificity,
            ChannelConfig {
                subscribe_rule: decode_json_rule_text(row.get_str("subscribeRule")),
                publish_rule: decode_json_rule_text(row.get_str("publishRule")),
                suffix,
            },
        ));
    }
    best.map(|(_, cfg)| cfg)
}

/// Reads a `_channels.subscribeRule`/`.publishRule` raw column value the
/// way `crate::mail_templates::decode_send_rule` reads
/// `_emailTemplates.sendRule` off an already-decoded `Record` — except
/// this runs against the *raw* SQL text from `resolve_channel_config`'s
/// single query covering every row, so it re-derives that same JSON
/// decode step by hand instead: the column is `Json`-kind precisely so
/// `NULL` and `""` stay distinct, stored as the submitted rule string
/// JSON-encoded *twice* (see
/// `crate::routes::records::normalize_json_rule_fields`'s doc comment for
/// exactly why). `NULL`/empty/anything that doesn't decode to a JSON
/// string is `None` — the safe, deny reading, never a rule that could
/// evaluate to `true`.
fn decode_json_rule_text(raw: Option<&str>) -> Option<String> {
    let raw = raw?;
    if raw.trim().is_empty() {
        return None;
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(Value::String(s)) => Some(s),
        _ => None,
    }
}

/// Evaluates a non-empty channel `subscribeRule`/`publishRule` — mirrors
/// `crate::mail_templates::eval_send_rule`'s doc comment exactly, except
/// for what's bound: `@request.auth.*` (the caller, or all-null when
/// anonymous) and `@request.data.channel`/`@request.data.suffix` (the
/// full channel name, and the part matched by a prefix pattern's `*` —
/// `""` for an exact match). No bare field reference: there is no
/// "record" a channel operation is about. A rule that cannot be
/// evaluated in-process (a relation path, `@collection.X`, ...) or fails
/// to parse is a deny — fails closed, never open.
async fn eval_channel_rule(
    app: &App,
    rule: &str,
    auth: Option<&AuthContext>,
    channel: &str,
    suffix: &str,
) -> bool {
    let rule = rule.trim();
    if rule.is_empty() {
        return true;
    }
    let Ok(ast) = cratebase_filter::parse_cached(rule) else {
        return false;
    };
    let Some(collection) = app
        .db()
        .collections
        .get_by_name(cratebase_core::CHANNELS_COLLECTION)
    else {
        return false;
    };
    let mut body = Map::new();
    body.insert("channel".into(), Value::String(channel.to_string()));
    body.insert("suffix".into(), Value::String(suffix.to_string()));
    let ctx = RequestContext {
        auth: auth.cloned(),
        body,
        ..RequestContext::default()
    };
    let resolver =
        CollectionResolver::new(collection, &app.db().collections, &ctx, app.db().dialect());
    matches!(
        cratebase_filter::evaluate(&ast, &Map::new(), &resolver),
        Ok(true)
    )
}

/// Shared allow/deny decision for one rule field (`subscribeRule` or
/// `publishRule`) against one caller: a superuser always passes (even a
/// syntactically broken rule — same as every other rule in this
/// codebase), `None` (no matching `_channels` row, or the matched row's
/// rule field is `null`) denies non-superusers outright, `Some("")`
/// allows anyone, and `Some(expr)` defers to [`eval_channel_rule`].
async fn rule_allows(app: &App, rule: &Option<String>, auth: Option<&Auth>, channel: &str, suffix: &str) -> bool {
    if auth.is_some_and(|a| a.is_superuser) {
        return true;
    }
    match rule {
        None => false,
        Some(r) if r.trim().is_empty() => true,
        Some(r) => {
            let ctx = auth.map(Auth::to_auth_context);
            eval_channel_rule(app, r, ctx.as_ref(), channel, suffix).await
        }
    }
}

/// Whether `auth` may subscribe to (or track presence on) `channel` —
/// re-checked per subscriber at every delivery, exactly like a
/// collection's `listRule` (see [`deliver_decision`]), so a rule edit or
/// a login/logout takes effect on the very next message rather than only
/// at the next `POST /api/realtime` resubscribe.
async fn channel_subscribe_allowed(app: &App, auth: Option<&Auth>, channel: &str) -> bool {
    match resolve_channel_config(app, channel).await {
        Some(cfg) => rule_allows(app, &cfg.subscribe_rule, auth, channel, &cfg.suffix).await,
        None => auth.is_some_and(|a| a.is_superuser),
    }
}

/// Whether `auth` may publish on `channel` — checked once, synchronously,
/// by [`channel_publish`] itself (unlike [`channel_subscribe_allowed`],
/// there is no per-recipient re-check: publishing is a single action, not
/// an ongoing subscription).
async fn channel_publish_allowed(app: &App, auth: Option<&Auth>, channel: &str) -> bool {
    match resolve_channel_config(app, channel).await {
        Some(cfg) => rule_allows(app, &cfg.publish_rule, auth, channel, &cfg.suffix).await,
        None => auth.is_some_and(|a| a.is_superuser),
    }
}

fn channel_denied() -> AppError {
    AppError::forbidden("You are not allowed to access this channel.")
}

/// Broadcast `{event, data}` to every current subscriber of
/// `channel:<name>` — `crate::jsvm_host`'s `$realtime.publish` and
/// `channel_publish` (`POST /api/realtime/channels/{name}/publish`) both
/// call this after their own authorization check. Local subscribers are
/// reached via a spawned [`channel_fan_out`]; every other node sharing
/// this database is reached via `notify_realtime`, exactly like
/// [`publish`]'s "Cross-node fan-out" (see the module doc) — unconditional
/// on a backend that supports it, since this process has no way to know
/// whether some *other* process has subscribers for this channel.
pub fn publish_channel(app: &App, channel: &str, event: &str, data: Value) {
    if app.db().engine.supports_cross_node() {
        let payload = serde_json::json!({
            "kind": "channel",
            "origin": app.realtime().origin(),
            "channel": channel,
            "event": event,
            "data": data.clone(),
        })
        .to_string();
        let app2 = app.clone();
        tokio::spawn(async move {
            if let Err(e) = app2.db().engine.notify_realtime(&payload).await {
                tracing::warn!(error = %e, "cross-node channel publish notify failed");
            }
        });
    }
    let app2 = app.clone();
    let channel2 = channel.to_string();
    let event2 = event.to_string();
    tokio::spawn(async move {
        channel_fan_out(&app2, &channel2, &event2, &data).await;
    });
}

/// Deliver one channel message to this node's own subscribers of
/// `channel:<name>`, re-checking [`channel_subscribe_allowed`] per
/// subscriber's own current auth (see that function's doc comment for
/// why per-delivery, not per-subscribe). Shared by a local
/// [`publish_channel`] call, [`receive_cross_node_channel`], and every
/// presence event (`join`/`update`/`leave` are just channel messages
/// with a well-known event name — see [`broadcast_presence`]).
async fn channel_fan_out(app: &App, channel: &str, event: &str, data: &Value) {
    let key = format!("channel:{channel}");
    let watchers = app.realtime().watchers_by_key(&key);
    if watchers.is_empty() {
        return;
    }
    let payload = serde_json::json!({ "event": event, "data": data }).to_string();
    for (client_id, client) in watchers {
        let auth = client.auth.read().clone();
        if !channel_subscribe_allowed(app, auth.as_ref(), channel).await {
            continue;
        }
        let subs = client.subscriptions.read().clone();
        for sub in subs {
            if sub.collection != key {
                continue;
            }
            let frame = Event::default().event(&sub.key).data(payload.clone());
            if client.queued.fetch_add(1, Ordering::Relaxed) as usize >= SEND_QUEUE_LIMIT {
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

/// One channel's presence entry, as `GET .../presence` and every
/// `presence.*` event report it.
#[derive(Serialize)]
struct PresenceEntry {
    #[serde(rename = "clientId")]
    client_id: String,
    state: Value,
    auth: Value,
}

/// Authorizes and records one presence heartbeat, broadcasting the
/// resulting `presence.join`/`presence.update`. Returns the event name
/// that fired, or `Err` when `auth` isn't allowed to subscribe to (and
/// therefore track presence on) `channel`.
async fn presence_track(
    app: &App,
    channel: &str,
    client_id: &str,
    auth: Option<&Auth>,
    state: Value,
) -> Result<&'static str, AppError> {
    if !channel_subscribe_allowed(app, auth, channel).await {
        return Err(channel_denied());
    }
    let auth_snapshot = auth
        .map(|a| serde_json::json!({ "id": a.id, "collectionName": a.collection_name }))
        .unwrap_or(Value::Null);
    let origin = app.realtime().origin().to_string();
    let is_new =
        app.realtime()
            .presence_upsert(channel, client_id, &origin, state.clone(), auth_snapshot.clone());
    let kind = if is_new {
        "presence.join"
    } else {
        "presence.update"
    };
    broadcast_presence(app, channel, kind, client_id, &state, &auth_snapshot).await;
    Ok(kind)
}

/// Broadcasts one presence event — `join`/`update` (from [`presence_track`])
/// or `leave` (from [`disconnect_client`] or the TTL sweep in
/// [`start_cross_node_listener`]) — to this node's own subscribers via
/// [`channel_fan_out`] and to every other node via the same cross-node
/// `"presence"` payload kind [`receive_cross_node_presence`] reads back.
async fn broadcast_presence(
    app: &App,
    channel: &str,
    kind: &str,
    client_id: &str,
    state: &Value,
    auth: &Value,
) {
    if app.db().engine.supports_cross_node() {
        let payload = serde_json::json!({
            "kind": "presence",
            "origin": app.realtime().origin(),
            "channel": channel,
            "event": kind,
            "clientId": client_id,
            "state": state,
            "auth": auth,
        })
        .to_string();
        let app2 = app.clone();
        tokio::spawn(async move {
            if let Err(e) = app2.db().engine.notify_realtime(&payload).await {
                tracing::warn!(error = %e, "cross-node presence notify failed");
            }
        });
    }
    let data = serde_json::json!({ "clientId": client_id, "state": state, "auth": auth });
    channel_fan_out(app, channel, kind, &data).await;
}

/// [`dispatch_cross_node`]'s `"channel"` handler: re-fan-out a publish
/// from another node to this node's own subscribers. Skips its own
/// origin (already delivered synchronously by [`publish_channel`]'s
/// local half).
async fn receive_cross_node_channel(app: &App, msg: &Map<String, Value>) {
    if msg.get("origin").and_then(Value::as_str) == Some(app.realtime().origin()) {
        return;
    }
    let (Some(channel), Some(event)) = (
        msg.get("channel").and_then(Value::as_str),
        msg.get("event").and_then(Value::as_str),
    ) else {
        return;
    };
    let data = msg.get("data").cloned().unwrap_or(Value::Null);
    channel_fan_out(app, channel, event, &data).await;
}

/// [`dispatch_cross_node`]'s `"presence"` handler: applies the join/
/// update/leave to this node's own replica of that member (see
/// [`RealtimeService::presence`]'s doc comment) and re-fans it out to
/// this node's own subscribers. Skips its own origin, same reasoning as
/// [`receive_cross_node_channel`].
async fn receive_cross_node_presence(app: &App, msg: &Map<String, Value>) {
    let Some(origin) = msg.get("origin").and_then(Value::as_str) else {
        return;
    };
    if origin == app.realtime().origin() {
        return;
    }
    let (Some(channel), Some(client_id), Some(kind)) = (
        msg.get("channel").and_then(Value::as_str),
        msg.get("clientId").and_then(Value::as_str),
        msg.get("event").and_then(Value::as_str),
    ) else {
        return;
    };
    let state = msg.get("state").cloned().unwrap_or(Value::Null);
    let auth = msg.get("auth").cloned().unwrap_or(Value::Null);
    if kind == "presence.leave" {
        app.realtime().presence_remove(channel, client_id);
    } else {
        app.realtime()
            .presence_upsert(channel, client_id, origin, state.clone(), auth.clone());
    }
    let data = serde_json::json!({ "clientId": client_id, "state": state, "auth": auth });
    channel_fan_out(app, channel, kind, &data).await;
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublishBody {
    event: String,
    #[serde(default)]
    data: Value,
}

/// `POST /api/realtime/channels/{name}/publish` — see the module doc.
/// Authorization is `channel_publish_allowed`, checked once here (not
/// per-recipient — see that function's doc comment); size is capped at
/// [`MAX_PUBLISH_PAYLOAD`] bytes, answered `413` rather than silently
/// dropping the cross-node half of delivery (see that constant's doc
/// comment) or truncating the message.
async fn channel_publish(
    State(app): State<App>,
    Path(name): Path<String>,
    MaybeAuth(auth): MaybeAuth,
    Json(body): Json<PublishBody>,
) -> ApiResult<impl IntoResponse> {
    if !is_valid_channel_name(&name) {
        return Err(ApiError::bad_request("Invalid channel name."));
    }
    if body.event.trim().is_empty() {
        return Err(ApiError::bad_request("event is required."));
    }
    let size = body.event.len() + body.data.to_string().len();
    if size > MAX_PUBLISH_PAYLOAD {
        return Err(ApiError(AppError::payload_too_large(format!(
            "publish payload is too large ({size} bytes; the limit is {MAX_PUBLISH_PAYLOAD} bytes)."
        ))));
    }
    if !channel_publish_allowed(&app, auth.as_ref(), &name).await {
        return Err(ApiError(channel_denied()));
    }
    publish_channel(&app, &name, &body.event, body.data);
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PresenceBody {
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    state: Value,
}

/// `POST /api/realtime/channels/{name}/presence` — the presence
/// heartbeat. `clientId` must be a currently-connected `GET /api/realtime`
/// stream's id (same requirement as `POST /api/realtime` itself): presence
/// is tied to a live SSE connection so it can be automatically left on
/// disconnect (see [`disconnect_client`]), not a standalone resource a
/// caller could create without ever opening a stream.
async fn channel_presence_track(
    State(app): State<App>,
    Path(name): Path<String>,
    MaybeAuth(auth): MaybeAuth,
    Json(body): Json<PresenceBody>,
) -> ApiResult<impl IntoResponse> {
    if !is_valid_channel_name(&name) {
        return Err(ApiError::bad_request("Invalid channel name."));
    }
    if body.client_id.is_empty() {
        return Err(ApiError::bad_request("Missing or invalid client id."));
    }
    if !app.realtime().is_connected(&body.client_id) {
        return Err(ApiError::not_found("Missing or invalid client id."));
    }
    let kind = presence_track(&app, &name, &body.client_id, auth.as_ref(), body.state)
        .await
        .map_err(ApiError)?;
    Ok(Json(serde_json::json!({ "event": kind })))
}

/// `GET /api/realtime/channels/{name}/presence` — the current member
/// list. Authorized the same way subscribing is (`channel_subscribe_allowed`):
/// presence membership is only ever visible to someone who could also
/// subscribe to the channel itself.
async fn channel_presence_list(
    State(app): State<App>,
    Path(name): Path<String>,
    MaybeAuth(auth): MaybeAuth,
) -> ApiResult<impl IntoResponse> {
    if !is_valid_channel_name(&name) {
        return Err(ApiError::bad_request("Invalid channel name."));
    }
    if !channel_subscribe_allowed(&app, auth.as_ref(), &name).await {
        return Err(ApiError(channel_denied()));
    }
    let members: Vec<PresenceEntry> = app
        .realtime()
        .presence_list(&name)
        .into_iter()
        .map(|(client_id, state, auth)| PresenceEntry {
            client_id,
            state,
            auth,
        })
        .collect();
    Ok(Json(serde_json::json!({ "members": members })))
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
