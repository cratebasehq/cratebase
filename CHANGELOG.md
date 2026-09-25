# Changelog

All notable changes to this project are documented in this file. The
format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project is pre-1.0 (currently `0.3.0`, per `Cargo.toml`); the 0.1.0
entries below are grouped by merged pull request rather than by release
tag, reconstructed from the actual merge history (`git log --merges` /
`gh pr list --state merged`) on this repository, since they predate the
first real tagged release.

## [Unreleased]

### Added

- **Postgres extension management**: `GET/POST/DELETE /api/db/extensions[/{name}]`
  (superuser only, Postgres only — 404s on SQLite), plus a
  `Settings → Database extensions` dashboard page. `CREATE EXTENSION IF
  NOT EXISTS` with an optional schema; `DELETE` refuses `CASCADE` unless
  `?cascade=true` is passed. Every install/drop is audited. `$app.db().exec(sql,
  params?)` — a new write-capable escape hatch alongside the existing
  read-only `$app.rawQuery` — lets a `pb_migrations/*.js` file version
  `CREATE EXTENSION postgis` the same way it versions schema changes.
- **Custom SQL RPC**: the `_rpc` system collection (`name`, `sql` with
  `:name`-style named placeholders, a `params` json-schema-lite array,
  a `rule`, `readOnly`/`timeoutMs`/`maxRows`) plus `POST /api/rpc/{name}`.
  Named parameters bind as real driver parameters, never
  string-interpolated; `rule` is evaluated against
  `@request.auth`/`@request.body` before the statement runs; `readOnly`
  (default `true`) is enforced by the database itself, reusing the same
  `BEGIN READ ONLY`/`PRAGMA query_only` machinery the SQL console uses to
  reject a write disguised as a read; a definition's `sql` is validated
  — single statement, only declared parameters, parses against the real
  driver — at save time, not on first call. New dashboard "RPC
  functions" page (CodeMirror SQL editor, params/rule editors, a Run test
  panel) and SDK method `cb.rpc<T>(name, params)`.
- **PostGIS-accelerated geo queries**: `sort=geoDistance(lon, lat, x, y)`
  (and `-geoDistance(...)`) for a nearest/farthest-first order, matching
  the existing `geoDistance(...) < r` radius filter's semantics exactly.
  On Postgres, once `postgis` is installed, both the radius filter and
  the nearest sort automatically compile against a GiST-indexed
  geography expression (`ST_DWithin`/KNN `<->`) instead of the portable
  haversine calculation — same syntax, an index-backed plan. The GiST
  index is created idempotently per `geoPoint` field as part of ordinary
  collection schema sync.

## 0.3.0 — 2026-09-25

