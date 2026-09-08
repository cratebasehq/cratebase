# Architecture

Cratebase is a single Rust process that gives a frontend (web, mobile, or
another service) a full backend — dynamic collections, auth, file
storage, and realtime — over a REST API, backed by either SQLite or
Postgres with **no code changes** between the two.

## Crate map

```
crates/
  core     — domain types only (Collection, Field, AppError). Zero I/O.
  filter   — filter expression parser + SQL compiler.
  db       — the storage engine: sqlx `Any` pool, collection<->table sync,
             record CRUD, API-rule enforcement.
  storage  — file storage: local disk or any S3-compatible bucket
             (object_store crate — one implementation for AWS S3,
             R2, MinIO, RustFS, B2, ...).
  auth     — Argon2id password hashing, HS256 JWT session/action tokens,
             OTP hashing.
  jsvm     — an embedded QuickJS runtime (PocketBase-compatible `pb_hooks/`
             JS hooks and `routerAdd`/`cronAdd`), decoupled from `server`'s
             HTTP/DB types behind a `HostApi` trait it calls back through.
  mailer   — pluggable mail backend: Resend HTTP API, plain SMTP, or a
             `Log` fallback that writes to `tracing` instead of delivering,
             so every email-dependent flow works with zero external setup.
  server   — axum HTTP API + CLI (`cratebase serve` / `superuser ...`) +
             the embedded admin dashboard + the `jsvm`/`mailer` wiring
             (`jsvm_host.rs`).
web/admin  — the admin dashboard (React + Vite), embedded into the server
             binary at compile time via rust-embed.
```

Dependencies flow one way: `server` depends on every other crate;
`db` depends on `filter`/`core`; `jsvm` and `mailer` depend on neither
`db` nor `server` (they talk back to `server` through `HostApi` and a
plain `Message` type, respectively). This keeps the storage engine, JS
runtime, and mailer each testable and reusable without pulling in HTTP.

## The dynamic collection engine (`crates/db`)

Every user-defined collection is:

1. A row in the system `_collections` table (JSON-encoded schema + rules).
2. A **real SQL table** `cb_<name>` with one physical column per field.

Creating/editing a collection (`collections::sync_table`) diffs the new
schema against the old one and runs `ALTER TABLE ADD/DROP COLUMN` — no
separate migration DSL, no codegen. This is a deliberate trade-off: schema
changes are live DDL, and changing a field's type or single/multi
cardinality drops and recreates that column (data loss on that column
only, not the whole table).

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
and `?=`/`?!=`/... "any of" operators for array fields.

## Auth model

Superusers are not a separate concept: `_superusers` is an ordinary
built-in `type: "auth"` collection, exactly like a user-defined one, and
goes through the same code path (PocketBase dropped its dedicated admin
table the same way as of v0.23). A `Collection` with `type: "auth"` gets
five extra physical columns alongside its user-defined schema — `password`
(the Argon2id hash), `tokenKey`, `email`, `emailVisibility`, `verified` —
and registration is just `POST .../records` with `password`/
`passwordConfirm`; there's no separate "register" endpoint.

Session tokens are stateless HS256 JWTs (`{collectionId, exp, id,
refreshable, type}`) signed with `app secret + record.tokenKey +
authToken.secret`. Rotating a record's `tokenKey` (which every password
or email change does) invalidates every outstanding session for that
record without a lookup table; rotating the app-wide `AUTH_SECRET`
invalidates everything at once. On top of that, targeted single-session
revocation is layered in via the `_sessions` ledger described below.
Verification, password-reset, and email-change tokens reuse the same
signed-JWT machinery under a different `type` and their own
`TokenConfig` secret, which is what makes them single-use for free: the
same `tokenKey` rotation that ends a session also burns any outstanding
one-shot token.

