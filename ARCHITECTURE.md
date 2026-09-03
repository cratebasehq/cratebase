# Architecture

Cratebase is a Rust rewrite of the PocketBase idea: one process that gives a
frontend (web, mobile, or another service) a full backend — dynamic
collections, auth, file storage, and realtime — over a REST API, backed by
either SQLite or Postgres with **no code changes** between the two.

## Crate map

```
crates/
  core     — domain types only (Collection, Field, AppError). Zero I/O.
  filter   — PocketBase-style filter expression parser + SQL compiler.
  db       — the storage engine: sqlx `Any` pool, collection<->table sync,
             record CRUD, API-rule enforcement, admin accounts.
  storage  — file storage: local disk or any S3-compatible bucket
             (object_store crate — one implementation for AWS S3,
             R2, MinIO, RustFS, B2, ...).
  auth     — Argon2id password hashing, HS256 JWT session tokens.
  server   — axum HTTP API + CLI (`cratebase serve` / `superuser ...`) +
             the embedded admin dashboard.
sdk/js     — official TypeScript client ("cratebase" on npm).
web/admin  — the admin dashboard (React + Vite), embedded into the server
             binary at compile time via rust-embed.
```

Dependencies flow one way: `server` depends on `db`/`storage`/`auth`/`filter`/`core`;
`db` depends on `filter`/`core`; nothing depends on `server`. This keeps the
storage engine testable and reusable without pulling in HTTP.

## The dynamic collection engine (`crates/db`)

Every user-defined collection is:

1. A row in the system `_collections` table (JSON-encoded schema + rules).
2. A **real SQL table** `cb_<name>` with one physical column per field.

Creating/editing a collection (`collections::sync_table`) diffs the new
schema against the old one and runs `ALTER TABLE ADD/DROP COLUMN` — no
separate migration DSL, no codegen. This is deliberately the same trade-off
PocketBase makes: schema changes are live DDL, and changing a field's type
or single/multi cardinality drops and recreates that column (data loss on
that column only, not the whole table).

### Why sqlx's `Any` driver

Every query is dynamic (the schema isn't known at compile time), so
compile-time-checked queries (`sqlx::query!`) were never on the table
anyway. `sqlx::Any` lets one query string run against both SQLite and
Postgres unmodified — both drivers accept `$1, $2, ...` placeholders — which
is what actually makes "switch `DATABASE_URL`, nothing else changes" true
rather than aspirational.

### Physical column types

Only three physical shapes exist, chosen so a value's binding is always
correct on both backends:

| Field type(s) | Physical column | Backend type |
| --- | --- | --- |
| number | `Number` | `REAL` (sqlite) / `DOUBLE PRECISION` (postgres) |
| bool | `Bool`, bound as `0`/`1` | `INTEGER` on both |
| everything else, including JSON, dates (RFC3339 text), and multi-value fields (JSON-array text) | `Text` | `TEXT` on both |

Two things forced this specific shape:

- sqlx's `Any` driver **cannot decode SQLite's native `BOOLEAN` column
  type** (its bridge only understands NULL/INTEGER/REAL/TEXT/BLOB), so
  booleans are stored as `0`/`1` integers on both backends instead.
- A `NULL` bound with the wrong physical type breaks Postgres inserts
  (`column "x" is of type double precision but expression is of type text`)
  — every `ColumnValue` variant carries its own `Option<T>` so a NULL is
  always sent with the column's real type, never a generic `Option<String>`.

Both are covered by integration tests that run the exact same test suite
against SQLite and a real Postgres container (`crates/db/tests/`).

### Filter language (`crates/filter`)

A small expression language — `status = "active" && (owner = @request.auth.id || public = true)`
— that compiles to a parameterized SQL `WHERE` fragment instead of being
evaluated in-process. The same compiler backs three things:

- The `filter` query param on `GET .../records`.
- API rules (`listRule`/`viewRule`/`updateRule`/`deleteRule`), pushed into
  the `WHERE` clause of the underlying `SELECT`/`UPDATE`/`DELETE` so a
  record the rule denies is indistinguishable from one that doesn't exist.
- `createRule`, which has no row to attach a `WHERE` to yet — evaluated via
  a FROM-less `SELECT 1 WHERE <expr>` against the submitted data instead of
  a second, JSON-only evaluator.

Not supported (by design, for now): relation dot-notation (`author.name`),
and the `?=`/`?!=`/... "any of" operators PocketBase uses for array fields.

## Auth model

Two independent identity types share one JWT shape (`cratebase-auth`):

- **Superusers** (`_admins` table) — manage schema, bypass every API rule.
- **Auth collection records** — a `Collection` with `type: "auth"` gets two
  extra physical columns (`email`, `password_hash`) alongside its
  user-defined schema. Registration is just `POST .../records` on an auth
  collection with `password`/`passwordConfirm`; there's no separate
  "register" endpoint.

Tokens are stateless HS256 JWTs with no server-side revocation list —
rotating `AUTH_SECRET` invalidates every outstanding token at once. This is
a deliberate simplicity trade-off, not an oversight.

## Realtime

`crates/server/src/realtime.rs` is an in-process pub/sub hub: SSE clients
connect to `GET /api/realtime`, get a `clientId`, then `POST /api/realtime`
to declare which collections/records they want (`"posts"` or
`"posts/<id>"`). Record mutations publish to matching subscribers. This is
single-node by design (no external broker) — the same trade-off PocketBase
makes. Horizontal scale-out needs a shared broker (Postgres
`LISTEN/NOTIFY`, or a queue) and is tracked in [ROADMAP.md](./ROADMAP.md).

## File storage

`crates/storage` wraps the `object_store` crate: `LocalFileSystem` for the
zero-config default, `AmazonS3Builder` (path-style, custom endpoint) for
every S3-compatible backend — AWS S3, Cloudflare R2, Backblaze B2, and
self-hosted RustFS/MinIO all go through the same code path, verified
against a real RustFS container. Downloads stream from the backend
(`Storage::get_stream`) instead of buffering the whole file in memory.

## The admin dashboard is not a separate deployment

`web/admin` is a normal Vite/React app during development (`bun run dev`,
proxying `/api` to a local `cratebase serve`). For production, its build
output (`web/admin/dist`) is embedded into the `cratebase` binary at
**compile time** via `rust-embed` (`crates/server/src/dashboard.rs`) — the
same trick PocketBase uses for its Svelte admin UI in a single Go binary.
`cargo build --release` produces one executable that serves the API and the
dashboard; no Node runtime in production.