Security-hardening and developer-experience release, the result of a
full audit (PR [#27](https://github.com/cratebasehq/cratebase/pull/27)).
**Everyone on 0.2.0 should upgrade** — several fixes below close
privilege-escalation, stored-XSS and data-loss paths.

### Upgrading from 0.2.0 (breaking changes)

- **First-run setup needs an install token.** `POST /api/setup` now
  requires the one-time token printed in the server log at boot (or
  `CB_SETUP_TOKEN`), sent as `token` in the body or `X-Setup-Token`.
  `@cratebase/client`'s `admin.setup()` takes it too. The dashboard reads
  it from the logged link. `cratebase superuser create` is unchanged.
- **Auth rate limiting is on by default** for fresh installs
  (`AUTH_RATE_LIMIT_ENABLED=false` opts out). Existing installs keep their
  stored settings.
- **Signing secrets are validated.** An empty `CB_SECRET`/`AUTH_SECRET`
  is ignored (the generated `.secret` is used) and a non-empty one shorter
  than 32 bytes refuses to boot.
- **Active file types are served as downloads.** html/svg/xml/js uploads
  get `Content-Disposition: attachment` plus a sandbox CSP; upload
  `mimeTypes` checks now use the sniffed type, not the client's header.
- **`.env.example` / `docker-compose.yml` use the env vars the server
  actually reads** (e.g. `S3_ACCESS_KEY`/`S3_SECRET`, `CB_SENDER_ADDRESS`,
  `SMTP_TLS`). Names like `STORAGE_DRIVER`, `MAIL_DRIVER`,
  `S3_ACCESS_KEY_ID` were never read — update any `.env` copied from the
  old example.
- **Passwords are capped at 256 bytes.**
- **Generated TypeScript types use `type` aliases instead of
  `interface`**, which makes `createClient<Schema>()` type-check.

### Security

- SQL console: a data-modifying CTE could bypass the read-only gate on
  Postgres (e.g. promote an admin to owner). Reads now run in a
  `READ ONLY` transaction with a real `statement_timeout`.
- Stored XSS via uploads closed: magic-byte MIME sniffing, sandbox CSP on
  file responses, global `nosniff`/`Referrer-Policy`, frame protection on
  the dashboard.
- Backup restore no longer deletes the live database when `DATABASE_URL`
  isn't `data.db`, and keeps `.secret` and `plugins/`; swaps roll back on
  failure.
- `POST /api/setup` race (concurrent calls created several owners) fixed.
- Tokens are redacted from request logs; `$http.send` has a default
  timeout; `rustls` bumped to 0.23.45 (RUSTSEC-2026-0285).

### Added

- **`cratebase dev`** — one command for local development: data dir,
  superuser, `schema.json`, seed data, TypeScript types, hook hot reload.
- **`cratebase typegen`**, `GET /api/typegen` and a `--dev` watch
  (`CB_TYPEGEN_OUT`); select unions, relation `expand` and Create/Update
  input types.
- **`cratebase seed` / `cratebase reset`**, `pb_seed/` / `CB_SEED_DIR`.
- **JS migrations actually run** (`pb_migrations/`, on boot and via
  `migrate up/down/collections/history-sync`), and `--automigrate`
  writes migration files.
- **Postgres**: TLS via `sslmode` (including `verify-full`),
  `DB_POOL_SIZE`, and backup/restore via `pg_dump`/`pg_restore`.
- **Dev mail inbox** in the dashboard (Settings → Mail inbox) whenever no
  SMTP/Resend is configured.
- Scheduled backup status and failures surfaced in the dashboard and
  audit log; CIDR entries in the superuser IP allowlist; `/api/health`
  checks the database.
- `examples/team-board` flagship app, `.devcontainer` for Codespaces,
  and a rewritten landing page.

- **`@cratebase/react` upgrade**: a `CratebaseProvider`/`useCratebase()`
  context (every hook keeps working with an explicit client too),
  `createCratebaseHooks<Schema>()` for collection-name-to-record-type
  inference (mirroring `createClient<Schema>()`), pagination and
  refetch-on-event realtime on `useRecords`, and three new hooks:
  `useInfiniteRecords` (load-more pagination), `useMutation`
  (create/update/remove with pending/error state and optional
  optimistic apply/rollback), and `usePresence` (wraps
  `client.presence.track`). `useAuth` now also returns `isLoading` and
  `signIn`/`signOut`. `sdk/js/{client,react,extras}` joined the root bun
  workspace and CI (`bun run sdk:check`), with a new bun-test suite for
  every hook against a mocked client and a `tsc`-checked
  type-inference regression fixture.

### Fixed

- **Auto-embeddings ran before `onRecordCreate`/`onRecordUpdate` hooks.**
  A hook that derives or overwrites a `vector` field's `sourceField` was
  embedded against stale pre-hook text, since `apply_embeddings` ran
  before any request hook had a chance to run at all. Embeddings are now
  computed inside `write_record`, after the create/update hook has run
  and before the record is persisted, matching PocketBase's own
  `onRecordCreate -> e.next() -> persist` ordering — fixed for
  create, update, and `POST /api/batch` alike.
- **`settings.teams.enabled` needed a restart to take effect.**
  `App::bootstrap` only bound the `_teams` owner-bootstrap hook when the
  setting was already on at boot, so enabling Teams via a running
  server's `PATCH /api/settings` left every `_teams` row created
  afterwards ownerless until the next restart. The hook is now always
  bound and checks the current setting live, on every `_teams` create.
- **The `--dev` hook-file watcher missed same-mtime content edits.**
  `spawn_watcher`'s change detection compared only path and modification
  time; rewriting a `pb_hooks/*.pb.js` file's content while its mtime
  happened to land on the same value as before (trivial editing a short
  string constant in place) went unnoticed, leaving the stale hook bound
  until some later, differently-timed edit came along. The watcher now
  also hashes each file's content.
- **A running server never saw a collection created by a separate
  process** (most notably `cratebase schema push`) **sharing the same
  database** — every request for it 404'd with "Missing collection
  context." until the server restarted, because the in-memory collection
  cache only ever reloaded on this process's own writes. A new
  background poll (`App::start_schema_watch`, both SQLite and Postgres)
  compares a cheap `_collections` fingerprint against what is currently
  cached and reloads the moment they disagree, with no restart needed;
  `cratebase schema push` also now prints a heads-up when a server
  appears to be listening on the configured port.

## 0.2.0 — 2026-09-09

### Added

- **First-class auth: `@cratebase/client` SDK, sessions, cookies, OAuth2
  redirect, bans.** A new first-party typed TypeScript SDK
  (`@cratebase/client`, `sdk/js/client`) covering records/auth/realtime/
  files/batch/admin plus the value-add surface (vector/LLM/MCP/queue/
  presence) built in, publishing independently via `client-v*` tags. On
  the server: `_sessions`/`_bans` system collections with O(1) in-memory
  revocation and no DB hit on the verify hot path; opt-in httpOnly cookie
  sessions (`SESSION_COOKIE*`) with a same-origin CSRF gate and
  credentialed CORS; a server-driven OAuth2 redirect flow (`GET
  .../oauth2/{provider}/start` + `.../callback`) alongside the existing
  `POST auth-with-oauth2`; and session/ban/impersonation routes
  (sign-out, sessions list/revoke/revoke-others/revoke-all,
  stop-impersonating, ban/unban). The admin dashboard migrated off the
  `pocketbase` npm SDK onto `@cratebase/client`, gained a Sessions
  settings panel and Impersonate/Ban actions on auth records, and its
  OAuth2 provider editor now shows the exact callback URL to register
  plus a per-provider (Google/GitHub/GitLab/Discord/Microsoft) setup
  guide with a direct console link.
- **ZIP-export plugin**: a generic, toggle-gated plugin
  (`settings.zipExport.enabled`, off by default) that bundles every file
  in a chosen collection field, for records matching an arbitrary
  filter, into one ZIP archive via `POST /api/plugins/zip-export/enqueue`
  (superuser-only), with the same transactional claim and stale-row
  reclaim as the Queue plugin and path-traversal-safe entry names.
- **`$app.rawQuery(sql, params?)` (JS hooks)**: a read-only escape hatch
  for SQL a `findRecordsByFilter` filter string can't express (`GROUP
  BY`, window functions, joins) — runs a `SELECT`/`WITH` statement with
  `{:name}`-bound parameters and returns plain rows with no `Record`
  hydration, closing [#15](https://github.com/cratebasehq/cratebase/issues/15).

## 0.1.0 — 2026-09-04

### Added

- **Batch API, plugin foundation, and a YAGNI pass**: the Batch API
  (`POST /api/batch`), API keys that can optionally act as a real
  record instead of always granting superuser access, a
  wasmtime-sandboxed third-party plugin runtime with a manifest format
  and local install command, Teams/the LLM chat gateway/a new
  pg_boss-style Queue plugin converted to toggle-gated built-in modules
  (off by default, zero background cost), and removal of several
  speculative features that never earned their place (an analytics
  beacon, SMS/Twilio dead code, an unused QR endpoint, an
  agent-memory pattern example).
- **Third-party adoption pass**: contribution/security/changelog docs,
  the published `@cratebase/extras` npm package, GHCR Docker image
  publishing, a PocketBase migration tool/guide, OSS community health
  files (Code of Conduct, issue/PR templates), and
  `@cratebase/schema-codegen` made publish-ready (schema-as-code to
  generated TypeScript types).
- **cratebase.dev**: a marketing site and full docs (Astro/Starlight),
  deployed to Cloudflare Pages, with an OG image/favicons/manifest/
  structured-data pass and a horizontal-scaling deploy guide.
- Repository transferred from `nicoaudy/cratebase` to the `cratebasehq`
  GitHub org; all repository references updated accordingly.
- **Value-add Wave 4** (PR [#10](https://github.com/cratebasehq/cratebase/pull/10)):
  fixed cross-node realtime on Postgres, incoming webhooks, OAuth2 login
  (Google/GitHub/custom providers), a DocMind example app, admin
  dashboard code-splitting, and an in-dashboard API docs tab.
- **Value-add Wave 3** (PR [#7](https://github.com/cratebasehq/cratebase/pull/7)):
  superuser-minted API keys, an MCP (Model Context Protocol) dashboard
  and client helpers, push notifications (Web Push/FCM/APNs), Postgres
  multi-node realtime via `LISTEN`/`NOTIFY`, schema-as-code, presence,
  and a security hardening pass.
- **Value-add Wave 1+2** (PR [#6](https://github.com/cratebasehq/cratebase/pull/6)):
  custom SQL cron jobs, outgoing webhooks, an in-dashboard SQL console,
  a file manager, vector search, MCP support, an LLM chat gateway, and
  team/workspace membership.
- **Full auth surface and remaining conformance gaps** (PR [#5](https://github.com/cratebasehq/cratebase/pull/5)):
  batch API, email verification/password reset/email-change confirmation,
  OTP passwordless login, MFA, superuser impersonation, new-location
  login alerts, first-run setup (no CLI required), and JS hooks
  (PocketBase-parity `pb_hooks/*.pb.js` support) — closing conformance to
  180/181 against the real PocketBase test suite.
- **Realtime (SSE)** (PR [#3](https://github.com/cratebasehq/cratebase/pull/3)):
  `GET /api/realtime` subscriptions, reaching 7/7 realtime conformance
  tests and 155/181 overall against the PocketBase SDK conformance suite.
- Manual macOS ARM64 build workflow (PR [#4](https://github.com/cratebasehq/cratebase/pull/4)).
- Initial worktree/project revamp establishing the current crate layout
  (`crates/core`, `crates/db`, `crates/filter`, `crates/storage`,
  `crates/auth`, `crates/jsvm`, `crates/mailer`, `crates/server`) and
  admin dashboard (PR [#1](https://github.com/cratebasehq/cratebase/pull/1)).

### Fixed

- Rebuilt a stale committed admin dashboard bundle and added the CI check
  that catches this drift going forward (PR [#9](https://github.com/cratebasehq/cratebase/pull/9)).
- CI: rebuilt dashboard `dist`, and ignored `RUSTSEC-2023-0071` (a Marvin
  Attack RSA timing side-channel with no upstream fix, reachable only
  through an unused RSA code path pulled in transitively for VAPID/Web
  Push signing — see the inline comment in
  `.github/workflows/ci.yml` for the full justification) (PR [#8](https://github.com/cratebasehq/cratebase/pull/8)).
- CI: fixed `jsvm` lifetime issues, a one-argument `cb.send` regression,
  and pinned an previously-unpinned Bun version so the committed dashboard
  bundle byte-comparison stopped failing spuriously (PR [#2](https://github.com/cratebasehq/cratebase/pull/2)).

[Unreleased]: https://github.com/cratebasehq/cratebase/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/cratebasehq/cratebase/compare/v0.1.0...v0.2.0