Beyond password login, `crates/server/src/routes/auth.rs` implements the
rest of PocketBase's auth surface on auth collections: email verification
and password reset (`request-`/`confirm-verification`,
`request-`/`confirm-password-reset`), email-change confirmation
(`request-`/`confirm-email-change`), OTP passwordless login (`request-otp`,
`auth-with-otp`), MFA gating a second factor behind a pending `_mfas`
session, OAuth2 (Google/GitHub, `oauth2.rs`), superuser impersonation
(`POST .../impersonate/{id}`), and best-effort new-location login alerts
tracked in the `_authOrigins` collection.

Sessions travel as either a bearer `Authorization` header or, when
`SESSION_COOKIE=true`, an `HttpOnly` cookie set by the server on
`auth-with-password`/`auth-with-otp`/`auth-with-oauth2`/`auth-refresh`;
both transports are accepted on every authenticated request regardless of
which one issued the session. Cookie mode also turns on a same-origin
CSRF check on unsafe methods (`Origin` header must match) and enables
CORS `allow_credentials`, so `CORS_ALLOW_ORIGINS` must be an explicit
origin list rather than the default `*`.

Because sessions are stateless JWTs, revoking one before its `exp` still
needs somewhere to record that fact: `_sessions` is a system collection
acting as a revocation ledger (one row per session, keyed by a digest of
its token, tagged with `kind` — `password`/`otp`/`oauth2`/`impersonation`/
`refresh`) that the token-verification hot path checks against an
in-memory digest set kept in sync with the table, so a normal request
never costs a DB round trip to confirm a session is still live — only
`auth-signout`, `sessions.revoke*`, and banning (which also rotates the
banned record's signing key, invalidating every session at once) touch
the table and the in-memory set together.

## Realtime

`crates/server/src/realtime.rs` is an in-process pub/sub hub: SSE clients
connect to `GET /api/realtime`, get a `clientId`, then `POST /api/realtime`
to declare which collections/records they want (`"posts"` or
`"posts/<id>"`). Record mutations publish to matching subscribers. This is
single-node by design (no external broker) — a deliberate scope decision.
Horizontal scale-out needs a shared broker (Postgres `LISTEN/NOTIFY`, or a
queue) and is tracked in [ROADMAP.md](./ROADMAP.md).

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
**compile time** via `rust-embed` (`crates/server/src/dashboard.rs`).
`cargo build --release` produces one executable that serves the API and the
dashboard; no Node runtime, and no separate frontend to host, in
production.

A fresh instance with no superuser yet needs no CLI to get started: `GET
/api/setup/status` reports `{needsSetup: true}` until the first superuser
exists, and the dashboard renders an inline setup form against `POST
/api/setup` instead of a bare login screen (`crates/server/src/routes/setup.rs`).
`cratebase superuser create` still works for scripted/headless setup —
`setup.rs` re-checks "does a superuser exist" at write time, so whichever
path wins the race closes the other one out.

## Extending with JavaScript (`crates/jsvm`)

A `pb_hooks/*.pb.js` file next to the data directory is evaluated at
startup by an embedded QuickJS runtime (`crates/jsvm`), giving
PocketBase's own extension surface: `onRecordCreate`/`onRecordUpdate`-style
lifecycle hooks bound through native `Hook<E>` registration
(`crates/server/src/hooks.rs::bind_js_hook`), and `routerAdd` for mounting
custom root-level HTTP routes, turned into real axum routes by
`jsvm_host::js_router` once every hook file has run. `crates/jsvm` itself
has no dependency on `db`/`server`; the glue lives in
`crates/server/src/jsvm_host.rs`, which implements `cratebase_jsvm::HostApi`
twice — once over the plain `App` for routes/crons/most hooks, and once
over an open `TxApp` for hooks that fire inside a record write's own
transaction, so a hook's own `$app.save`/`$app.delete` commits or rolls
back with the write that triggered it. No `pb_hooks/` directory, or an
empty one, is a complete no-op: nothing is spawned, matching PocketBase's
own behavior.
