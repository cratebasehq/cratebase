/** `GET /api/collections/{name}/tool-schema` — a JSON-Schema /
 * OpenAI-function-calling-shaped description of a collection's
 * writable, non-system fields (see
 * `crates/server/src/routes/tool_schema.rs` and
 * `Collection::to_json_schema` in `crates/core/src/collection.rs`).
 *
 * The same conversion backs the Model Context Protocol server at
 * `GET`/`POST /api/mcp` (`crates/server/src/mcp.rs`) — this endpoint
 * hands back the identical shape for a caller that wants it without
 * speaking MCP/JSON-RPC at all, e.g. to build a `tools` array for an
 * LLM chat-completions call by hand.
 *
 * Superuser-gated, the same as reading a collection's full schema
 * (`GET /api/collections/{id}`) — `pb` must be authenticated as a
 * superuser.
 */

import type PocketBase from "pocketbase";
import type { SendOptions } from "pocketbase";

/** The shape returned by `/tool-schema`, matching an OpenAI
 * function-calling tool definition one-for-one. */
export interface ToolSchema {
  name: string;
  description: string;
  parameters: {
    type: "object";
    properties: Record<string, unknown>;
    required: string[];
  };
}

/** Fetch one collection's tool schema.
 *
 * ```ts
 * import PocketBase from "pocketbase";
 * import { getToolSchema } from "@cratebase/extras";
 *
 * const pb = new PocketBase("http://127.0.0.1:8090");
 * await pb.collection("_superusers").authWithPassword(email, password);
 *
 * const schema = await getToolSchema(pb, "posts");
 * // { name: "posts", description: "The 'posts' base collection (3 fields).",
 * //   parameters: { type: "object", properties: { title: {...}, ... }, required: ["title"] } }
 * ```
 */
export async function getToolSchema(
  pb: PocketBase,
  collectionIdOrName: string,
  options: SendOptions = {},
): Promise<ToolSchema> {
  return pb.send<ToolSchema>(
    `/api/collections/${encodeURIComponent(collectionIdOrName)}/tool-schema`,
    { method: "GET", ...options },
  );
}

/** Fetch several collections' tool schemas at once — the array shape an
 * LLM chat-completions call's `tools` field expects directly.
 *
 * ```ts
 * const tools = await getToolSchemas(pb, ["docs", "chunks"]);
 * ```
 */
export async function getToolSchemas(
  pb: PocketBase,
  collectionIdsOrNames: string[],
  options: SendOptions = {},
): Promise<ToolSchema[]> {
  return Promise.all(collectionIdsOrNames.map((name) => getToolSchema(pb, name, options)));
}
