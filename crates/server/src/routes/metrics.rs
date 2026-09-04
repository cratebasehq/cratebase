//! `GET /metrics`: Prometheus text-exposition of request counts, latency
//! and error rate.
//!
//! # Why unauthenticated, and why not under `/api`
//!
//! Prometheus's own convention is an unauthenticated `/metrics` on a port
//! the operator trusts — the exporter does not invent auth, the deployment
//! firewalls the port. Cratebase is a single binary bound to one address,
//! so there is no separate "internal" port to put this behind; adding a
//! second auth scheme just for this endpoint would be more surface area
//! for less safety than the standard answer, which is: an operator who
//! wants metrics private puts a reverse proxy or firewall rule in front of
//! this route, exactly as they would for any other Prometheus target.
//!
//! For the same reason this route is mounted at the router root (see
//! `crate::router`) rather than nested under `/api`: it is not part of
//! the PocketBase-compatible surface, must not compete with `/api`'s rate
//! limiter (a scraper polling every few seconds would otherwise trip it),
//! and must not show up as a logged API call in `_logs`.
//!
//! # Why a process-global registry instead of an `App` field
//!
//! Every route and hook handle carries `State(app): State<App>`, but
//! metrics do not need anything `App` provides (no DB, no settings) and a
//! process only ever runs one `App` in practice, so a `static` avoids
//! threading a new field through `AppInner` for a counter that outlives
//! any single request or transaction anyway. [`record_metrics`] is wired
//! in as the outermost layer around the `/api` stack (see `crate::router`)
//! so it observes exactly what a client observed: the final status after
//! rate limiting and error handling, not just what reached the handler.
//!
//! # Why hand-rolled exposition instead of a `prometheus` crate
//!
//! Three counters and one fixed-bucket histogram do not justify a new
//! dependency; the text format itself is a handful of lines per metric.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;
use std::time::Instant;

use parking_lot::Mutex;

use axum::extract::Request;
use axum::http::{header, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

use crate::app::App;

pub fn router() -> Router<App> {
    Router::new().route("/metrics", get(metrics_handler))
}

/// Latency histogram bucket upper bounds, in seconds. Prometheus's own
/// default buckets, which comfortably span "instant SQLite read" to
/// "slow multipart upload" without needing to be tuned per deployment.
const BUCKETS: [f64; 11] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

#[derive(Default)]
struct Histogram {
    /// Cumulative counts: `bucket_counts[i]` is the number of
    /// observations `<= BUCKETS[i]`, i.e. already in Prometheus's `le=`
    /// form so exporting is a direct field read, not a running sum.
    bucket_counts: [u64; BUCKETS.len()],
    sum: f64,
    count: u64,
}

impl Histogram {
    fn observe(&mut self, seconds: f64) {
        for (bound, bucket) in BUCKETS.iter().zip(self.bucket_counts.iter_mut()) {
            if seconds <= *bound {
                *bucket += 1;
            }
        }
        self.sum += seconds;
        self.count += 1;
    }
}

/// The process-wide counters. `errors` is a plain atomic since it is a
/// single number on the hot path; the per-(method, status) breakdown and
/// the histogram are behind a `Mutex` because Prometheus text exposition
/// needs a consistent snapshot of a whole map/struct, not per-field
/// atomics that could be read mid-update.
#[derive(Default)]
struct Metrics {
    requests: Mutex<std::collections::HashMap<(Method, StatusCode), u64>>,
    latency: Mutex<Histogram>,
    errors: AtomicU64,
}

impl Metrics {
    fn record(&self, method: Method, status: StatusCode, seconds: f64) {
        *self.requests.lock().entry((method, status)).or_insert(0) += 1;
        if status.is_client_error() || status.is_server_error() {
            self.errors.fetch_add(1, Ordering::Relaxed);
        }
        self.latency.lock().observe(seconds);
    }

    /// Prometheus text-exposition format (the `0.0.4` variant every
    /// scraper still understands; OpenMetrics is a superset a real
    /// Prometheus server negotiates down to automatically).
    fn render(&self) -> String {
        let mut out = String::new();

        out.push_str(
            "# HELP cratebase_http_requests_total Total HTTP requests, by method and status.\n",
        );
        out.push_str("# TYPE cratebase_http_requests_total counter\n");
        let requests = self.requests.lock();
        // Sorted so exposition is stable across scrapes (nicer diffs, and
        // makes the unit test below deterministic).
        let mut rows: Vec<_> = requests.iter().collect();
        rows.sort_by_key(|((method, status), _)| (method.to_string(), status.as_u16()));
        for ((method, status), count) in rows {
            out.push_str(&format!(
                "cratebase_http_requests_total{{method=\"{method}\",status=\"{}\"}} {count}\n",
                status.as_u16()
            ));
        }
        drop(requests);
        out.push('\n');

        out.push_str(
            "# HELP cratebase_http_request_duration_seconds Request latency in seconds.\n",
        );
        out.push_str("# TYPE cratebase_http_request_duration_seconds histogram\n");
        let latency = self.latency.lock();
        for (bound, count) in BUCKETS.iter().zip(latency.bucket_counts.iter()) {
            out.push_str(&format!(
                "cratebase_http_request_duration_seconds_bucket{{le=\"{bound}\"}} {count}\n"
            ));
        }
        out.push_str(&format!(
            "cratebase_http_request_duration_seconds_bucket{{le=\"+Inf\"}} {}\n",
            latency.count
        ));
        out.push_str(&format!(
            "cratebase_http_request_duration_seconds_sum {}\n",
            latency.sum
        ));
        out.push_str(&format!(
            "cratebase_http_request_duration_seconds_count {}\n",
            latency.count
        ));
        drop(latency);
        out.push('\n');

        // Exposed as a raw counter rather than a precomputed ratio:
        // Prometheus's convention is `rate(cratebase_http_errors_total[5m])
        // / rate(cratebase_http_requests_total[5m])` in the query layer, so
        // the server only needs to hand over the two counters it already
        // has rather than pick a window length on the operator's behalf.
        out.push_str(
            "# HELP cratebase_http_errors_total Total requests answered with a 4xx or 5xx status.\n",
        );
        out.push_str("# TYPE cratebase_http_errors_total counter\n");
        out.push_str(&format!(
            "cratebase_http_errors_total {}\n",
            self.errors.load(Ordering::Relaxed)
        ));

        out
    }
}

fn registry() -> &'static Metrics {
    static REGISTRY: LazyLock<Metrics> = LazyLock::new(Metrics::default);
    &REGISTRY
}

