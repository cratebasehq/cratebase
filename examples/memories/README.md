# Agent memory service

A worked pattern for "an AI agent remembers things across sessions," built
entirely from primitives Cratebase already ships — vector search, auto-
embedding, MCP tool exposure, custom cron jobs, and realtime — with **zero
new server code**. This is the concrete version of the architecture sketched
mid-design: storage, semantic retrieval, per-user/per-agent scoping,
agent-writable-via-MCP, short-term expiry, and a live feed, each mapped to an
existing feature rather than a new one.

## 1. Import the collection

`memories-collection.json` in this directory is a ready-to-import collection
fixture (same format `GET /api/collections` returns, and what
`POST /api/schema/apply` or the dashboard's *Import collections* dialog
accepts):

```bash
curl -X POST http://localhost:8090/api/schema/apply \
  -H "authorization: $SUPERUSER_TOKEN" \
  -H "content-type: application/json" \
  --data-binary @memories-collection.json
```

Shape:

| Field | Type | Notes |
| --- | --- | --- |
| `content` | text | The memory itself, in plain language. |
| `vector` | vector (1536 dims) | Auto-embedded from `content` on every write — see §2. |
| `ownerRef` | relation → `users` | Whoever/whatever this memory belongs to (a human user *or* an agent's own service-account record). |
| `sessionRef` | text | Optional grouping key for a single conversation/run. |
| `expiresAt` | date | Optional. Unset = permanent; set = eligible for cleanup, see §4. |

Rules are owner-scoped end to end (`ownerRef = @request.auth.id`) — the same
shape `_mfas`/`_otps`/`_push_subscriptions` use for "a record manages only
its own rows," reused here for "an agent's memories are private to that
agent's identity." Retype the rule if your app's scoping is per-team
(`@collection._team_members...`) instead of per-user.

## 2. Semantic retrieval is automatic

The `vector` field's `embedding` config (`provider: "echo"`, `sourceField:
"content"`) means every create/update recomputes the embedding server-side —
"write text, get search" holds with no application code. Swap `"echo"` for a
real provider once one is configured in `settings.llm`
(`crates/server/src/embeddings.rs` — the same auto-embedding path the
vector-field feature ships for any collection).

Nearest-neighbour search from a client (via `@cratebase/extras`):

```ts
import { CratebaseExtras } from "@cratebase/extras";
const extras = new CratebaseExtras(pb);
const relevant = await extras.nearestTo("memories", "vector", queryVector, {
  filter: `ownerRef = "${agentUserId}"`,
  nearestLimit: 5,
});
```

## 3. An agent reads/writes its own memories via MCP

No extra wiring: every collection is automatically exposed as MCP tools
(`crates/server/src/mcp.rs`). Once the collection above is imported, `POST
/api/mcp {"method":"tools/list"}` already lists `list_memories`,
`get_memories`, `create_memories`, `update_memories`, `delete_memories` —
verified live against a real running server. An agent authenticates as an
ordinary auth record (an API key from the dashboard's *API keys* page is
just another identity) and gets exactly the rule-gated access defined above
— the same authz an MCP tool call gets for every other collection, nothing
bespoke for memory.

## 4. Short-term memory expires itself

A custom cron job (`_cron_jobs`, see the dashboard's *Cron jobs* page or
`POST /api/collections/_cron_jobs/records`) runs the cleanup on a schedule:

```json
{
  "name": "expire-memories",
  "expression": "0 3 * * *",
  "sql": "DELETE FROM memories WHERE expiresAt IS NOT NULL AND expiresAt != '' AND expiresAt < datetime('now')",
  "enabled": true
}
```

Verified live: creating the job, running it on demand
(`POST /api/crons/custom:<id>`), and confirming `lastStatus`/`lastMessage`
write back (`"success"` / `"0 row(s) affected"`) with no expired rows yet —
the exact same reactive-scheduler path every other custom cron job uses, no
restart required.

## 5. Realtime feed

Subscribe to the collection like any other — `pb.collection("memories").subscribe("*", cb)`
— to watch memories appear/expire live, e.g. to show an agent's "thinking"
trace in a UI. No new realtime surface; this is the same SSE fan-out every
collection already gets, filtered by the same owner rule above so a
subscriber only ever sees their own memories.

## What's deliberately not built here

Consolidation, deduplication, or summarization of memories over time is an
*application* concern layered on top of this substrate (e.g. a `pb_hooks` JS
hook on `onRecordCreate` that checks for near-duplicate vectors before
inserting) — the storage/retrieval/access-control/expiry substrate above is
what Cratebase provides; the memory-management policy on top of it is
product-specific and intentionally not opinionated here.
