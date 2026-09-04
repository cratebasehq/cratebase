/** `@cratebase/extras` — optional client extensions for the handful of
 * Cratebase endpoints the official `pocketbase` npm SDK has no
 * first-class surface for: vector search, MCP tool schemas, and the LLM
 * chat gateway (see
 * `docs/superpowers/specs/2026-09-04-value-add-strategy.md` §1/§1a).
 *
 * This package deliberately does **not** replace or fork the SDK — a
 * project that only wants PocketBase-parity behavior never installs it
 * and pays zero cost. Everything here takes an existing `PocketBase`
 * client instance and bolts extra methods onto it, either through the
 * {@link CratebaseExtras} facade or as the standalone functions it
 * wraps (`nearestTo`, `getToolSchema(s)`, `chat`), for callers who'd
 * rather not carry an extra object around.
 */

import type PocketBase from "pocketbase";
import type { RecordModel } from "pocketbase";

import { nearestTo, type NearestToOptions } from "./vector.js";
import { getToolSchema, getToolSchemas, type ToolSchema } from "./mcp.js";

export { nearestTo, type NearestToOptions } from "./vector.js";
export { getToolSchema, getToolSchemas, type ToolSchema } from "./mcp.js";

/** One turn of a chat exchange, mirroring the server's `WireMessage`
 * (`crates/server/src/routes/llm.rs`). */
export interface ChatMessage {
  role: string;
  content: string;
}

/** Options for {@link chat}. */
export interface ChatOptions {
  /** Persist the exchange to this collection after the reply completes,
   * subject to that collection's own `createRule` — evaluated against
   * `pb`'s current auth, exactly like a normal
   * `pb.collection(name).create()` would be. Omit to skip persistence. */
  collection?: string;
  /** Called with each incremental token/text delta as the provider
   * streams it. Wiring this up opens `pb`'s realtime connection (if not
   * already open) and subscribes to this one chat call's frames for the
   * duration of the request — see the module doc for what that requires
   * in a non-browser runtime. Omit for a plain non-streamed call. */
  onDelta?: (delta: string) => void;
  /** Called once if the provider fails mid-stream, with the same
   * message the rejected `chat()` promise carries — a convenience for
   * a UI that wants to react to the realtime frame before `await`
   * settles, not required for correctness (the promise still rejects
   * either way). */
  onError?: (message: string) => void;
}

/** The server's `ChatResponse`, camelCased as PocketBase always is. */
export interface ChatResult {
  reply: string;
  promptTokens: number;
  completionTokens: number;
  /** The persisted `{prompt, response, model}` record, present only
   * when `options.collection` was set. */
  record?: RecordModel;
}

/** `POST /api/llm/chat` — one chat completion against the provider
 * configured in the server's `settings.llm` (see
 * `crates/server/src/llm.rs`). The HTTP response only resolves once the
 * provider's stream ends; incremental chunks arrive over `pb`'s
 * *existing* `GET /api/realtime` SSE connection as `llm_chunk` frames,
 * which is why streaming needs `onDelta` rather than an async iterator
 * over the HTTP response body.
 *
 * Requires a runtime with a global `EventSource` (any browser; Node
 * needs a polyfill such as the `eventsource` package) *only* when
 * `onDelta`/`onError` is supplied — a plain non-streamed call has no
 * such requirement.
 *
 * ```ts
 * import PocketBase from "pocketbase";
 * import { chat } from "@cratebase/extras";
 *
 * const pb = new PocketBase("http://127.0.0.1:8090");
 * await pb.collection("users").authWithPassword(email, password);
 *
 * const result = await chat(
 *   pb,
 *   [{ role: "user", content: "Summarize the onboarding doc." }],
 *   {
 *     collection: "messages",
 *     onDelta: (delta) => process.stdout.write(delta),
 *   },
 * );
 * console.log(result.reply, result.promptTokens, result.completionTokens);
 * ```
 */
export async function chat(
  pb: PocketBase,
  messages: ChatMessage[],
  options: ChatOptions = {},
): Promise<ChatResult> {
  let unsubscribeChunk: (() => Promise<void>) | undefined;
  let unsubscribeDone: (() => Promise<void>) | undefined;
  let unsubscribeError: (() => Promise<void>) | undefined;
  let clientId: string | undefined;

  if (options.onDelta || options.onError) {
    unsubscribeChunk = await pb.realtime.subscribe("llm_chunk", (data: { delta: string }) => {
      options.onDelta?.(data.delta);
    });
    unsubscribeDone = await pb.realtime.subscribe("llm_done", () => {});
    unsubscribeError = await pb.realtime.subscribe("llm_error", (data: { message: string }) => {
      options.onError?.(data.message);
    });
    clientId = pb.realtime.clientId;
  }

  try {
    return await pb.send<ChatResult>("/api/llm/chat", {
      method: "POST",
      body: { messages, collection: options.collection, clientId },
    });
  } finally {
    await unsubscribeChunk?.();
    await unsubscribeDone?.();
    await unsubscribeError?.();
  }
}

/** A thin facade bundling every `@cratebase/extras` helper as methods on
 * one object, for callers who'd rather not import each function
 * separately. Wraps `pb` — never replaces it; every ordinary
 * `pb.collection(...)` call still goes straight to the SDK.
 *
 * ```ts
 * import PocketBase from "pocketbase";
 * import { CratebaseExtras } from "@cratebase/extras";
 *
 * const pb = new PocketBase("http://127.0.0.1:8090");
 * const extras = new CratebaseExtras(pb);
 *
 * const nearest = await extras.nearestTo("chunks", "embedding", queryVector, { limit: 5 });
 * const schema = await extras.getToolSchema("posts");
 * const reply = await extras.chat([{ role: "user", content: "hi" }]);
 * ```
 */
export class CratebaseExtras {
  private readonly pb: PocketBase;

  constructor(pb: PocketBase) {
    this.pb = pb;
  }

  nearestTo<T extends RecordModel = RecordModel>(
    collectionIdOrName: string,
    field: string,
    to: number[] | string,
    options?: NearestToOptions,
  ) {
    return nearestTo<T>(this.pb, collectionIdOrName, field, to, options);
  }

  getToolSchema(collectionIdOrName: string): Promise<ToolSchema> {
    return getToolSchema(this.pb, collectionIdOrName);
  }

  getToolSchemas(collectionIdsOrNames: string[]): Promise<ToolSchema[]> {
    return getToolSchemas(this.pb, collectionIdsOrNames);
  }

  chat(messages: ChatMessage[], options?: ChatOptions): Promise<ChatResult> {
    return chat(this.pb, messages, options);
  }
}
