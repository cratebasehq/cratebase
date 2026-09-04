# Roadmap

Cratebase's v1 scope: dynamic collections, auth, files, realtime, on
SQLite or Postgres, with an admin dashboard, one binary. What's below is
what's deliberately **not** in v1, roughly in the order it's likely to
land.

## Next up

- **Plugin system.** Shipped as of `crates/server/src/plugin.rs`: a
  `Plugin` trait with three extension points — `setup()` for one-time
  async startup work (e.g. provisioning a collection a plugin depends
  on), `routes()` to mount extra HTTP endpoints under
  `/api/plugins/<name>`, and `scheduled_tasks()` for fixed-interval
  background jobs. This is a compile-time Rust trait, not a dynamically
  loaded/scripted plugin format: "installing a plugin" means implementing
  `Plugin` and registering it in a `PluginRegistry`, then shipping your
  own binary — which doesn't have to be this repo's own binary either,
  since `cratebase-server` is a normal library crate a downstream project
  can depend on (see `crate::plugin`'s module doc for the pattern).
  `plugins/example.rs` is a working reference (a `/stats` route + a
  5-minute logging job) to copy from.
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
  - **Feature flags as a plugin** (`plugins/feature_flags.rs`) — a
    self-contained `_feature_flags` collection + an `evaluation rule`
    (reuses the existing filter/rule engine, evaluated against
    `@request.auth.*`) + `cb.featureFlags.isEnabled(key)` in the SDK.
  - **Durable job queue as a plugin** (`plugins/queue.rs`) — a
    `_queue_jobs` collection, `POST /api/plugins/queue/enqueue`, and a
    worker tick with exponential-backoff retry and stale-job (crashed
    mid-run) reclaim. Same "small built-in job registry" trade-off as
    cron jobs. `cb.queue.enqueue(queue, payload)` in the SDK.
  **Still not built on this foundation:**
  - **Team management as a plugin** — multiple superusers with roles is a
    real data-model change (today `_superusers` is one flat auth
    collection, no roles); once record-lifecycle hooks exist on `Plugin`
    this can enforce role checks without touching the core auth crate.
  - Record lifecycle hooks (`on_create`/`on_update`/`on_delete`) are not
    on the trait yet — add them when the first plugin actually needs one,
    rather than speculatively.

## Shipped

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

## Later

- Multi-file append/remove semantics on update (today, uploading new files
  for a field replaces the whole value; `field+`/`field-` suffix syntax
  for appending/removing individual files is not implemented).
- Admin audit log / activity feed.
- Per-collection rate limiting (today's rate limiting is IP-based on the
  auth/email endpoints only, not a general per-collection mechanism).

## Explicitly out of scope for now

- A visual query builder for filters (the filter language is meant to be
  hand-written; a builder is a dashboard feature, not a core one).
- Multi-tenant / workspace-scoped superusers (one superuser table, full
  access to everything).
