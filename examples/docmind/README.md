# DocMind — Cratebase's AI-flagship bundle, end to end

The worked example scoped in
`docs/superpowers/specs/2026-09-04-value-add-strategy.md` §8: a 20-person
startup's internal knowledge-base copilot, built entirely on collections +
one `pb_hooks` file — no separate vector DB, no separate RAG service, no
separate chat backend. It ties together every item in the doc's §1a
"AI-flagship" bundle in one app instead of four independent checkboxes:

1. Employees upload plain-text documents through a normal app UI →
   Cratebase file storage holds the raw file, and a `pb_hooks` JS hook
   chunks the text and writes `{docId, text}` rows to a `chunks`
   collection whose `vector` field auto-embeds on write.
2. An employee asks a question → a custom endpoint does an
   application-side cosine-similarity search over `chunks`
   (`?nearestTo=`), calls the existing `POST /api/llm/chat` gateway with
   the matched chunks as context, streams the answer back over the
   *existing* SSE realtime infrastructure, and persists the exchange to a
   `messages` collection scoped to that employee.

## Why plain HTML/JS here, not Vite + React (like `examples/kanban`)

This UI is a form, a list, and a chat box — no drag-and-drop, no
multi-tab optimistic state reconciliation, nothing that benefits from a
component tree or real state management the way kanban's board does.
`index.html` + `app.js` with an [import
map](https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/script/type/importmap)
resolving `"pocketbase"` to the official SDK on esm.sh — the same
zero-build convention as `examples/todo` and `examples/realtime-chat` —
is the right amount of machinery for what this page actually does.

## Why no `@cratebase/extras`

`@cratebase/extras` ships `nearestTo` and `chat` helpers for exactly this
kind of RAG flow, and this example deliberately doesn't import it. The
reason is structural, not stylistic: turning a raw employee question into
a *query vector* requires writing a throwaway record through the real
auto-embedding pipeline (see "Why a throwaway record" in
`pb_hooks/docmind.pb.js`), which needs privileges an ordinary `users`
account doesn't have. That has to happen server-side, so all of the
retrieval + chat-gateway orchestration lives in one custom `pb_hooks`
endpoint (`POST /docmind/ask`) — the W8 "BFF" pattern the spec doc's §6
already validates (`routerAdd` + `HostApi` record reads composing several
calls into one response). The *client* is left with exactly one custom
`fetch`-shaped call (`cb.send("/docmind/ask", ...)`) and the stock SDK's
`pb.realtime.subscribe`, both of which the official `pocketbase` package
already covers — there is nothing left for `@cratebase/extras` to do on
the client side of this particular flow. `app.js`'s realtime subscription
code is, in effect, `@cratebase/extras`'s own `chat()` helper written out
by hand against the custom endpoint instead of `/api/llm/chat` directly.

## 1. Start Cratebase

From the repo root, build once if needed:

```bash
cargo build --release -p cratebase-server --bin cratebase
```

This example needs two extra environment variables beyond every other
example's plain `cargo run ... -- serve` — both explained in full in
`pb_hooks/docmind.pb.js`'s module doc, summarized here:

```bash
CB_HOOKS_DIR="$(pwd)/examples/docmind/pb_hooks" \
  DOCMIND_DATA_DIR="$(pwd)/pb_data" \
  cargo run -p cratebase-server --bin cratebase -- serve
```

- **`CB_HOOKS_DIR`** — every other example has no `pb_hooks/` at all, so
  the server's default (`<dataDir>/../pb_hooks`) is irrelevant to them.
  This example's hooks live under `examples/docmind/pb_hooks/`, not next
  to whatever `pb_data` directory the server is using, so it has to be
  pointed there explicitly.
- **`DOCMIND_DATA_DIR`** — must match whatever data directory the server
  actually uses (`--dir`, `CRATEBASE_DATA_DIR`, or the `./pb_data`
  default). The `docs` hook reads an uploaded file straight off local
  disk (`<dataDir>/storage/<collectionId>/<recordId>/<filename>`) instead
  of over HTTP — see the hook file for exactly why — and has no other way
  to learn where that directory is.

This serves the API at `http://localhost:8090` (the URL `app.js` is
hardcoded to point at — edit `BASE_URL` in `app.js` if you're running it
elsewhere, or override `meta.appURL` — see step 2 — if you're pointing
`setup.sh` at a different host/port).

## 2. Provision the `docs`, `chunks`, and `messages` collections

```bash
bun run examples:docmind:setup
# or directly: bash examples/docmind/setup.sh
```

This upserts an `admin@example.com` / `changeme123` superuser (override
with `ADMIN_EMAIL`/`ADMIN_PASSWORD`, same as every other example), sets
`meta.appURL` to `CRATEBASE_URL` (the hook and its cron job self-call the
API — see below — and need to know their own address), and
creates/updates three collections:

