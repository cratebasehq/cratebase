//! `settings.rateLimits`: PocketBase's configurable rate limiter.
//!
//! # Rule matching
//!
//! A rule's `label` is one of three things, and they are tried in this
//! order — most specific first, exactly as PocketBase does it:
//!
//! 1. **an exact path** (`/api/batch`) — wins outright;
//! 2. **a path prefix** (`/api/`) — the *longest* matching prefix wins,
//!    so `/api/collections/` beats `/api/`;
//! 3. **a tag** (`*:auth`, `*:create`, `posts:list`) — the route's own
//!    labels, so one rule can cover "every create on every collection"
//!    or "list on this one collection".
//!
//! Within each tier a rule only applies when its `audience` matches the
//! caller: `""` everyone, `@guest` only unauthenticated, `@auth` only
//! authenticated.
//!
//! # Cost per request
//!
//! The rule list is **compiled once** — into an exact-path hash map, a
//! length-sorted prefix list and a tag hash map — and rebuilt only when
//! [`App::set_settings`] swaps the settings. A request therefore costs one
//! `ArcSwap` load, one hash lookup, a walk of the (handful of) prefix
//! rules, and one hash lookup per tag; it never iterates the raw rule
//! vector and never re-parses settings. When `rateLimits.enabled` is
//! false the layer returns before doing any of it.
//!
//! Counters are fixed windows in memory keyed by `(rule label, client
//! id)`, where the client id is the authenticated record id when there is
//! one and the client IP otherwise — so one logged-in user does not spend
//! a shared office IP's budget. Expired windows are dropped by a
//! background sweep on a timer, never by iterating the map on the request
//! path.
//!
//! The client IP comes from [`crate::middleware::client_ip`], which only
//! trusts a forwarded header the operator has declared.
//!
//! [`App::set_settings`]: crate::app::App::set_settings

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use axum::extract::{ConnectInfo, Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use cratebase_core::settings::{RateLimitRule, RateLimits};
use cratebase_core::AppError;

use crate::app::App;
use crate::http_error::ApiError;
use crate::middleware::client_ip;

/// How often expired windows are swept out of the counter map.
const SWEEP_INTERVAL: Duration = Duration::from_secs(60);

/// Extra rate-limit labels a route can attach to its request when the URL
/// shape alone does not say enough (used for anything
/// [`tags_for`] cannot derive).
#[derive(Debug, Clone, Default)]
pub struct RouteTags(pub Vec<String>);

/// The rules for one label, split by audience so the lookup is a single
/// hash hit rather than a filtered scan.
#[derive(Debug, Clone, Default)]
struct Slots {
    all: Option<RateLimitRule>,
    guest: Option<RateLimitRule>,
    auth: Option<RateLimitRule>,
}

impl Slots {
    fn insert(&mut self, rule: &RateLimitRule) {
        let slot = match rule.audience.as_str() {
            "" => &mut self.all,
            "@guest" => &mut self.guest,
            "@auth" => &mut self.auth,
            // An unknown audience is treated as "nobody" rather than
            // "everybody": a typo must not silently widen a limit.
            _ => return,
        };
        // First rule with a given (label, audience) wins, matching
        // PocketBase's first-match iteration.
        if slot.is_none() {
            *slot = Some(rule.clone());
        }
    }

    fn pick(&self, authenticated: bool) -> Option<&RateLimitRule> {
        let specific = if authenticated {
            self.auth.as_ref()
        } else {
            self.guest.as_ref()
        };
        specific.or(self.all.as_ref())
    }
}

/// The compiled rule set. Immutable; swapped wholesale on a settings
/// change.
#[derive(Debug, Default)]
struct Compiled {
    enabled: bool,
    excluded_ips: HashSet<String>,
    exact: HashMap<String, Slots>,
    /// `(prefix, slots)` sorted by descending prefix length, so the first
    /// match is the longest one.
    prefixes: Vec<(String, Slots)>,
    tags: HashMap<String, Slots>,
    /// Longest configured window, used to bound the sweep.
    max_window: Duration,
}

impl Compiled {
    fn build(limits: &RateLimits) -> Compiled {
        let mut exact: HashMap<String, Slots> = HashMap::new();
        let mut prefixes: HashMap<String, Slots> = HashMap::new();
        let mut tags: HashMap<String, Slots> = HashMap::new();
        let mut max_window = Duration::from_secs(60);

        for rule in &limits.rules {
            if rule.label.is_empty() || rule.max_requests <= 0 || rule.duration <= 0 {
                continue;
            }
            max_window = max_window.max(Duration::from_secs(rule.duration as u64));
            let bucket = if !rule.label.starts_with('/') {
                &mut tags
            } else if rule.label.ends_with('/') {
                // PocketBase treats a trailing slash as "this subtree".
                &mut prefixes
            } else {
                &mut exact
            };
            bucket.entry(rule.label.clone()).or_default().insert(rule);
            // A non-trailing-slash path is also usable as a prefix
            // (`/api/collections` covering `/api/collections/posts`),
            // which is how PocketBase's `strings.HasPrefix` check behaves.
            if rule.label.starts_with('/') && !rule.label.ends_with('/') {
                prefixes.entry(rule.label.clone()).or_default().insert(rule);
            }
        }

        let mut prefixes: Vec<(String, Slots)> = prefixes.into_iter().collect();
        prefixes.sort_by_key(|(prefix, _)| std::cmp::Reverse(prefix.len()));

        Compiled {
            enabled: limits.enabled,
            excluded_ips: limits.excluded_ips.iter().cloned().collect(),
            exact,
            prefixes,
            tags,
            max_window,
        }
    }

    fn find(&self, path: &str, tags: &[String], authenticated: bool) -> Option<&RateLimitRule> {
        if let Some(rule) = self.exact.get(path).and_then(|s| s.pick(authenticated)) {
            return Some(rule);
        }
        for (prefix, slots) in &self.prefixes {
            if path.starts_with(prefix.as_str()) {
                if let Some(rule) = slots.pick(authenticated) {
                    return Some(rule);
                }
            }
        }
        tags.iter()
            .find_map(|tag| self.tags.get(tag).and_then(|s| s.pick(authenticated)))
    }
}

#[derive(Debug, Clone, Copy)]
struct Window {
    started: Instant,
    count: i64,
}

/// The in-memory limiter. One per [`App`].
pub struct RateLimiter {
    compiled: ArcSwap<Compiled>,
    windows: Mutex<HashMap<(String, String), Window>>,
    sweeping: AtomicBool,
}

impl Default for RateLimiter {
    fn default() -> Self {
        RateLimiter {
            compiled: ArcSwap::from_pointee(Compiled::default()),
            windows: Mutex::new(HashMap::new()),
            sweeping: AtomicBool::new(false),
        }
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Recompile the rule set. Called from `App::apply_settings`, so the
    /// request path never sees the raw `RateLimits`.
    pub fn configure(&self, limits: &RateLimits) {
        self.compiled.store(Arc::new(Compiled::build(limits)));
        // A shrunk window must take effect at once, so old counters go.
        self.reset();
    }

    /// Start the background sweep. Idempotent.
    pub fn start_sweeper(self: &Arc<Self>) {
        if self.sweeping.swap(true, Ordering::SeqCst) {
            return;
        }
        let limiter = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                let Some(limiter) = limiter.upgrade() else {
                    return;
                };
                limiter.sweep();
            }
        });
    }

    /// Drop every expired window. Runs off the request path.
    pub fn sweep(&self) {
        let max_window = self.compiled.load().max_window;
        let now = Instant::now();
        self.windows
            .lock()
            .expect("rate limiter poisoned")
            .retain(|_, w| now.duration_since(w.started) < max_window);
    }

    pub fn reset(&self) {
        self.windows.lock().expect("rate limiter poisoned").clear();
    }

    pub fn is_enabled(&self) -> bool {
        self.compiled.load().enabled
    }

    /// Count one request against the matching rule.
    ///
    /// `Err(AppError::TooManyRequests)` when the window is full; `Ok(())`
    /// when the limiter is off, the IP is excluded, no rule matches, or
    /// there is budget left.
    pub fn check(
        &self,
        path: &str,
        tags: &[String],
        authenticated: bool,
        client_id: &str,
        ip: &str,
    ) -> Result<(), AppError> {
        let compiled = self.compiled.load();
        if !compiled.enabled || compiled.excluded_ips.contains(ip) {
            return Ok(());
        }
        let Some(rule) = compiled.find(path, tags, authenticated) else {
            return Ok(());
        };

        let window = Duration::from_secs(rule.duration as u64);
        let now = Instant::now();
        let mut windows = self.windows.lock().expect("rate limiter poisoned");
        let entry = windows
            .entry((rule.label.clone(), client_id.to_string()))
            .or_insert(Window {
                started: now,
                count: 0,
            });
        if now.duration_since(entry.started) >= window {
            *entry = Window {
                started: now,
                count: 0,
            };
        }
        entry.count += 1;
        if entry.count > rule.max_requests {
            return Err(AppError::too_many_requests());
        }
        Ok(())
    }

    /// Number of live windows; used by tests.
    pub fn tracked_keys(&self) -> usize {
        self.windows.lock().expect("rate limiter poisoned").len()
    }
}

