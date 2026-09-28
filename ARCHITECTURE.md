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
             OTP/TOTP hashing, 12 built-in OAuth2 presets + generic OIDC
             (JWKS verification, issuer discovery).
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
`auth-with-otp`), magic-link login (`request-magic-link`,
`auth-with-magic-link`, gated by `authOptions.magicLink.enabled`), MFA
gating a second factor behind a pending `_mfas` session (a record with
confirmed TOTP requires one independently of `authOptions.mfa`), TOTP 2FA
with backup codes (`totp.rs` — RFC 6238, replay-protected, `_totps`),
OAuth2 — 12 built-in presets (Google, GitHub, Apple, Microsoft, Discord,
GitLab, Facebook, X/Twitter, LinkedIn, Slack, Twitch, Spotify,
`oauth2.rs`) plus a generic OIDC provider discovered from
`/.well-known/openid-configuration` and verified against its own JWKS
(`oidc.rs`) — linked-account management
(`GET`/`DELETE .../external-auths[/{provider}]`), superuser impersonation
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
`"posts/<id>"`). Record mutations publish to matching subscribers directly
within the process that handled the write. On SQLite this is single-node
by construction (one file, one process) — nothing more is needed. On
Postgres, every write also calls `Engine::notify_realtime`
(`pg_notify`/`LISTEN`, `crates/db/src/postgres.rs`) with a small JSON
payload (collection id, action, record id, and — only for a delete — a
record snapshot), and every app process sharing that database runs a
`subscribe_realtime` listener started at boot, so a client connected to
one instance behind a load balancer still gets events from a write
handled by a different instance. A receiving process re-fetches the
record and re-evaluates the subscriber's own `listRule`/topic filter
itself, never trusting anything the writer serialized. See
[ROADMAP.md](./ROADMAP.md)'s "Cross-node realtime (Postgres only)" entry
for the full design and the two-process integration test that proves it.

## Email platform

An editable `_emailTemplates` system collection renders with `{{var}}`
mustache-style substitution (`crates/mailer/src/mustache.rs`, dotted
paths, HTML-escaped by default, `{{{raw}}}` for unescaped), wrapped in a
shared branded base layout unless `layout: false`. `crates/server/src/mail_templates.rs`
resolves each of the built-in auth-flow emails (verification, password
reset, email change, OTP, login alert, magic link) through a three-step
priority chain — a customized `authOptions.*Template` field, else the
matching `_emailTemplates` row (locale-aware, falling back to `""`), else
the same built-in default — so an unmodified install's mail is
byte-for-byte unchanged. `crates/server/src/mails.rs` is the actual send
pipeline (validate → render → log to `_mailLog` → deliver inline or via
the durable queue), shared verbatim between `POST /api/mails/send`
(`crates/server/src/routes/mails.rs`) and the JS hook binding `$mails.send`.
`_emailTemplates.sendRule` (a filter-rule expression evaluated by
`crate::mail_templates::eval_send_rule` against `@request.auth`/
`@request.body.{to,data,locale}`) is what lets a non-superuser HTTP caller
reach `POST /api/mails/send` at all — restricted to `template`/`to`/`data`,
a handful of recipients, and its own rate-limit tag. `_emailTriggers`
(`crates/server/src/email_triggers.rs`) fires a template automatically
from an after-success record hook on create/update/delete, with an
optional `condition` filter and `dataMap`; it can never fail the
triggering request.

## Database extensibility

Three independent pieces sit on top of the same `crates/db` engine
described above. **Postgres extension management**
(`GET/POST/DELETE /api/db/extensions[/{name}]`, superuser only, 404 on
SQLite) runs `CREATE EXTENSION IF NOT EXISTS`/`DROP EXTENSION` and audits
every install/drop; `$app.db().exec(sql, params?)` is a write-capable
escape hatch alongside the existing read-only `$app.rawQuery`, letting a
`pb_migrations/*.js` file version something like `CREATE EXTENSION postgis`
the same way it versions schema changes. **Custom SQL RPC**
(`crates/server/src/rpc.rs`) is the `_rpc` system collection — `sql` with
`:name`-style named placeholders bound as real driver parameters (never
string-interpolated), a `rule` evaluated the same way a collection's own
rules are, and `readOnly` (default `true`) enforced by reusing the SQL
console's `BEGIN READ ONLY`/`PRAGMA query_only` machinery — plus
`POST /api/rpc/{name}`. **PostGIS-accelerated geo queries**: the existing
`geoDistance(...)` filter/sort function (`crates/filter`) compiles against
a GiST-indexed `geography` expression (`ST_DWithin`/KNN `<->`) instead of
the portable haversine calculation once `postgis` is installed on a
Postgres database, with the index created idempotently per `geoPoint`
field as part of ordinary collection schema sync
(`crates/server/src/geo.rs`) — identical filter/sort syntax either way.

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

Settings navigation is 7 tabbed groups (Application, Email, Auth &
security, Database, Automation, Integrations, Logs — `web/admin/src/lib/settings-nav.ts`),
each one page with tabs addressed by a `?tab=` URL search param rather
than a flat ~24-item sidebar; every server setting is reachable from it.
A dismissible onboarding checklist on the dashboard home
(`components/dashboard/onboarding-checklist.tsx`) is computed live from
the same settings/collections APIs the rest of the dashboard uses, not a
separate wizard with its own state.

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