- **`docs`** — `title` (text), `file` (file, not `protected` — see the
  hook file's module doc for why that matters). `listRule`/`viewRule`/
  `createRule` are `@request.auth.id != ""` — any signed-in employee can
  upload and browse the shared knowledge base; `updateRule`/`deleteRule`
  are superuser-only.
- **`chunks`** — `docId` (relation → `docs`, optional), `text`,
  `chunkIndex` (number), `embedding` (`vector`, 64 dimensions,
  `embedding: {provider: "echo", model: "", sourceField: "text"}`).
  `createRule`/`updateRule`/`deleteRule` are all superuser-only — nothing
  but `pb_hooks/docmind.pb.js` ever writes here (with a freshly minted
  superuser token — see "Why a throwaway record"); any signed-in employee
  can list/view for their own client-side inspection.
- **`messages`** — `author` (relation → `users`, required), `prompt`,
  `response`, `model`. Every rule is `@request.auth.id != "" && author =
  @request.auth.id` — each employee sees only their own conversations,
  the per-employee scoping the spec's §8 story calls for.

Two collections (`chunks.docId`, `messages.author`) need a real
collection *id*, not a name, at creation time — `setup.sh` creates `docs`
first and reads its id back before building `chunks`. Safe to re-run:
`ensure_collection` (shared by every example) PATCHes rules back in line
on a second run and leaves fields/data alone.

Uses `EMBEDDINGS`/`LLM` zero-config fallbacks throughout — no API key
needed anywhere. `chunks.embedding`'s `provider: "echo"` is Cratebase's
deterministic, network-free embedding fake (`crates/server/src/
embeddings.rs::EchoProvider`); the LLM gateway falls back to its own
`EchoProvider` (`crates/server/src/llm.rs`) whenever `settings.llm.enabled`
is `false`, which is the default. Both are what every live-verification
run below actually exercised.

## 3. Serve the example

```bash
bun run examples:serve
```

Then open **`http://localhost:4173/examples/docmind/`**, register an
account (email + password, 8 characters minimum), upload a `.txt` file,
and ask a question about it.

## How it works

- **Auth**: `users.create(...)` + `authWithPassword(...)` to register,
  `authWithPassword` alone to sign in — identical to `examples/todo`.
- **Upload**: `docs.create(formData)` with a `FormData` carrying `title`
  and `file` — the SDK's normal multipart path, no special handling.
- **Chunking, immediately**: `pb_hooks/docmind.pb.js` binds
  `onRecordAfterCreateSuccess` on `docs`. It reads the just-uploaded
  file's bytes straight off local disk (not over HTTP — see the file's
  module doc for the transaction-visibility reason why), splits it into
  plain fixed-size 800-character pieces (no overlap, no sentence-aware
  logic — "simple fixed-size" is what the assignment asked for), and
  writes one `chunks` record per piece via `e.app.save()`. This is
  transaction-safe (same executor as the `docs` row's own still-open
  write) but does **not** compute an embedding — `$app.save()` bypasses
  the auto-embed pipeline entirely (see below).
