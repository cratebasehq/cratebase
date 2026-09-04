//! `GET /api/collections/{name}/tool-schema` — a plain REST endpoint that
//! hands back the same JSON-Schema / OpenAI-function-calling-shaped
//! description of a collection the MCP server (`crate::mcp`) uses to
//! build its `create_<collection>` tool definition, for callers that
//! want the shape without speaking MCP at all.
//!
//! The conversion itself lives once, on
//! [`cratebase_core::Collection::to_json_schema`], and both this route
//! and `crate::mcp::build_tools` call it — see that method's doc comment
//! for the reasoning.
//!
//! Gated the same way the full collection schema is (`GET
//! /api/collections/{id}`, see `routes::collections::view`): the field
//! list is schema, not data, and PocketBase already treats schema reads
//! as a superuser-only operation.

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::Value;

use crate::app::App;
use crate::extract::RequireSuperuser;
use crate::http_error::ApiResult;
use crate::routes::common;

pub fn router() -> Router<App> {
    Router::new().route("/collections/{name}/tool-schema", get(tool_schema))
}

async fn tool_schema(
    State(app): State<App>,
    _su: RequireSuperuser,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    let collection = common::collection_of(&app, &name)?;
    Ok(Json(collection.to_json_schema()))
}
