use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::response::sse::Event;
use serde_json::Value;
use tokio::sync::{mpsc, RwLock};

/// One connected SSE client and the topics it currently cares about.
/// Topics are either a bare collection name (`"posts"`, every change) or
/// `"posts/<id>"` (only that record).
struct Client {
    tx: mpsc::UnboundedSender<Event>,
    subscriptions: HashSet<String>,
}

/// In-process pub/sub hub for realtime record change events. Deliberately
/// single-node (no external broker): Cratebase's realtime feature targets
/// the common single-instance deployment, matching PocketBase's own
/// realtime design. Horizontal scale-out would need a shared broker (e.g.
/// Postgres `LISTEN/NOTIFY` or a queue) — tracked as a future enhancement.
#[derive(Clone, Default)]
pub struct RealtimeHub {
    clients: Arc<RwLock<HashMap<String, Client>>>,
}

impl RealtimeHub {
    /// Register a new client connection and return its id plus the receiver
    /// half of its event channel.
    pub async fn connect(&self) -> (String, mpsc::UnboundedReceiver<Event>) {
        let id = cratebase_core::new_id();
        let (tx, rx) = mpsc::unbounded_channel();
        self.clients.write().await.insert(
            id.clone(),
            Client {
                tx,
                subscriptions: HashSet::new(),
            },
        );
        (id, rx)
    }

    pub async fn disconnect(&self, client_id: &str) {
        self.clients.write().await.remove(client_id);
    }

    /// Replace a client's subscription set. Returns `false` if the client
    /// id is unknown (e.g. the SSE connection already dropped).
    pub async fn subscribe(&self, client_id: &str, topics: Vec<String>) -> bool {
        let mut clients = self.clients.write().await;
        match clients.get_mut(client_id) {
            Some(client) => {
                client.subscriptions = topics.into_iter().collect();
                true
            }
            None => false,
        }
    }

    /// Notify every subscribed client that `record` in `collection` was
    /// created/updated/deleted.
    pub async fn publish(&self, collection: &str, action: &str, record: &Value) {
        let collection_topic = collection.to_string();
        let record_topic = record
            .get("id")
            .and_then(Value::as_str)
            .map(|id| format!("{collection}/{id}"));

        let payload = serde_json::json!({ "action": action, "record": record }).to_string();
        let clients = self.clients.read().await;
        for client in clients.values() {
            let matches = client.subscriptions.contains(&collection_topic)
                || record_topic
                    .as_ref()
                    .is_some_and(|t| client.subscriptions.contains(t));
            if matches {
                let _ = client.tx.send(Event::default().event("message").data(payload.clone()));
            }
        }
    }
}
