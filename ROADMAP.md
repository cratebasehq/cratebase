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
  - **Cron jobs as a plugin** (`plugins/cron_jobs.rs`) — real calendar
    cron expressions (`croner`), reading job definitions from a
    `_cron_jobs` collection instead of being hardcoded, with
    `lastRunAt`/`lastStatus` written back per run. Job bodies are a small
    built-in registry (`run_job`), not scripted — add a match arm and
    ship your own binary for a new job type.
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
  - **Team management as a plugin** — multiple admins with roles is a
    real data-model change (today there is one `_admins` table, no
    roles); once record-lifecycle hooks exist on `Plugin` this can enforce
    role checks without touching the core auth crate.
  - Record lifecycle hooks (`on_create`/`on_update`/`on_delete`) are not
    on the trait yet — add them when the first plugin actually needs one,
    rather than speculatively.
- **View collections.** The `Collection::View` type and `view_query` field
  already exist in the schema, but nothing executes the backing SQL yet —
  `list_records`/`get_record` assume a real physical table. Needs: safe
  view-query validation, read-only enforcement, and dashboard UI.
- **Relation dot-notation in filters** (`author.name = "..."`) — currently
  filters only see the record's own columns.
- **"Any of" filter operators** (`?=`, `?!=`, ...) for multi-value fields.
- **Batch API** (`POST /api/batch`) — transactional multi-record
  create/update/upsert/delete in one request.
- File field constraints in the dashboard UI (`mimeTypes`, `maxSize` are
  already schema fields but have no editor).
- File thumbnails (`?thumb=WxH`) and protected-file access tokens (today
  every file download is already gated by the owning record's `viewRule`
  — see `routes/files.rs` — so this is about convenience/perf, not a
  security gap).

## Shipped

- **Auth rate limiting.** `/admins/auth-with-password`,
  `/collections/{c}/auth-with-password`, and the three `request-*` email
  flows below are rate-limited per client IP (`AUTH_RATE_LIMIT_ENABLED`,
  on by default). `auth-refresh` is deliberately excluded — see
  `routes/auth.rs`'s doc comment for why.
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

- **OAuth2 (Google, GitHub).** `crates/server/src/oauth2.rs`: authorization-
  code exchange + provider-specific userinfo parsing. `GET
  /collections/{c}/auth-methods?redirectUri=...` returns each configured
  provider's ready-to-open `authUrl`; `POST
  /collections/{c}/auth-with-oauth2` does the exchange and signs in — via
  an existing link (`_external_auths`), an auto-linked matching email, or
  a newly created (pre-verified) record. Configuring a provider is two env
  vars (`OAUTH_GOOGLE_CLIENT_ID`/`_SECRET`); unconfigured providers just
  don't appear in `auth-methods`. Adding a provider beyond Google/GitHub
  means a branch in `providers_from_env`/`fetch_user`, not a new
  abstraction — the two providers' userinfo shapes already differ enough
  (GitHub's email is a separate scoped call) that a generic trait would
  just wrap a `match`.

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
