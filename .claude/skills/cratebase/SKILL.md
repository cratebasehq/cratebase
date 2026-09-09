---
name: cratebase
description: Build apps on top of Cratebase, the self-hostable Rust backend (PocketBase-wire-compatible — dynamic collections, auth, file storage, realtime, filter-expression API rules, JS hooks). Use this whenever the user wants to design a collection schema, write or debug an API rule / filter expression, wire up the `pocketbase` JS/TS SDK for CRUD, auth, realtime or file uploads against a Cratebase instance, write a `pb_hooks/*.pb.js` lifecycle hook or `routerAdd`/`cronAdd` handler, or asks "how do I do X in Cratebase/PocketBase" — even if they never say the word "skill". Also use it to sanity-check that a proposed change matches Cratebase's actual API shape instead of guessing from PocketBase docs alone (Cratebase is compatible but has its own source of truth).
---

# Cratebase

Cratebase is a single Rust binary: dynamic collections, auth, file
storage, realtime, and filter-expression API rules behind one REST API
that's wire-compatible with PocketBase's official SDKs.

## Mental model

1. **Collection** — a schema (typed fields) plus five API rules
   (list/view/create/update/delete). Three kinds: `base` (plain data),
   `auth` (adds `email`/`password` + auth endpoints), `view` (read-only,
   backed by a `SELECT`).
2. **Record** — one row. Always has `id`, `created`, `updated`,
   `collectionId`, `collectionName`, plus the schema's own fields.
3. **API rule** — `null` = superuser only (the default, safe starting
   point), `""` = public, or a **filter expression** evaluated per
   request/row (e.g. `owner = @request.auth.id`), enforced in SQL, not
   in application code.
4. Everything else (JS hooks, cron, realtime, file storage) hangs off
   collections/records — there's no separate backend-logic layer.

If you're working inside the `cratebase` repo itself, `llms.txt` and
`openapi.yaml` at the repo root go further (full endpoint list, source
map) — check those before guessing. Outside that repo, this mental model
plus `references/` in this skill is the whole story; no network fetch
required.

## The PocketBase-compatibility trick

Cratebase ships a first-party SDK, `@cratebase/client` — use it by
default: `npm install @cratebase/client`, typed methods, built-in auth
state, realtime, plus Cratebase-only extras (vector search, LLM chat, MCP
tool schemas, presence) with no extra package. Cratebase's API is also
byte-compatible with PocketBase v0.23+, so the official `pocketbase` SDK
still works unmodified if a project is already on it — don't hand-roll
`fetch`/`axios` calls either way. The `cratebase` repo's `examples/`
directory has six real, runnable reference apps (todo list, kanban, chat,
webhooks, RAG/vector search, realtime cursors):
https://github.com/cratebasehq/cratebase/tree/main/examples

## Recipe: server → collection → rules → auth → data → realtime

1. **Bring up the server**: `cratebase serve` (SQLite + local disk, zero
   config) or `docker compose up`. First visit to `http://localhost:8090`
   shows an inline setup form for the superuser account instead of a
   login screen, or script it with `cratebase superuser create
   you@example.com yourpassword`.
2. **Create a collection**: `POST /api/collections` (superuser only).
   See `references/collections-and-sdk.md` for the field-type list and
   schema shape.
3. **Set API rules as filter expressions** — see
   `references/filter-syntax.md` for the grammar, operators, and
   `@request.*` context variables before writing anything beyond
   `owner = @request.auth.id`.
4. **Auth and CRUD from the client** with the `pocketbase` SDK — see
   `references/collections-and-sdk.md` for real call shapes.
5. **Realtime**: `collection.subscribe("*", handler)` — gated by the
   same `listRule`/`viewRule` as everything else, not a bypass.
6. **When a filter rule can't express the logic** (side effects, calling
   another API, computed fields, custom endpoints, cron), drop a
   `pb_hooks/*.pb.js` file next to the data directory — see
   `references/js-hooks.md` for every lifecycle hook, `routerAdd`/
   `cronAdd`, and the `$app`/`$http`/`$security`/etc. globals.

Filter expressions are also legal directly in `?filter=` on a list
request — prototype a rule there before pasting it into a `*Rule` field.

## Pitfalls specific to this API

- `""` on an API rule means **public**, not "no rule set" — `null` is
  the restrictive one. Easy to invert by accident.
- `@request.auth.*` is empty/falsy for unauthenticated requests, not an
  error — a rule like `author = @request.auth.id` already compiles safely
  for anonymous requests (matches nothing), no `!= ''` guard required.
  See `references/filter-syntax.md` for the compiled-SQL proof.
- Registration on an `auth` collection is just `collection.create({
  email, password, passwordConfirm })` — no separate register endpoint.
  Superuser login is `POST /collections/_superusers/auth-with-password`,
  not a dedicated admin route.
- `file`-typed fields need `multipart/form-data`, not JSON — pass a
  `File`/`FormData` and the SDK switches transport automatically.

## Reference files

- `references/filter-syntax.md` — full filter grammar, operators,
  context variables, macros, relation dot-notation, worked/compiled
  examples.
- `references/collections-and-sdk.md` — field types, the
  `POST /api/collections` shape, real SDK call patterns.
- `references/js-hooks.md` — every hook, `routerAdd`/`cronAdd`, request
  context, and the globals available inside `pb_hooks/*.pb.js`.

These reference files cite exact `crates/filter/src/...` and
`examples/...` paths from the `cratebase` repo as evidence for each
claim — if you have that repo checked out, those paths resolve directly;
if not, treat the citations as provenance, not as links.