- **Embedding, on the next cron tick**: a `docmind_index` cron job (every
  minute — `croner`'s finest resolution) finds every `chunks` row with an
  empty `embedding` and `PATCH`es it with `{text: <same text>}`, using a
  freshly minted superuser token. Re-supplying the source field is what
  makes `crate::embeddings::apply_embeddings` recompute the vector — and
  a real `PATCH /api/collections/chunks/records/{id}` call is the *only*
  code path that runs `apply_embeddings` at all in this server; neither
  `$app.save()` nor a `$http.send` call made from inside a still-open
  record-creation transaction can reach it (see the hook file's module
  doc — this took two live-verified failed attempts to pin down, worth
  reading if you're extending this file).
- **Retrieval + chat, in one custom endpoint**: `POST /docmind/ask`
  (`routerAdd`, gated by `$apis.requireAuth("users")`) mints a superuser
  token, round-trips the caller's question through `chunks` itself (a
  throwaway record with no `docId`, through the real create route so it
  gets a real "echo" vector back), ranks every other `chunks` row against
  it with `?nearestTo=embedding:<throwawayId>&nearestLimit=5`, deletes
  the throwaway row, then calls `POST /api/llm/chat` — as the *calling
  employee*, not the superuser — with the matched chunks as a numbered
  system-prompt context. The gateway's own optional `collection`
  persistence only writes `{prompt, response, model}` with no
  request-scoping field, so this endpoint persists the `messages` row
  itself afterward with an explicit `author: e.auth.id`, which is what
  actually makes `messages`' per-employee `authRule` mean something.
- **Streaming**: `app.js` calls `pb.realtime.subscribe("llm_chunk"/
  "llm_done"/"llm_error", ...)` once, on sign-in — these are not
  collection topics, they're the three realtime event names
  `crates/server/src/routes/llm.rs` sends over the caller's *existing*
  `GET /api/realtime` SSE connection, and the SDK's `subscribe` accepts
  any topic string. `POST /docmind/ask`'s body carries
  `pb.realtime.clientId` through to the internal `/api/llm/chat` call, so
  the chunks land on the same connection the page already opened. The
  `POST /docmind/ask` promise's own resolved `reply` is still what
  finally renders — the streamed deltas are a live preview, not the
  source of truth.

## Verification status

**Fully live-verified**, end to end, against the prebuilt release binary
(`cargo build --release -p cratebase-server --bin cratebase`) on an
isolated port + data directory, using only the committed `setup.sh` and
`pb_hooks/docmind.pb.js` (no manual patching) — the run below started
from a completely empty data directory:

- `bash examples/docmind/setup.sh` against a fresh instance: created
  `docs`, `chunks`, `messages` and set `meta.appURL`. Re-ran it a second
  time: PATCHed the same rules back (idempotent, confirmed by
  `Updated`/no-error output on both `docs`/`chunks`/`messages`).
- Registered a `users` account via curl, uploaded a real 146-byte `.txt`
  file to `docs` (multipart). A `chunks` row appeared **immediately**
  (same request) with `docId`/`text` populated and `embedding: []`.
  Waited for the `docmind_index` cron tick (confirmed in server logs:
  `INFO jsvm: docmind: backfilled embeddings count=1`); re-queried the
  same `chunks` row and its `embedding` was a populated 64-float array.
- Uploaded a second, differently-worded document and a long (3.1KB,
  ~400-word) synthetic document; confirmed the long one split into
  exactly 4 chunks (800/800/800/729 characters) immediately on upload,
  all four backfilled with a 64-float `embedding` on the next tick.
- **Deterministic retrieval check** (the point of using the echo
  provider — identical text always embeds to an identical vector, so
  cosine similarity to *itself* is exactly 1.0): called `POST
  /docmind/ask` with a question string set to the *exact* text of one
  chunk. `sources[0]` was that exact chunk, verbatim, ranked above the
  unrelated second document's chunk. Repeated with a question matching
  the *other* document's chunk text — the ranking flipped, confirming
  this isn't a fixed/accidental ordering. Also verified the optional
  `docId` scope parameter: passing it returned exactly one source, from
  that document only, out of six embedded chunks across three documents
  at the time.
- Confirmed the full gateway round trip: `reply` in the response matched
  the echo LLM provider's deterministic output, `promptTokens`/
  `completionTokens` were populated, and a `messages` row was persisted
  with the correct `author`, `prompt`, and `response`. The throwaway
  query-embedding `chunks` record was gone afterward (`DELETE` ran) —
  confirmed by an unchanged total `chunks` count across two `/docmind/ask`
  calls.
- **Per-employee scoping**: registered a second `users` account
  (`bob`/`dana`-style test accounts) and confirmed it could `list` every
  `docs` row (the shared knowledge base) but zero `messages` rows —
  the first account's conversations stayed private, matching `messages`'
  `authRule`.
- `POST /docmind/ask` from an unauthenticated request: `401`.
- **Browser UI**, driven headless end to end against a real running
  server (a static file server serving just `examples/docmind/`, with
  `app.js`'s `BASE_URL` temporarily pointed at the isolated test
  instance, reverted to the documented `http://localhost:8090` default
  before this file was finalized): registered an account through the
  actual form, uploaded a `.txt` file through the actual file input
  (`docList` updated immediately with the new title), asked a question
  matching an uploaded chunk's exact text, and read the rendered
  `#answer`/`#sources` DOM after the request settled — the answer text
  and the top two ranked sources matched the curl-driven API results
  exactly, with no error banner shown.

**Not separately verified**: `bun run examples:serve` itself (verified
via an equivalent standalone static server instead, for isolation from
another concurrently-running dev instance on the shared default port);
live token-by-token streaming rendering mid-flight (the SSE plumbing and
subscription code path were verified working — `pb.realtime.subscribe`
successfully connects and the gateway's `llm_chunk`/`llm_done` events are
real, existing infrastructure already proven by `examples/kanban`'s
presence feature and this repo's own conformance suite — but a
frame-by-frame capture of the cursor animation mid-stream was not
captured, only the settled final state); a genuine PDF/DOCX upload (the
chunker intentionally only understands plain text, as scoped by the
assignment — noted in the hook file's own comments, not silently
assumed); S3-backed file storage (the hook's direct disk read only covers
the default local-storage backend — noted as a known limitation in the
hook file, not fixed here).
