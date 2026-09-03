use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::response::sse::Event;
use cratebase_core::Collection;
use cratebase_db::Db;
use cratebase_db::resolver::{evaluate_record_rule, AuthContext, RequestContext};
use serde_json::Value;
use tokio::sync::{mpsc, RwLock};

/// One connected SSE client and the topics it currently cares about.
/// Topics are either a bare collection name (`"posts"`, every change) or
/// `"posts/<id>"` (only that record).
struct Client {
    tx: mpsc::UnboundedSender<Event>,
    subscriptions: HashSet<String>,
    /// The identity that authenticated this connection, if any — set at
    /// connect time and refreshed whenever a subscribe request carries a
    /// bearer token. Used to enforce `listRule`/`viewRule` per subscriber
    /// so a client only ever receives events for records it could
    /// actually read through the regular REST API.
    auth: Option<AuthContext>,
}

/// In-process pub/sub hub for realtime record change events. Deliberately
/// single-node (no external broker): Cratebase's realtime feature targets
/// the common single-instance deployment. Horizontal scale-out would need
/// a shared broker (e.g. Postgres `LISTEN/NOTIFY` or a queue) — tracked as
/// a future enhancement.
#[derive(Clone, Default)]
pub struct RealtimeHub {
    clients: Arc<RwLock<HashMap<String, Client>>>,
}

impl RealtimeHub {
    /// Register a new client connection and return its id plus the receiver
    /// half of its event channel. `auth` is whatever identity the initial
    /// SSE request carried (a bearer token, if the client's transport can
    /// set one on a GET request); anonymous is fine, `subscribe` can
    /// upgrade it later.
    pub async fn connect(&self, auth: Option<AuthContext>) -> (String, mpsc::UnboundedReceiver<Event>) {
        let id = cratebase_core::new_id();
        let (tx, rx) = mpsc::unbounded_channel();
        self.clients.write().await.insert(
            id.clone(),
            Client {
                tx,
                subscriptions: HashSet::new(),
                auth,
            },
        );
        (id, rx)
    }

    pub async fn disconnect(&self, client_id: &str) {
        self.clients.write().await.remove(client_id);
    }

    /// Replace a client's subscription set, and refresh its identity if
    /// this request carried a bearer token (most realtime clients
    /// authenticate on the subscribe POST rather than the SSE GET, since
    /// only the former lets them set an `Authorization` header). Returns
    /// `false` if the client id is unknown (e.g. the SSE connection
    /// already dropped).
    ///
    /// Topics are opaque strings — there's no collection lookup here, so
    /// subscribing to a `View` collection's name always succeeds. It just
    /// never receives anything: `publish` is only ever called from the
    /// record write path (`records::create_record`/`update_record`/
    /// `delete_record`), and those reject writes against view collections
    /// before reaching it. No special-casing needed either way.
    pub async fn subscribe(&self, client_id: &str, topics: Vec<String>, auth: Option<AuthContext>) -> bool {
        let mut clients = self.clients.write().await;
        match clients.get_mut(client_id) {
            Some(client) => {
                client.subscriptions = topics.into_iter().collect();
                if auth.is_some() {
                    client.auth = auth;
                }
                true
            }
            None => false,
        }
    }

    /// Notify every subscribed client that `record` in `collection` was
    /// created/updated/deleted — but only the ones whose `listRule`
    /// (collection-topic subscribers) or `viewRule` (record-topic
    /// subscribers) actually admits this record for their identity.
    /// Evaluated against the record snapshot rather than the live table
    /// (see `evaluate_record_rule`), since a `delete` event fires after
    /// the row is already gone.
    pub async fn publish(&self, db: &Db, collection: &Collection, action: &str, record: &Value) {
        let collection_topic = collection.name.clone();
        let record_topic = record
            .get("id")
            .and_then(Value::as_str)
            .map(|id| format!("{}/{}", collection.name, id));
        let record_data = record.as_object().cloned().unwrap_or_default();

        // Snapshot the matching subscribers (id, sender, rule to check)
        // under the lock, then drop it before doing any DB work so a slow
        // rule evaluation never blocks new connects/subscribes.
        let candidates: Vec<(mpsc::UnboundedSender<Event>, Option<AuthContext>, bool)> = {
            let clients = self.clients.read().await;
            clients
                .values()
                .filter_map(|client| {
                    let via_collection = client.subscriptions.contains(&collection_topic);
                    let via_record = record_topic
                        .as_ref()
                        .is_some_and(|t| client.subscriptions.contains(t));
                    if !via_collection && !via_record {
                        return None;
                    }
                    // A collection-topic subscription is the broader
                    // intent (governed by listRule); a record-topic-only
                    // subscription is checked against viewRule.
                    Some((client.tx.clone(), client.auth.clone(), via_collection))
                })
                .collect()
        };

        if candidates.is_empty() {
            return;
        }

        let payload = serde_json::json!({ "action": action, "record": record }).to_string();

        for (tx, auth, via_collection) in candidates {
            let rule = if via_collection {
                &collection.list_rule
            } else {
                &collection.view_rule
            };
            let ctx = RequestContext {
                auth,
                data: Some(record_data.clone()),
            };
            let allowed = evaluate_record_rule(db, rule, collection, &ctx)
                .await
                .unwrap_or(false);
            if allowed {
                let _ = tx.send(Event::default().event("message").data(payload.clone()));
            }
        }
    }
}