impl std::fmt::Debug for RateLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimiter")
            .field("enabled", &self.is_enabled())
            .field("windows", &self.tracked_keys())
            .finish()
    }
}

/// The tags a request carries, derived from the URL shape so the limiter
/// works independently of which routes are registered.
///
/// PocketBase attaches these at route-registration time; deriving them
/// from the path gives the same labels (`*:auth`, `posts:list`, ...)
/// without the routes having to exist yet. A service may add extra tags through
/// [`RouteTags`] for anything not visible in the URL.
pub fn tags_for(app: &App, method: &axum::http::Method, path: &str) -> Vec<String> {
    use axum::http::Method;

    let mut tags = Vec::new();
    let mut segments = path.trim_matches('/').split('/');
    if segments.next() != Some("api") {
        return tags;
    }
    match segments.next() {
        Some("collections") => {}
        Some("files") => {
            tags.push("*:file".into());
            return tags;
        }
        _ => return tags,
    }
    let (Some(collection), Some(action)) = (segments.next(), segments.next()) else {
        return tags;
    };
    // The handler resolves this same segment by name *or* id
    // (`common::collection_of`) after axum has already percent-decoded
    // it, so the tag must be derived the identical way — otherwise a
    // `posts:list` rule is dodged by hitting the collection through its
    // id, or through a percent-encoded spelling of its name. `try_db`
    // (rather than `db`, which panics pre-bootstrap) falls back to the
    // raw decoded segment when there is no store to resolve against.
    let collection = decode_path_segment(collection);
    let collection = app
        .try_db()
        .and_then(|db| db.collections.get(&collection))
        .map(|c| c.name.clone())
        .unwrap_or(collection);
    let mut push = |name: &str| {
        tags.push(format!("*:{name}"));
        tags.push(format!("{collection}:{name}"));
    };
    match action {
        "records" => {
            let has_id = segments.next().is_some();
            match (method, has_id) {
                // HEAD is served by the same handler as GET (see the
                // `get()` route registration) and must carry the same
                // tags, or a collection-scoped `list`/`view` limit is
                // silently skipped for HEAD requests.
                (&Method::GET | &Method::HEAD, false) => push("list"),
                (&Method::GET | &Method::HEAD, true) => push("view"),
                (&Method::POST, _) => push("create"),
                (&Method::PATCH, _) => push("update"),
                (&Method::DELETE, _) => push("delete"),
                _ => {}
            }
        }
        "auth-with-password" | "auth-with-oauth2" | "auth-refresh" | "auth-with-otp" => {
            push("auth");
            push(&lower_camel(action));
        }
        "request-otp"
        | "request-password-reset"
        | "confirm-password-reset"
        | "request-verification"
        | "confirm-verification"
        | "request-email-change"
        | "confirm-email-change"
        | "impersonate" => push(&lower_camel(action)),
        _ => {}
    }
    tags
}

