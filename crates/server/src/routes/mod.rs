//! The `/api` router.
//!
//! Everything under `/api`, mounted inside the logging and rate-limit
//! layers.

pub mod api_keys;
pub mod auth;
pub mod backups;
pub mod batch;
pub mod collections;
pub mod common;
pub mod crons;
pub mod file_manager;
pub mod files;
pub mod health;
pub mod llm;
pub mod logs;
pub mod metrics;
pub mod push;
pub mod records;
pub mod schema;
pub mod settings;
pub mod setup;
pub mod sql_console;
pub mod tool_schema;
pub mod utils;

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
        .merge(crons::router())
        // `/api/collections/...` is shared by three groups: the schema
        // API, the record API nested under it, and the auth endpoints.
        // They are separate modules but one route table, built once at
        // boot.
        .merge(collections::router())
        .merge(records::router())
        .merge(auth::router())
        .merge(files::router())
        .merge(crate::realtime::router())
        .merge(batch::router())
        .merge(setup::router())
        .merge(utils::router())
        .merge(file_manager::router())
        .merge(sql_console::router())
        .merge(llm::router())
        .merge(tool_schema::router())
        .merge(schema::router())
        .merge(api_keys::router())
        .merge(push::router())
        // MCP lives in its own top-level module (like `realtime`), not
        // under `routes`, because it is a protocol server (JSON-RPC +
        // SSE) rather than a plain REST route table.
        .merge(crate::mcp::router());

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
