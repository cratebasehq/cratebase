# Roadmap

Cratebase's v1 scope: dynamic collections, auth, files, realtime, on
SQLite or Postgres, with an admin dashboard, one binary. What's below is
what's deliberately **not** in v1, roughly in the order it's likely to
land.

## Next up

- **Plugin system.** Shipped as of `crates/server/src/plugin.rs`: a
  `Plugin` trait with two extension points — `setup()` for one-time
  synchronous startup work (bind hooks, spawn a background task) and
  `routes()` to mount extra HTTP endpoints under `/api/plugins/<name>`.
  This is a compile-time Rust trait, not a dynamically loaded/scripted
  plugin format: "installing a plugin" means implementing `Plugin` and
  registering it in a `PluginRegistry`, then shipping your own binary —
  which doesn't have to be this repo's own binary either, since
  `cratebase-server` is a normal library crate a downstream project can
  depend on (see `crate::plugin`'s module doc for the pattern). An
  earlier revision of this file described a third extension point,
  `scheduled_tasks()`, and a `plugins/example.rs` reference file;
  neither exists in the tree — that was aspirational text outrunning the
  code, corrected here. `setup()` receiving a plain `&App` (not `async`)
  is enough for periodic background work: a plugin that needs a fixed
  interval spawns its own `tokio::spawn` loop from inside `setup()` —
  see `crates/server/src/queue.rs` below for the pattern.
  **Shipped on this foundation:**
  - **Custom SQL cron jobs** (`crates/server/src/cron_jobs.rs`, not a
    plugin) — a `_cron_jobs` system collection: name, a 5-field cron
    expression, and a raw SQL statement to run on it, editable from the
    dashboard's Cron jobs screen or the generic Records API. No match-arm
    registry, no rebuild for a new job type — the SQL itself is the job
    body. Superuser-only end to end (no rule enforcement bypasses it —
    same trust tier as a collection schema edit), validated as a real
    cron expression before the row is even written, and reactive: a
    create/update/delete takes effect on the live scheduler immediately,
    no restart. `lastRunAt`/`lastStatus`/`lastMessage` are written back
    after every run, including the real driver error on failure.
  - **Durable job queue** (`crates/server/src/queue.rs`,
    `QueuePlugin`) — a real pg_boss-style worker, toggle-gated off by
    default behind `settings.queue.enabled` (see "Toggle-gated built-in
    modules" below). A `_queue_jobs` system collection
    (`queue`/`payload`/`status`/`attempts`/`maxAttempts`/`runAfter`/
    `startedAt`/`lastError`), `POST /api/plugins/queue/enqueue`
    (superuser-only, goes through `cratebase_db::records::create` for id
    generation and validation), and a `setup()`-spawned tick loop:
    `reclaim_stale` first puts any `in_progress` job stuck past a
    timeout back to `pending` (a crashed worker's job is never orphaned
    forever — pg_boss's own core guarantee), then `claim_next` atomically
    claims the earliest due `pending` row inside one transaction
    (`SELECT ... FOR UPDATE SKIP LOCKED` on Postgres so concurrent
    workers never race for the same row; SQLite's single writer lock
    already serializes it) and an `UPDATE ... WHERE status = 'pending'`
    that re-checks status inside the same transaction rather than
    trusting the row the `SELECT` just read. A failed job's `runAfter` is
    pushed out with exponential backoff (`min(2^attempts * base, max)`)
    until `attempts` reaches `maxAttempts`, at which point it becomes
    `failed` for good. Job bodies are a small in-process handler registry
    (`QueuePlugin::handle().register_handler(name, ...)`), not a dynamic
    execution plane — that is the separate WASM plugin system below.
  **Not built on this foundation:**
  - A feature-flags plugin was also listed as "shipped" in an earlier
    revision of this file; `plugins/feature_flags.rs` never existed in
    the tree either. Not currently planned — it would need the same
    "named user asked for this" bar as anything else in this file before
    landing.
  - Record lifecycle hooks (`on_create`/`on_update`/`on_delete`) are not
    on the trait yet — add them when the first plugin actually needs one,
    rather than speculatively.

## Toggle-gated built-in modules

Teams, the LLM chat gateway, and the queue plugin above are all built
in — compiled into the `cratebase-server` binary either way — but off by
default and hidden from the dashboard until an operator opts in, rather
than always-on background cost or a permanently deleted feature:

- **Teams** (`crates/server/src/teams.rs`). `settings.teams.enabled`
  (default `false`). `App::bootstrap` only calls `teams::bind_hooks` when
  set — the reactive bootstrap-owner hook is never bound otherwise, zero
  background cost. The `_teams`/`_team_members` system collections
  always exist regardless (cheap, and avoids a migration-reversibility
  story), but stay out of the dashboard sidebar's System group until
  enabled.
- **LLM chat gateway** (`crates/server/src/routes/llm.rs`). Reuses the
  existing `settings.llm.enabled` flag rather than adding a redundant
  second one: `routes::api_router` only merges `llm::router()` when it's
  `true`, so `POST /api/llm/chat` 404s outright (not a 403) while
  disabled. Confirmed independent of vector search's auto-embedding
  (`crates/server/src/embeddings.rs` reads its own
  `EMBEDDINGS_BASE_URL`/`EMBEDDINGS_API_KEY` env vars) — see
  `crates/server/tests/vector_fields.rs`'s
  `vector_auto_embedding_is_unaffected_by_a_disabled_llm_gateway` for the
  live proof.
- **Queue plugin** (`crates/server/src/queue.rs`). `settings.queue.enabled`
  (default `false`). `App::bootstrap` only provisions `_queue_jobs` and
  registers `QueuePlugin` when set, so an idle install never spawns the
  worker tick.

Both flags' route/hook wiring is decided once, at boot (`App::serve`
assembles the router exactly once from `App::bootstrap`'s already-loaded
settings) — flipping a toggle via `PATCH /api/settings` takes effect on
the next restart, not live. That is the deliberate trade-off for "zero
background cost while disabled": there is nothing to tear down or
re-wire at runtime because nothing was ever wired up.

## Shipped

- **Team management (superuser roles).** `_superusers` gained a required
  `owner`/`admin` role field (`crates/core/src/field.rs`'s `role_field`),
  enforced by `RequireOwner` (`crates/server/src/extract.rs`) alongside
  inline guards in `routes/records.rs`: creating, deleting, or changing
  another superuser's role needs `owner`; `admin` keeps identical access
  to everything else, including self-service on their own row. The sole
  remaining `owner` cannot be demoted or deleted (migration
  `8_add_superuser_role.rs` backfills every pre-existing row to `owner`,
  so no installation loses admin access on upgrade). Built directly on
  the core auth crate rather than the `Plugin` trait, since record
  lifecycle hooks (below) still don't exist on it.

- **Streaming backup upload.** `write_backup` in `routes/backups.rs`
  builds the `VACUUM INTO` snapshot into a temp-file ZIP via
  `spawn_blocking`, then opens that file and feeds it through
  `tokio_util::io::ReaderStream` into `Storage::put_stream` — never a
  `Vec<u8>`/`tokio::fs::read` of the whole archive. `put_stream` buffers
  only up to one 5 MiB multipart chunk before switching to the driver's
  multipart upload (S3 `CreateMultipartUpload`, or a renamed temp file
  for the local driver) and bounds in-flight parts with
  `wait_for_capacity(4)`, so memory use is capped regardless of archive
  size — the same primitive `download` already used via
  `Storage::get_stream`. Verified live: a 1.6 GB SQLite database backed
  up through the real `/api/backups` endpoint peaked at ~330 MB RSS
  (vs. a 1.6+ GB spike the old whole-file-buffering implementation would
  have hit), produced an 871.8 MB compressed archive, and the downloaded
  copy passed `sqlite3 <file> .tables` with every row intact.

- **Write throughput under contention and wide pages.** The single-writer
  pool + native-driver storage engine (`crates/db`) resolved the
  contention this item used to track. Measured on an idle host
  (`benchmarks/run.sh --skip-build`, 0 errors both sides): Cratebase beats
  PocketBase on 22 of 24 cells, several by an order of magnitude
  (`search` at concurrency 50: 49986 vs 4774 req/s, 10.47x; `search-auth`
  at concurrency 100: 38768 vs 3989 req/s, 9.72x). The two remaining
  cells, `delete` at concurrency 1 and 20, are within noise of parity
  (0.95x and 0.97x) — not a regression to chase, just not yet a win.
  Full table: `benchmarks/README.md`; raw numbers:
  `benchmarks/results/{cratebase,pocketbase}.json`.
- **Auth rate limiting.** `/collections/{c}/auth-with-password`, and the
  `request-*`/`confirm-*` email and OTP flows, are rate-limited per client
  IP (`AUTH_RATE_LIMIT_ENABLED`, on by default). `auth-refresh` is
  deliberately excluded — see `routes/auth.rs`'s doc comment for why.
- **OAuth2 (Google, GitHub, custom).** `crates/auth/src/oauth2.rs` (token
  exchange body, Google/GitHub userinfo parsing, generic-provider
  fallback) plus `routes/auth.rs`'s `auth-with-oauth2` and `auth-methods`
  providers list. Authorization-code + PKCE, matching the SDK's
  `authWithOAuth2Code`: `auth-methods` hands back a per-provider
  `authURL`/`state`/`codeVerifier`, and `auth-with-oauth2` exchanges the
  resulting `code` server-side, fetches userinfo, and either signs in
  the `_externalAuths`-linked record, links onto a same-email match, or
  creates a new one (`resolve_oauth2_record`). "google"/"github" only
  need a client id/secret configured on the collection; any other
  `name` is a hand-configured provider using its own auth/token/userinfo
  URLs.
- **Mailer.** `crates/mailer`: Resend HTTP API, plain SMTP, or a `Log`
  fallback that writes the email to `tracing` instead of delivering it —
  every email-dependent flow below is exercisable with zero external
  setup. Templates live in `web/email` (react-email, built once via
  `bun run email:build` and embedded into the binary the same way the
  admin dashboard is).
- **Email verification.** Auth collections get a `verified` column
  (starts `false`, never client-settable) and
  `authOptions.requireEmailVerification` now actually gates
  `auth-with-password` when set. `request-verification` /
  `confirm-verification` endpoints, one-time `VerifyEmail` action tokens
  (stateless JWTs, same trade-off as session tokens — see
  `crates/auth/src/token.rs`).
- **Password reset.** `request-password-reset` / `confirm-password-reset`,
  `ResetPassword` action tokens.
- **Email change confirmation.** `request-email-change` (authenticated) /
  `confirm-email-change`, `ChangeEmail` action tokens carrying the pending
  new address — the identity only changes once the *new* address confirms
  ownership.

Password reset and email verification/change are email-identity-only for
now (`authOptions.identityField == "email"`); a username-identity auth
collection has no address to send them to.

- **OTP (passwordless) login and MFA.** `POST
  /collections/{c}/request-otp` / `auth-with-otp` for a code-only login,
  backed by a new `_otps` collection (one-time codes, SHA-256 hashed —
  not Argon2id, since an OTP is single-use and discarded within minutes).
  `authOptions.mfa.enabled` (with `authOptions.mfa.rule` selecting which
  records need it) gates a successful first-factor login behind a second
  one: the first successful credential check opens a pending `_mfas`
  session and answers `401 {"mfaId": "..."}`; completing it is a second
  call to `auth-with-password` or `auth-with-otp` with that `mfaId`,
  using a *different* method than the one that already succeeded — there
  is no separate `mfa/confirm` endpoint.
- **Superuser impersonation.** `POST /collections/{c}/impersonate/{id}`
  (superuser only) mints a non-refreshable session token for another
  record — a one-shot loan, not a credential the impersonated record can
  extend itself.
- **New-location login alerts.** Every successful login records a
  fingerprint in the `_authOrigins` collection; a genuinely new device for
  a record that already had a prior origin on file fires an emailed
  alert (best-effort — never fails the login it rides along with). A
  record's very first login ever is never alerted.
- **First-run setup.** `GET /api/setup/status` / `POST /api/setup`
  (`crates/server/src/routes/setup.rs`) create the first `_superusers`
  record without a CLI: the dashboard renders an inline setup form
  instead of a bare login screen until one exists, then logs in through
  the ordinary `auth-with-password` flow. `cratebase superuser create`
  still works for scripted/headless setup; the endpoint re-checks "does a
  superuser exist" at write time and is permanently closed once one does.
- **JS hooks (PocketBase parity).** A `pb_hooks/*.pb.js` file next to the
  data directory is evaluated at startup by an embedded QuickJS runtime
  (`crates/jsvm`): PocketBase-style `onRecordCreate`/`onRecordUpdate`-style
  lifecycle hooks bound through native hook registration, and `routerAdd`
  for mounting custom root-level HTTP routes (`cronAdd` for JS-defined
  cron jobs too). The glue lives in `crates/server/src/jsvm_host.rs`,
  which implements `cratebase_jsvm::HostApi` once over the plain `App`
  and once over an open `TxApp` so a hook's own `$app.save`/`$app.delete`
  inside a record write-path hook joins that write's own transaction. No
  `pb_hooks/` directory (or an empty one) is a complete no-op.
- **Batch API.** `POST /api/batch` — transactional multi-record
  create/update/delete in one request, sharing one SQL transaction with
  the same rule/validation logic the individual record routes use.
- **View collections.** `Collection::View`'s `view_query` now backs a
  real SQL `VIEW` (recreated on update); writes are rejected with 400.
- **Relation dot-notation** (`author.name = "..."`) and **"any of"
  filter operators** (`?=`, `?!=`, ...) for multi-value fields — both in
  `crates/filter` + `crates/db/resolver.rs`.
- **File thumbnails** (`?thumb=WxH`/`WxHf`/`WxHt`/`WxHb`, generated on
  first request and cached to the storage backend) and **protected-file
  access tokens** (`POST /api/files/token`, `?token=`) for embedding a
  gated file where an `Authorization` header can't be sent.
- **Cross-node realtime (Postgres only).** `GET /api/realtime` SSE
  subscriptions used to be strictly single-process: `RealtimeService`'s
  client registry and fan-out (`crates/server/src/realtime.rs`) only
  ever knew about writes handled by the same process, so a deployment
  behind a load balancer with more than one app instance would silently
  drop realtime events for a client parked on a different instance than
  the one that handled the write. Fixed for Postgres deployments via
  `pg_notify`/`LISTEN`: every write still does its normal local
  in-process fan-out, and now also calls the new
  `Engine::notify_realtime` (`crates/db/src/postgres.rs`) with a small
  JSON payload — collection id, action, record id, and (only for a
  delete, where the row won't exist for another process to re-fetch) a
  snapshot of the record — bounded well under Postgres's 8000-byte
  `NOTIFY` payload limit and rejected outright rather than silently
  truncated if it isn't. Every app process sharing that database starts
  one `Engine::subscribe_realtime` listener at boot
  (`App::bootstrap` → `realtime::start_cross_node_listener`), on a
  dedicated (non-pooled) connection that reconnects with backoff on
  connection loss so one dropped connection can't permanently kill
  cross-node realtime. A receiving process re-fetches the record and
  re-evaluates `listRule`/the topic's own filter itself, against its
  *own* current settings and rule text — never against anything the
  writer serialized — the same access-decision path a local write
  already used. SQLite deployments are unaffected: `notify_realtime`/
  `subscribe_realtime` default to no-ops on any `Engine` that doesn't
  override them, which is exactly right for a backend that is
  single-node by construction (one file, one process).
  See `crates/db/tests/postgres.rs` for a two-connection LISTEN/NOTIFY
  test proving the plumbing, and `crates/server/tests/postgres_multi_node.rs`
  for the genuine end-to-end proof: two full `App`s, each with its own
  real `axum::serve` listener on its own port, both against one
  Postgres database. A real `reqwest` client opens `GET /api/realtime`
  against instance A and subscribes to a collection; a second real
  client POSTs a record create to instance B's REST API for that same
  collection; the test asserts the create event actually arrives on
  A's SSE stream — a connection B never touched — within a timeout,
  observing a real round trip through two independent Postgres pools
  and a dedicated `LISTEN` connection on each side, not an in-process
  function call. That test caught a real bug in the process:
  `origin_id()` (the id `notify_cross_node` stamps on every payload so
  a listener can recognize and skip its own writes) was a
  process-wide `LazyLock` static rather than per-`App`-instance, so
  two `App`s sharing one OS process — exactly what an in-process
  multi-node test needs, and not something the design ruled out for
  an embedder either — shared one origin and each mistook the other's
  writes for its own echo, silently dropping every cross-node event.
  Fixed by moving the origin onto `RealtimeService` itself, generated
  once per instance instead of once per process. With that fix the
  observed cross-node latency (HTTP POST on B to SSE frame on A,
  through commit, `pg_notify`, the dedicated `LISTEN` connection, a
  fresh `SELECT` on A, and rule re-evaluation) is consistently
  ~95-100ms against a local Postgres 16 container.
- **Admin dashboard: Settings area** (request logs, backups — including
  upload/restore, cron jobs, and a Network page for rate limits/trusted
  proxy/superuser IPs), **collection export/import**, a **geoPoint field
  editor**, **per-collection auth-provider UI**, **design system pass**
  (light/dark contrast), **sidebar system-collection grouping**,
  **type-aware records table**, and **field-editor UX polish** (per-type
  option panels, inline validation, a syntax-help popover on every rule
  input).
- **Multi-file append/remove semantics.** A multipart update field named
  `field+` appends newly uploaded files to an existing multi-file field
  without disturbing the rest; `field-` removes named files (deleting
  their storage blobs, not just the JSON list entry) — verified against
  real PocketBase v0.40.2 to confirm removing a name that isn't present
  is a silent no-op, not an error. An unsuffixed field name still fully
  replaces, unchanged (`crates/server/src/routes/records.rs`).
- **Admin audit log.** A new append-only `_audit_log` system collection
  (`crates/server/src/audit.rs`) records collection schema changes,
  settings updates, and superuser account changes, plus any record
  delete where a superuser bypassed the collection's own `deleteRule` —
  not ordinary rule-permitted deletes, to avoid drowning the log in
  noise. No rule string can express "nobody, ever," so update/delete on
  `_audit_log` itself is rejected at the hook level even for a
  superuser. Dashboard page at Settings → Audit log.
- **Per-collection rate limiting.** Already fell out of the existing
  `RateLimitRule` tag-matching (`crates/server/src/middleware/rate_limit.rs`'s
  `tags_for()` derives a `{collection}:{action}` tag per request purely
  from URL shape); the real gap was dashboard discoverability, closed
  with a collection-name autocomplete on the rule editor
  (`settings/network-page.tsx`).

## Later

(nothing currently tracked here — everything previously listed shipped
this session; see Shipped below.)

## Explicitly out of scope for now

- A visual query builder for filters (the filter language is meant to be
  hand-written; a builder is a dashboard feature, not a core one).
- Multi-tenant / workspace-scoped superusers (one superuser table, full
  access to everything).
