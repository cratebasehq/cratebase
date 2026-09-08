# @cratebase/extras

Optional client extensions for the handful of [Cratebase](https://github.com/cratebasehq/cratebase)
endpoints the official [`pocketbase`](https://www.npmjs.com/package/pocketbase) npm SDK has no
first-class surface for: vector search, MCP tool schemas, and the LLM chat gateway.

This surface is available two ways. `@cratebase/client`, Cratebase's first-party SDK, has it
built in — no extra package needed, and it's the default recommendation for new projects. This
package, `@cratebase/extras`, is the other way: for projects that stay on the official
`pocketbase` SDK, it bolts the same methods on top of an existing `PocketBase` client instance
without replacing or forking it — see `docs/superpowers/specs/2026-09-04-value-add-strategy.md`
§1 for the original reasoning (superseded as the default path, still valid for this audience). A
project that only wants PocketBase-parity behavior never installs `@cratebase/extras` and pays
zero cost. A project that wants the extra surface on the `pocketbase` SDK adds one dependency on
top of the client it already has, and passes that existing client instance in — nothing here
constructs its own connection or duplicates auth/session handling.

```
bun add pocketbase @cratebase/extras
```

`pocketbase` is a peer dependency (pinned to `^0.28.0`, the version this package was built and
verified against) — you bring your own `PocketBase` instance, already authenticated however your
app normally authenticates it.

Every helper is available two ways: as a standalone function (`nearestTo`, `getToolSchema(s)`,
`chat`) taking the `PocketBase` client as its first argument, or bundled as methods on a
`CratebaseExtras` facade that holds the client for you. Pick whichever fits your call sites; they
call the exact same code.

```ts
import PocketBase from "pocketbase";
import { CratebaseExtras } from "@cratebase/extras";

const pb = new PocketBase("http://127.0.0.1:8090");
await pb.collection("_superusers").authWithPassword(email, password);

const extras = new CratebaseExtras(pb);
```

## Vector search — `nearestTo`

Wraps `?nearestTo=` on the records list endpoint: application-side cosine-similarity ranking over
a `vector` field (`FieldKind::Vector` in `crates/core/src/field.rs`, ranking logic in
`crates/server/src/embeddings.rs`). There is no native ANN index behind this in this pass — it's a
Rust-side rank over the rows your `listRule`/`?filter=` would already return, capped at 20,000
candidate rows server-side. Fine for app-scale collections (thousands to low millions of vectors);
not a substitute for a dedicated vector DB at massive scale.

`to` is either the query vector itself (a `number[]` matching the field's configured
`dimensions`) or another record's id — whose own value on `field` becomes the query vector,
subject to that record's `viewRule`.

```ts
import { nearestTo } from "@cratebase/extras";

// by an explicit query vector, scoped to one document with `filter`
const results = await nearestTo(pb, "chunks", "embedding", [0.12, -0.4, 0.91, /* ... */], {
  limit: 5,
  filter: 'docId = "abc123"',
});

// "more like this record" — reuse an existing row's own vector
const similar = await nearestTo(pb, "chunks", "embedding", "RECORD_ID_HERE", { limit: 5 });

// via the facade
const viaExtras = await extras.nearestTo("chunks", "embedding", [0.12, -0.4, 0.91], { limit: 5 });
```

`results` is an ordinary PocketBase `ListResult<T>` — same shape `getList()` returns, just ranked
by similarity instead of `sort`, and always exactly one page.

## MCP tool schemas — `getToolSchema` / `getToolSchemas`

Wraps `GET /api/collections/{name}/tool-schema` (`crates/server/src/routes/tool_schema.rs`): the
same JSON-Schema / OpenAI-function-calling-shaped description of a collection that the MCP server
at `GET`/`POST /api/mcp` (`crates/server/src/mcp.rs`) uses to build its own tool definitions —
useful when you want to hand an LLM a `create_<collection>`-style structured-output target without
speaking MCP/JSON-RPC at all. Superuser-gated, same tier as reading a collection's full schema.

```ts
import { getToolSchema, getToolSchemas } from "@cratebase/extras";

const postsSchema = await getToolSchema(pb, "posts");
// {
//   name: "posts",
//   description: "The 'posts' base collection (3 fields).",
//   parameters: { type: "object", properties: { title: {...}, body: {...}, tags: {...} }, required: ["title"] }
// }

// build a chat-completions `tools` array directly
const tools = await getToolSchemas(pb, ["docs", "chunks"]);

// via the facade
const viaExtras = await extras.getToolSchema("posts");
```

## LLM chat gateway — `chat`

Wraps `POST /api/llm/chat` (`crates/server/src/routes/llm.rs`): one chat completion against the
provider configured in the server's `settings.llm`. The HTTP call only resolves once the provider
finishes generating — incremental chunks arrive over your `PocketBase` client's *existing*
`GET /api/realtime` SSE connection as `llm_chunk` frames, which is why streaming is exposed as an
`onDelta` callback rather than an async-iterable response body.

```ts
import { chat } from "@cratebase/extras";

const result = await chat(pb, [{ role: "user", content: "Summarize the onboarding doc." }], {
  // persist {prompt, response, model} to this collection, subject to its own createRule
  collection: "messages",
  onDelta: (delta) => process.stdout.write(delta),
  onError: (message) => console.error("llm stream error:", message),
});

console.log(result.reply, result.promptTokens, result.completionTokens, result.record);

// via the facade, non-streamed (no realtime connection opened)
const plain = await extras.chat([{ role: "user", content: "hi" }]);
```

`onDelta`/`onError` require a runtime with a global `EventSource` (any browser out of the box;
Node needs a polyfill such as the [`eventsource`](https://www.npmjs.com/package/eventsource)
package assigned to `globalThis.EventSource`) — this is the same requirement `pb.realtime.subscribe`
already has for any other realtime use, not something this package adds. A plain, non-streamed
`chat()` call (omitting both callbacks) has no such requirement.

## Presence — `trackPresence`

"Who's online right now", built entirely on things Cratebase already ships: an ordinary
collection plus `GET /api/realtime` (`crates/server/src/realtime.rs`). There is no server-side
presence feature to enable — you define a `presence` collection in your own schema, and
`trackPresence` does the client-side heartbeat-and-observe pattern on top of it: it keeps your own
row's timestamp fresh on an interval, subscribes to every other row's changes, and maintains a
local `online` set that sweeps out any peer whose heartbeat has gone stale (closed tab, dead
network — no clean "offline" event ever arrives for those).

Create a `presence` collection first (e.g. via the dashboard or the Admin API) with fields
matching your own presence data — a minimal shape:

```json
{
  "name": "presence",
  "type": "base",
  "fields": [
    { "name": "userRef", "type": "text", "required": true },
    { "name": "status", "type": "text" },
    { "name": "lastSeenAt", "type": "date" }
  ],
  "listRule": "@request.auth.id != ''",
  "viewRule": "@request.auth.id != ''",
  "createRule": "@request.auth.id != '' && userRef = @request.auth.id",
  "updateRule": "@request.auth.id != '' && userRef = @request.auth.id",
  "deleteRule": "@request.auth.id != '' && userRef = @request.auth.id"
}
```

```ts
import PocketBase from "pocketbase";
import { trackPresence } from "@cratebase/extras";

const pb = new PocketBase("http://127.0.0.1:8090");
await pb.collection("users").authWithPassword(email, password);

const presence = await trackPresence(pb, "presence", {
  userRef: pb.authStore.record!.id,
  status: "online",
});

const unsubscribe = presence.subscribe((online) => {
  console.log(`${online.size} peers online:`, [...online]);
});

// later, e.g. on unmount or page unload:
unsubscribe();
await presence.stop();

// via the facade
const viaExtras = await extras.trackPresence("presence", { userRef: pb.authStore.record!.id });
```

`trackPresence`'s third argument upserts: pass `{ id, ...fields }` to keep refreshing a row you
already created, or fields with no `id` to create one on the first call and reuse it for every
heartbeat after. `heartbeatMs` (default 20s), `staleMs` (default `3 * heartbeatMs`) and
`lastSeenField` (default `"lastSeenAt"`) in the options object tune the timing to your schema and
how quickly a departed peer should read as offline.

## Versioning

This package versions independently from the `cratebase` server binary. It's an optional
client-side add-on with its own semver lifecycle — the `pocketbase` peer dependency it targets,
not the server release cadence, is what actually constrains breaking changes here. Publishing is
triggered by pushing a `extras-v<version>` tag (see `.github/workflows/publish-extras.yml`), kept
deliberately separate from the server's own `v*` release tags so a server release never forces an
unrelated extras publish, and an extras hotfix never waits on one.

## Development

```
bun install
bunx tsc --noEmit   # typecheck
bun run build        # emit dist/
```
