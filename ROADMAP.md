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
- **OAuth2 (Google, GitHub).** Not wired up despite appearances:
  `.env.example` documents `OAUTH_GOOGLE_CLIENT_ID`/`_SECRET` and
  `OAUTH_GITHUB_CLIENT_ID`/`_SECRET` as placeholders, `auth-methods`'
  response already has an `oauth2: {enabled, providers}` shape, and
  `AuthOptions.oauth2` exists on every auth collection — but there is no
  `crates/server/src/oauth2.rs`, no authorization-code exchange, and no
  `auth-with-oauth2` route; `routes/auth.rs`'s router explicitly comments
  `// auth-with-oauth2 (no provider wiring yet)`, and `auth-methods`
  always reports `providers: []`. External auths (`_externalAuths`
  collection CRUD, `listExternalAuths`/`unlinkExternalAuth` in the SDK)
  work today for a provider linked by some other means, but nothing in
  this codebase can create that link yet.
- File field constraints in the dashboard UI (`mimeTypes`, `maxSize` are
  already schema fields but have no editor).
- **Streaming backup upload.** `routes/backups.rs`'s `create` reads the
  entire `VACUUM INTO` snapshot into a `Vec<u8>` (`tokio::fs::read`)
  before a single `Storage::put`, unlike `download`, which streams
  (`Storage::get_stream`). Fine for a small/medium SQLite file; holds
  the whole database in memory for a multi-GB one, risking an OOM on a
  self-hosted box with limited RAM. Needs a streaming `put` on the
  `Storage` trait (both the local-disk and S3 multipart-upload impls
  support it) before this is safe at scale — file uploads have the same
  shape today (bounded by upload size limits) but a backup has no such
  cap.
- **Write throughput under contention and wide pages.** After the Phase 1
  performance pass (`benchmarks/README.md`) Cratebase is faster than
  PocketBase on 19 of 24 measured cells; the two it still loses, `create`
  at concurrency 20+ and `perPage=200` reads at concurrency 20+, share a
  cause: every pooled SQLite connection contends for the single writer
  lock via `busy_timeout`, and every row is decoded twice through
  `sqlx::Any`. Fixed by the Phase 2 storage engine (single-writer pool +
  native drivers), not by tuning. The `SELECT COUNT(*)` this entry used to
  blame was measured at well under 5% of the request.

## Shipped

- **Auth rate limiting.** `/collections/{c}/auth-with-password`, and the
  `request-*`/`confirm-*` email and OTP flows, are rate-limited per client
  IP (`AUTH_RATE_LIMIT_ENABLED`, on by default). `auth-refresh` is
  deliberately excluded — see `routes/auth.rs`'s doc comment for why.
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
- **Admin dashboard: Settings area** (request logs, backups — including
  upload/restore, cron jobs, and a Network page for rate limits/trusted
  proxy/superuser IPs), **collection export/import**, a **geoPoint field
  editor**, **per-collection auth-provider UI**, **design system pass**
  (light/dark contrast), **sidebar system-collection grouping**,
  **type-aware records table**, and **field-editor UX polish** (per-type
  option panels, inline validation, a syntax-help popover on every rule
  input).

## Later

- Realtime beyond a single node (Postgres `LISTEN/NOTIFY` or a queue as the
  fan-out layer, so `RealtimeHub` isn't in-process-only).
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
