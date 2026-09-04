//! The `/api` router.
//!
//! Only the services that do not depend on the record layer are mounted
//! here — W4a's scope. Every `// W4b:` comment marks where a
//! record-dependent route group will be nested once `cratebase_db`'s
//! `records`, `expand`, `validate` and `context` modules (W3) land. Those
//! routes are deliberately *absent* rather than stubbed with `todo!()`,
//! so a missing endpoint is a clean PocketBase 404 and never a panic.

pub mod backups;
pub mod crons;
pub mod health;
pub mod logs;
pub mod settings;

use axum::Router;

use crate::app::App;

/// The `/api` subtree, without the layers (`crate::router` adds those, so
/// they wrap plugin routes too).
pub fn api_router(app: &App) -> Router<App> {
    let mut router = Router::new()
        .merge(health::router())
        .merge(settings::router())
        .merge(logs::router())
        .merge(backups::router())
        .merge(crons::router());

    // W4b: `.merge(collections::router())` — /api/collections CRUD,
    //      import, scaffolds, truncate.
    // W4b: `.merge(records::router())` — /api/collections/{c}/records...
    // W4b: `.merge(auth::router())` — auth-with-password / oauth2 / otp /
    //      mfa / refresh / impersonate / verification / reset / email
    //      change / external auths, plus /api/collections/{c}/auth-methods.
    // W4b: `.merge(files::router())` — /api/files/{c}/{id}/{file} and
    //      /api/files/token.
    // W4b: `.merge(realtime::router())` — GET/POST /api/realtime.
    // W4b: `.merge(batch::router())` — POST /api/batch.

    // Plugin routes live inside this nest so they inherit request logging
    // and rate limiting (see `crate::plugin`).
    router = router.merge(app.plugin_router());

    // The nest needs its own fallbacks: without them an unmatched
    // `/api/...` path escapes to the outer router and is answered
    // *outside* the logging and rate-limit layers, so a 404 on an API
    // path would never be logged.
    //
    // Both fallbacks are the same handler because PocketBase answers a
    // wrong method on an existing route with 404, not 405.
    router
        .fallback(crate::http_error::not_found_fallback)
        .method_not_allowed_fallback(crate::http_error::not_found_fallback)
}