/// Wired as the outermost layer around the `/api` stack (see
/// `crate::router`), so it counts what a caller actually received —
/// including a 429 from the rate limiter or a 5xx from a panic handler —
/// not just requests that reached a route handler.
pub async fn record_metrics(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let started = Instant::now();
    let response = next.run(req).await;
    let elapsed = started.elapsed().as_secs_f64();
    registry().record(method, response.status(), elapsed);
    response
}

async fn metrics_handler() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        registry().render(),
    )
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use axum::routing::get as axum_get;
    use tower::ServiceExt;

    use super::*;

    /// Builds a tiny standalone router (no `App`, no DB) wired the same
    /// way `crate::router` wires the real one: `record_metrics` as a
    /// layer around ordinary routes, plus `/metrics` itself outside it.
    fn test_app() -> Router {
        let instrumented = Router::new()
            .route("/ping", axum_get(|| async { StatusCode::OK }))
            .route(
                "/boom",
                axum_get(|| async { StatusCode::INTERNAL_SERVER_ERROR }),
            )
            .layer(axum::middleware::from_fn(record_metrics));
        instrumented.merge(Router::new().route("/metrics", axum_get(metrics_handler)))
    }

    async fn scrape(app: &Router) -> String {
        let response = app
            .clone()
            .oneshot(HttpRequest::get("/metrics").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(content_type.starts_with("text/plain"));
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    /// A count parsed out of one specific exposition line, so the
    /// increment assertions below do not depend on exact global state
    /// (this registry is process-global, and other tests in this binary
    /// could in principle run first) — only on the delta this test
    /// itself produces.
    fn parse_counter(body: &str, metric: &str, labels: &str) -> u64 {
        let needle = if labels.is_empty() {
            format!("{metric} ")
        } else {
            format!("{metric}{{{labels}}} ")
        };
        // A combination that has never been observed simply has no
        // exposition line (Prometheus counters only appear once
        // incremented at least once) — that is a count of zero, not a
        // malformed scrape.
        match body.lines().find(|line| line.starts_with(&needle)) {
            Some(line) => line.rsplit(' ').next().unwrap().parse().unwrap(),
            None => 0,
        }
    }

    #[tokio::test]
    async fn exposition_is_well_formed_prometheus_text() {
        let app = test_app();
        let body = scrape(&app).await;

        assert!(body.contains("# HELP cratebase_http_requests_total"));
        assert!(body.contains("# TYPE cratebase_http_requests_total counter"));
        assert!(body.contains("# TYPE cratebase_http_request_duration_seconds histogram"));
        assert!(body.contains("cratebase_http_request_duration_seconds_bucket{le=\"+Inf\"}"));
        assert!(body.contains("cratebase_http_request_duration_seconds_sum"));
        assert!(body.contains("cratebase_http_request_duration_seconds_count"));
        assert!(body.contains("# TYPE cratebase_http_errors_total counter"));

        // Every non-comment, non-blank line is either a metric sample
        // (`name{labels} value` or `name value`) with a value Prometheus
        // can parse as a float, never anything malformed.
        for line in body.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let value = line.rsplit(' ').next().unwrap();
            assert!(
                value.parse::<f64>().is_ok(),
                "line {line:?} has a non-numeric value"
            );
        }
    }

    #[tokio::test]
    async fn counts_increment_on_requests() {
        let app = test_app();

        let before_ok = parse_counter(
            &scrape(&app).await,
            "cratebase_http_requests_total",
            "method=\"GET\",status=\"200\"",
        );
        let before_err = parse_counter(
            &scrape(&app).await,
            "cratebase_http_requests_total",
            "method=\"GET\",status=\"500\"",
        );
        let before_errors_total =
            parse_counter(&scrape(&app).await, "cratebase_http_errors_total", "");

        for _ in 0..3 {
            let response = app
                .clone()
                .oneshot(HttpRequest::get("/ping").body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
        let response = app
            .clone()
            .oneshot(HttpRequest::get("/boom").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let after = scrape(&app).await;
        assert_eq!(
            parse_counter(
                &after,
                "cratebase_http_requests_total",
                "method=\"GET\",status=\"200\""
            ),
            before_ok + 3
        );
        assert_eq!(
            parse_counter(
                &after,
                "cratebase_http_requests_total",
                "method=\"GET\",status=\"500\""
            ),
            before_err + 1
        );
        assert_eq!(
            parse_counter(&after, "cratebase_http_errors_total", ""),
            before_errors_total + 1
        );
    }
}
