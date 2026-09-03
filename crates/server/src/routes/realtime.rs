use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use axum::{Json, Router};
use futures::Stream;
use serde::Deserialize;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::StreamExt;

use crate::extract::CurrentAuth;
use crate::http_error::ApiResult;
use crate::realtime::RealtimeHub;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/realtime", get(connect).post(set_subscriptions))
}

/// Wraps the client's event stream so the hub forgets about it as soon as
/// the SSE connection drops (browser navigated away, network died, ...),
/// instead of leaking a dead subscriber that would otherwise sit in the hub
/// forever.
struct ClientStream<S> {
    inner: S,
    hub: RealtimeHub,
    client_id: String,
}

impl<S: Stream + Unpin> Stream for ClientStream<S> {
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        Pin::new(&mut this.inner).poll_next(cx)
    }
}

impl<S> Drop for ClientStream<S> {
    fn drop(&mut self) {
        let hub = self.hub.clone();
        let client_id = self.client_id.clone();
        tokio::spawn(async move { hub.disconnect(&client_id).await });
    }
}

/// Opens a Server-Sent Events stream. The first event carries the
/// connection's `clientId`, which the client then posts back to
/// `/api/realtime` to declare which collections/records it wants updates
/// for. If the initial request carries a bearer token (not every SSE
/// transport can set one on a GET), that identity governs
/// `listRule`/`viewRule` checks until `set_subscriptions` refreshes it.
async fn connect(
    State(app): State<AppState>,
    CurrentAuth(auth): CurrentAuth,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (client_id, rx) = app.realtime.connect(auth).await;
    let hello = Event::default()
        .event("PB_CONNECT")
        .data(format!(r#"{{"clientId":"{client_id}"}}"#));

    let inner = tokio_stream::once(Ok(hello)).chain(UnboundedReceiverStream::new(rx).map(Ok));
    let stream = ClientStream {
        inner,
        hub: app.realtime.clone(),
        client_id,
    };

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(25))
            .text("ping"),
    )
}

#[derive(Deserialize)]
struct SubscriptionUpdate {
    #[serde(rename = "clientId")]
    client_id: String,
    #[serde(default)]
    subscriptions: Vec<String>,
}

async fn set_subscriptions(
    State(app): State<AppState>,
    CurrentAuth(auth): CurrentAuth,
    Json(body): Json<SubscriptionUpdate>,
) -> ApiResult<axum::http::StatusCode> {
    let ok = app
        .realtime
        .subscribe(&body.client_id, body.subscriptions, auth)
        .await;
    Ok(if ok {
        axum::http::StatusCode::NO_CONTENT
    } else {
        axum::http::StatusCode::NOT_FOUND
    })
}