/// `auth-with-password` → `authWithPassword`, PocketBase's tag spelling.
fn lower_camel(kebab: &str) -> String {
    let mut out = String::with_capacity(kebab.len());
    let mut upper_next = false;
    for c in kebab.chars() {
        if c == '-' {
            upper_next = true;
        } else if upper_next {
            out.extend(c.to_uppercase());
            upper_next = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// Percent-decode a single URL path segment (`%XX` bytes only — unlike
/// form encoding, a path segment does not treat `+` as a space). This
/// mirrors what axum's `Path<String>` extractor already did before the
/// handler saw the segment, so [`tags_for`] resolves the *same* string
/// [`crate::routes::common::collection_of`] does.
fn decode_path_segment(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The axum layer. Mounted inside the `/api` nest so plugin routes are
/// covered too (the audit found they bypassed both this and logging).
pub async fn rate_limit(State(app): State<App>, req: Request, next: Next) -> Response {
    let limiter = app.rate_limiter();
    // One atomic load, and nothing is allocated when limits are off.
    if !limiter.is_enabled() {
        return next.run(req).await;
    }

    let (mut parts, body) = req.into_parts();
    // Already resolved and cached by the logging layer on most requests;
    // this is a map lookup then.
    let auth = crate::extract::resolve_and_cache(&mut parts, &app).await;
    let settings = app.settings();
    let peer = parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0);
    let ip = client_ip::client_ip(&parts.headers, peer, &settings.trusted_proxy);
    // Rule labels are written against the full URL (`/api/batch`), so the
    // nest-stripped `parts.uri` is not what to match on.
    let path = crate::middleware::original_uri(&parts).path().to_string();

    let mut tags = tags_for(&app, &parts.method, &path);
    if let Some(extra) = parts.extensions.get::<RouteTags>() {
        tags.extend(extra.0.iter().cloned());
    }

    let client_id = match &auth {
        Some(a) => format!("@{}", a.id),
        None => ip.clone(),
    };
    if let Err(e) = limiter.check(&path, &tags, auth.is_some(), &client_id, &ip) {
        return ApiError(e).into_response();
    }

    next.run(Request::from_parts(parts, body)).await
}

#[cfg(test)]
mod tests {
    use axum::http::Method;
    use cratebase_core::{Collection, CollectionType, Field, FieldKind};

    use super::*;
    use crate::config::Config;
    use crate::extract::RequestInfo;
    use crate::routes::collections;

    fn rule(label: &str, audience: &str, max: i64, duration: i64) -> RateLimitRule {
        RateLimitRule {
            label: label.into(),
            audience: audience.into(),
            max_requests: max,
            duration,
        }
    }

    fn limits(rules: Vec<RateLimitRule>) -> RateLimits {
        RateLimits {
            rules,
            excluded_ips: vec![],
            enabled: true,
        }
    }

    fn compiled(rules: Vec<RateLimitRule>) -> Compiled {
        Compiled::build(&limits(rules))
    }

    /// A bootstrapped app over an in-memory database, mirroring the
    /// per-file test harnesses used elsewhere in this crate (see
    /// `routes::schema`'s test module).
    async fn test_app() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::new(Config::memory(dir.path()));
        app.bootstrap().await.expect("bootstrap");
        (app, dir)
    }

    async fn create_collection(app: &App, name: &str, fields: Vec<Field>) -> Collection {
        let mut next = Collection::new(name, CollectionType::Base);
        next.fields = fields;
        collections::prepare_new(&mut next);
        collections::validate(app, &next, None, "test").unwrap();
        let info = RequestInfo::default();
        (*collections::apply(app, next, None, collections::Change::Create, &info, None)
            .await
            .unwrap())
        .clone()
    }

    fn text_field(name: &str) -> Field {
        Field::new(
            name,
            FieldKind::Text {
                min: 0,
                max: 0,
                pattern: String::new(),
                autogenerate_pattern: String::new(),
                primary_key: false,
            },
        )
    }

    #[test]
    fn exact_path_beats_prefix_beats_tag() {
        let c = compiled(vec![
            rule("*:create", "", 1, 60),
            rule("/api/", "", 2, 60),
            rule("/api/collections/", "", 3, 60),
            rule("/api/batch", "", 4, 60),
        ]);
        let tags = vec!["*:create".to_string()];
        assert_eq!(c.find("/api/batch", &tags, false).unwrap().max_requests, 4);
        assert_eq!(
            c.find("/api/collections/posts/records", &tags, false)
                .unwrap()
                .max_requests,
            3,
            "longest prefix wins"
        );
        assert_eq!(c.find("/api/health", &tags, false).unwrap().max_requests, 2);

        let only_tag = compiled(vec![rule("*:create", "", 1, 60)]);
        assert_eq!(
            only_tag.find("/other", &tags, false).unwrap().max_requests,
            1
        );
        assert!(only_tag.find("/other", &[], false).is_none());
    }

    #[test]
    fn audience_filters_the_match() {
        let c = compiled(vec![
            rule("/api/", "@auth", 5, 60),
            rule("/api/", "@guest", 1, 60),
        ]);
        assert_eq!(c.find("/api/health", &[], true).unwrap().max_requests, 5);
        assert_eq!(c.find("/api/health", &[], false).unwrap().max_requests, 1);

        // A rule for everyone is the fallback when no specific one exists.
        let c = compiled(vec![rule("/api/", "", 7, 60)]);
        assert_eq!(c.find("/api/x", &[], true).unwrap().max_requests, 7);
        assert_eq!(c.find("/api/x", &[], false).unwrap().max_requests, 7);

        // An unknown audience matches nobody.
        let c = compiled(vec![rule("/api/", "@nonsense", 1, 60)]);
        assert!(c.find("/api/x", &[], false).is_none());
        assert!(c.find("/api/x", &[], true).is_none());
    }

    #[test]
    fn a_window_allows_max_requests_then_rejects() {
        let limiter = RateLimiter::new();
        limiter.configure(&limits(vec![rule("/api/", "", 2, 60)]));
        for _ in 0..2 {
            limiter
                .check("/api/health", &[], false, "1.2.3.4", "1.2.3.4")
                .unwrap();
        }
        let err = limiter
            .check("/api/health", &[], false, "1.2.3.4", "1.2.3.4")
            .unwrap_err();
        assert_eq!(err.status(), 429);
        assert_eq!(err.body().message, "Too Many Requests.");

        // A different client has its own window.
        limiter
            .check("/api/health", &[], false, "5.6.7.8", "5.6.7.8")
            .unwrap();
    }

    #[test]
    fn zero_and_disabled_limits_never_reject() {
        let limiter = RateLimiter::new();
        limiter.configure(&limits(vec![rule("/api/", "", 1, 0)]));
        for _ in 0..5 {
            limiter.check("/api/x", &[], false, "ip", "ip").unwrap();
        }
        let mut off = limits(vec![rule("/api/", "", 1, 60)]);
        off.enabled = false;
        limiter.configure(&off);
        for _ in 0..5 {
            limiter.check("/api/x", &[], false, "ip", "ip").unwrap();
        }
        assert_eq!(limiter.tracked_keys(), 0);
    }

    #[test]
    fn excluded_ips_are_never_counted() {
        let limiter = RateLimiter::new();
        let mut l = limits(vec![rule("/api/", "", 1, 60)]);
        l.excluded_ips = vec!["10.0.0.1".into()];
        limiter.configure(&l);
        for _ in 0..10 {
            limiter
                .check("/api/x", &[], false, "10.0.0.1", "10.0.0.1")
                .unwrap();
        }
        assert_eq!(limiter.tracked_keys(), 0);
    }

    #[test]
    fn reconfiguring_resets_the_counters() {
        let limiter = RateLimiter::new();
        limiter.configure(&limits(vec![rule("/api/", "", 1, 60)]));
        limiter.check("/api/x", &[], false, "ip", "ip").unwrap();
        assert_eq!(limiter.tracked_keys(), 1);
        limiter.configure(&limits(vec![rule("/api/", "", 5, 60)]));
        assert_eq!(limiter.tracked_keys(), 0);
        limiter.check("/api/x", &[], false, "ip", "ip").unwrap();
    }

    #[tokio::test]
    async fn tags_follow_the_url_shape() {
        let (app, _dir) = test_app().await;
        assert_eq!(
            tags_for(&app, &Method::GET, "/api/collections/posts/records"),
            ["*:list", "posts:list"]
        );
        assert_eq!(
            tags_for(&app, &Method::GET, "/api/collections/posts/records/abc"),
            ["*:view", "posts:view"]
        );
        assert_eq!(
            tags_for(&app, &Method::POST, "/api/collections/posts/records"),
            ["*:create", "posts:create"]
        );
        assert_eq!(
            tags_for(&app, &Method::PATCH, "/api/collections/posts/records/abc"),
            ["*:update", "posts:update"]
        );
        assert_eq!(
            tags_for(&app, &Method::DELETE, "/api/collections/posts/records/abc"),
            ["*:delete", "posts:delete"]
        );
        assert_eq!(
            tags_for(
                &app,
                &Method::POST,
                "/api/collections/users/auth-with-password"
            ),
            [
                "*:auth",
                "users:auth",
                "*:authWithPassword",
                "users:authWithPassword"
            ]
        );
        assert_eq!(
            tags_for(&app, &Method::GET, "/api/files/posts/abc/x.png"),
            ["*:file"]
        );
        assert!(tags_for(&app, &Method::GET, "/api/health").is_empty());
        assert!(tags_for(&app, &Method::GET, "/_/index.html").is_empty());
    }

    #[tokio::test]
    async fn per_collection_tag_rule_is_scoped_to_that_collection() {
        // A rule labelled `posts:list` is the URL-shape tag `tags_for`
        // derives only for GET requests against `posts`'s records
        // endpoint (see the doc comment on `tags_for`); an identically
        // shaped request against a different collection carries the tag
        // `comments:list` instead, which this rule set has no rule for.
        // This is the actual "per-collection" guarantee: not merely a
        // path prefix (which `/api/collections/posts/` would give too,
        // and which would still match `/api/collections/posts/records`
        // exactly the same for every verb), but one rule that fires for
        // one collection's one action and never for another's.
        let (app, _dir) = test_app().await;
        let posts = create_collection(&app, "posts", vec![text_field("title")]).await;
        let limiter = RateLimiter::new();
        limiter.configure(&limits(vec![rule("posts:list", "", 1, 60)]));

        let posts_tags = tags_for(&app, &Method::GET, "/api/collections/posts/records");
        let comments_tags = tags_for(&app, &Method::GET, "/api/collections/comments/records");
        assert_eq!(posts_tags, ["*:list", "posts:list"]);
        assert_eq!(comments_tags, ["*:list", "comments:list"]);

        // First request against `posts` is within budget.
        limiter
            .check(
                "/api/collections/posts/records",
                &posts_tags,
                false,
                "1.2.3.4",
                "1.2.3.4",
            )
            .unwrap();
        // Second request against `posts` from the same client blows the
        // one-request budget: the rule fires.
        let err = limiter
            .check(
                "/api/collections/posts/records",
                &posts_tags,
                false,
                "1.2.3.4",
                "1.2.3.4",
            )
            .unwrap_err();
        assert_eq!(err.status(), 429);

        // The identically shaped endpoint on a different collection, same
        // client, is untouched — no rule matches its `comments:list` tag
        // and its `posts:list` window was never shared.
        for _ in 0..5 {
            limiter
                .check(
                    "/api/collections/comments/records",
                    &comments_tags,
                    false,
                    "1.2.3.4",
                    "1.2.3.4",
                )
                .unwrap();
        }

        // The bug this regression-tests: the tag used to be built straight
        // from the raw URL segment, so a `posts:list` rule was dodged
        // entirely by addressing the *same* collection through its id
        // (axum's `Path<String>` and the real handler's
        // `routes::common::collection_of` both accept name or id) or
        // through a percent-encoded spelling of its name. A fresh client
        // hitting either variant must still be scoped to the same
        // `posts:list` window as the name-addressed route above.
        let by_id = format!("/api/collections/{}/records", posts.id);
        let id_tags = tags_for(&app, &Method::GET, &by_id);
        assert_eq!(
            id_tags,
            ["*:list", "posts:list"],
            "addressing the collection by id must resolve to the same posts:list tag"
        );
        limiter
            .check(&by_id, &id_tags, false, "9.9.9.9", "9.9.9.9")
            .unwrap();
        let err = limiter
            .check(&by_id, &id_tags, false, "9.9.9.9", "9.9.9.9")
            .unwrap_err();
        assert_eq!(
            err.status(),
            429,
            "a posts:list rule must not be dodged by addressing the collection by id"
        );

        let encoded = "/api/collections/po%73ts/records"; // %73 = 's'
        let encoded_tags = tags_for(&app, &Method::GET, encoded);
        assert_eq!(
            encoded_tags,
            ["*:list", "posts:list"],
            "a percent-encoded collection name must still resolve to posts:list"
        );
        limiter
            .check(encoded, &encoded_tags, false, "8.8.8.8", "8.8.8.8")
            .unwrap();
        let err = limiter
            .check(encoded, &encoded_tags, false, "8.8.8.8", "8.8.8.8")
            .unwrap_err();
        assert_eq!(
            err.status(),
            429,
            "a posts:list rule must not be dodged by percent-encoding the collection name"
        );

        // HEAD is served by the same `get()` handler as GET on the list
        // route and must carry the identical tags, or the rule silently
        // never applies to it.
        let head_tags = tags_for(&app, &Method::HEAD, "/api/collections/posts/records");
        assert_eq!(head_tags, posts_tags, "HEAD must be tagged like GET");
        let view_get = tags_for(&app, &Method::GET, "/api/collections/posts/records/abc");
        let view_head = tags_for(&app, &Method::HEAD, "/api/collections/posts/records/abc");
        assert_eq!(
            view_head, view_get,
            "HEAD on the view route must match GET too"
        );
    }
}
