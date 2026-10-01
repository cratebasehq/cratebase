# Changelog

All notable changes to this project are documented in this file. The
format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project is pre-1.0 (currently `0.6.0`, per `Cargo.toml`); the 0.1.0
entries below are grouped by merged pull request rather than by release
tag, reconstructed from the actual merge history (`git log --merges` /
`gh pr list --state merged`) on this repository, since they predate the
first real tagged release.

## [Unreleased]

### Fixed

- **`create-cratebase` search box returned nothing while typing** —
  `?search=` matches whole FTS5 words, so a partial word ("sear") found no
  rows until it was complete. The `nextjs` and `vite-react` dashboards now
  pass the box through `toPrefixSearch()` (`lib/cratebase.ts`), which turns
  each word into a `term*` prefix query.

## 0.6.0 — 2026-09-29

Background jobs, webhooks and cron you can rely on in production, plus
one-command starter kits (`bun create cratebase`).

### Added

- **JS job handlers** — `onQueueJob("name", (e) => {...})` in a
  `pb_hooks/*.pb.js` file handles background jobs with the same claim,
  retry and backoff as a Rust handler (a Rust handler for the same name
  wins). `$queue.enqueue(name, payload, { runAt, delay, maxAttempts,
  dedupeKey, priority })` is deferred past commit inside a record-write
  hook. `dedupeKey` makes enqueue idempotent among pending/in-progress
  jobs (`deduped: true` in the response); higher `priority` runs first.
  Canonical `POST /api/queue/enqueue` (superuser or API key;
  `/api/plugins/queue/enqueue` kept as an alias). Dead-letter surface:
  `POST /api/queue/jobs/{id}/retry` and `DELETE /api/queue/jobs/{id}`.
  SDK `cb.queue.{enqueue,retry,delete}`. Exactly-once claiming is proven
  across nodes sharing one Postgres database.
- **Durable webhook delivery** — every `_webhooks` dispatch is a
  `_webhookDeliveries` row delivered by an always-on worker: fixed retry
  schedule (1m, 5m, 30m, 2h, 6h; per-webhook `maxAttempts`, default 6),
  10s timeout, ≤2KB response excerpt, SSRF check on every attempt. Each
  attempt sends `X-Cratebase-Delivery` (stable across retries),
  `X-Cratebase-Timestamp` and `X-Cratebase-Signature: sha256=…` (HMAC of
  `"{timestamp}.{body}"`). `POST /api/webhooks/deliveries/{id}/replay`
  resends one. `settings.webhooks.disableAfterFailures` (default 50)
  auto-disables a failing webhook with an `_audit_log` entry;
  `settings.webhooks.deliveryRetentionDays` (default 14) prunes old rows.
- **Cron run history** — `_cronRuns` records every run of a SQL
  `_cron_jobs` row or JS `cronAdd` job (`jobId`, `source`, `status`,
  `startedAt`, `durationMs`, `message`).
- **Dashboard** — Settings → Automation gains a Queue tab (per-queue
  counts, payload viewer, retry/delete), a Deliveries view under
  Webhooks (with replay), and run history for every cron job.

### Changed

- `settings.queue.enabled` is now a live toggle — no restart. It gates
  only processing; enqueueing, retrying and deleting always work.
- Cron jobs (SQL and JS) claim each scheduled tick with a
  `UNIQUE (jobId, tickAt)` row in `_cronRuns` before running, so a
  multi-node cluster runs each tick on exactly one node — durable, not
  timing-dependent. Single-node SQLite behaves as before.
- **`create-cratebase`** — a scaffolding CLI (`bun create cratebase my-app` /
  `npm create cratebase@latest my-app`) with three templates (`nextjs`,
  `vite-react`, `expo`), each shipping a `schema.json`, `pb_seed/` demo
  data, a `pb_hooks/welcome.pb.js` signup email, and a `dev` script that
  runs `cratebase dev` and the frontend together. `nextjs`/`vite-react`
  wire up the full `@cratebase/client`/`@cratebase/react` surface (auth —
  password/OTP/magic-link/OAuth/TOTP — a realtime dashboard, file upload,
  search, notifications); `expo` is a lean password/OTP + realtime-list
  starter. See [`packages/create-cratebase`](./packages/create-cratebase)
  and [Starter kits](https://cratebase.dev/docs/getting-started/starter-kits/).

### Upgrading from 0.5.0

- Migrations `21` and `22` run automatically (`_webhookDeliveries`,
  `_cronRuns` with a unique tick claim, new `_webhooks` fields). Existing webhooks start recording deliveries and
  retrying on failure.
- Webhook receivers can now verify `X-Cratebase-Signature: sha256=…` over
  `"{X-Cratebase-Timestamp}.{body}"` and dedupe on `X-Cratebase-Delivery`.

## 0.5.0 — 2026-09-29

Full-text search, storage upgrades, in-app notifications, and realtime
channels with presence — the pieces every consumer app needs next.

### Added

- **Full-text search** — a `searchable` option on text/editor/email/url
  fields and a per-collection search language. Cratebase keeps an index
  in sync automatically (SQLite FTS5 external-content table; Postgres
  `tsvector` + GIN). `GET /api/collections/{c}/records?search=…` combines
  with `filter` (rules still enforced in SQL) and ranks by relevance
  unless `sort` is given; `search("…")` is also available in the filter
  language. Query text is sanitized (prefixes and phrases supported). At
  50k rows a selective query is ~31x faster than `~` LIKE on SQLite and
  ~10x faster than ILIKE on Postgres. SDK `list({ search })`; dashboard
  search box, "Searchable" checkbox and language picker.
- **Presigned direct uploads** — `POST /api/files/presign` validates the
  field's rules, `maxSize` and `mimeTypes`, returns a direct S3 PUT URL
  (same-origin URL on local storage) and a token that a normal
  create/update attaches after the server verifies the object.
  `_pendingUploads` + hourly expiry sweep. SDK `cb.files.upload()`, React
  `useUpload()` with progress.
- **Image transforms** — `?w=&h=&fit=&format=&q=` on the files route
  (`?thumb=` unchanged), cached, with allow-lists and limits against
  cache-busting.
- **Per-user storage quota** (off by default), scoped to presigned
  uploads via a per-collection owner field. Storage settings (S3, limits,
  quota) now live in Settings → Application → Storage.
- **Notifications** — `_notifications` system collection (recipient in
  any auth collection, rule-scoped to "my own", `readAt`-only updates
  for non-superusers) plus `$notify.send(...)`/`POST /api/notifications/send`:
  one call fans a notification out across in-app (realtime), email (the
  `notification` `_emailTemplates` row), and push (`_push_subscriptions`)
  channels. `GET /api/notifications/unread-count` (index-backed) and
  `POST /api/notifications/read-all` round out the surface the generic
  records API doesn't cover. Retention: `settings.notifications.retentionDays`
  (default 90, prunes read rows only).
- **Realtime channels & presence** — topics not tied to any record
  (`channel:<name>` on the existing `GET /api/realtime` SSE connection),
  authorized by `_channels` config rows (exact name or `prefix:*`
  pattern; `subscribeRule`/`publishRule` follow the same null/""/expr
  convention as `_emailTemplates.sendRule`; no matching row disables a
  channel by default). `POST /api/realtime/channels/{name}/publish`
  broadcasts `{event, data}` (413 past ~7.5KB, to stay under Postgres's
  8000-byte `NOTIFY` cap); `POST`/`GET .../presence` is a heartbeat/
  member-list pair with automatic leave on disconnect or a 45s TTL;
  `GET .../stats` backs a dashboard live inspector. Works across nodes on
  Postgres over the same `LISTEN`/`NOTIFY` bridge record events use,
  distinguished by a `"kind"` field. `$realtime.publish(...)` in JS
  hooks runs at the trusted-hook tier (no `publishRule` check).
- **Dashboard** — Settings → Automation → Realtime channels (CRUD
  `_channels` plus the live subscriber/presence-count inspector);
  Settings → Integrations → Notifications (retention); `_notifications`/
  `_channels` visible under System collections.
- **SDK** — `cb.notifications.{list,fullList,subscribe,unreadCount,markRead,markAllRead,send}`;
  `cb.channel(name).{subscribe,publish,presence.{track,list,onChange}}`;
  React `useNotifications()`, `useChannel(name)`, `usePresence(name, state)`.
  The previous collection-backed `usePresence` is renamed
  `useRecordPresence` (same implementation, still exported) now that
  `usePresence` means the new server-authoritative channel presence.

### Upgrading

- Migrations `19_add_pending_uploads` and
  `20_add_notifications_and_channels` run automatically, adding
  `_pendingUploads`, `_notifications`/`_channels` and seeding the
  `notification` email template.
- Channels are disabled until you add a `_channels` row, so nothing is
  exposed by default.
- `@cratebase/react`: if you used the old `usePresence(collection, data, options)`,
  rename the import to `useRecordPresence` — `usePresence` now takes
  `(channelName, state, options?)` instead.

### Fixed

- **SQLite full-text search query shape**: `search()`/`?search=` compiled its FTS5 predicate as
  `(rowid IN (SELECT rowid FROM {table}_fts WHERE {table}_fts MATCH $1)) = $2` — the boolean
  literal from `search(...) = true`/the parser's bare-predicate sugar bound as an extra parameter
  instead of folded away. Wrapped in that outer `= $2` against a value unknown at plan time,
  SQLite's planner couldn't recognize the semi-join as flattenable and fell back to a full
  `SCAN {table}` re-testing FTS5 membership per row, which is why this page used to document
  SQLite's FTS5 search as *slower* than a plain `~`/LIKE scan. `search(...)` now compiles straight
  to the bare predicate (and its negation to `NOT (...)`), so the planner drives the scan by
  `rowid` instead: `SEARCH {table} USING INTEGER PRIMARY KEY (rowid=?)`. Same fix applies to
  `search()` used from an API rule and to the `COUNT` half of a `?search=` request, since both
  share the same compiled `WHERE` fragment. See
  [Database → Full-text search](https://cratebase.dev/docs/database/full-text-search/) for the
  updated benchmark numbers.

## 0.4.0 — 2026-09-28

Email, database extensibility, consumer-app auth, and a dashboard where
every setting is configurable. Highlights:

- **Email platform** — editable templates (`{{var}}`, locales, branded
  layout) with a visual editor built on React Email Editor, redesigned
  default emails, `POST /api/mails/send` (callable from a frontend per
  template via `sendRule`), no-code triggers, a mail log, and magic-link
  login.
- **Database** — Postgres extension management, custom SQL RPC
  (`cb.rpc()`), nearest-first `sort=geoDistance(...)` with automatic
  PostGIS acceleration.
- **Auth** — Apple, Microsoft, Discord, GitLab, Facebook, X, LinkedIn,
  Slack, Twitch and Spotify presets, generic OIDC, linked accounts, TOTP
  2FA with backup codes.
- **Dashboard** — settings consolidated into 7 tabbed groups, every
  setting configurable without the API, an onboarding checklist.
- **MCP** — runtime tools (`call_rpc`, `send_email`, `search_nearby`) and
  superuser developer tools (schema, rules, email templates, logs, SQL).

### Upgrading from 0.3.0

- Migrations run automatically (`16_add_rpc`, `17_add_totps`,
  `18_refresh_default_email_templates`). Seeded email
  templates you never edited are refreshed to the new design; edited ones
  are left alone.
- `$mails.send`, email triggers and webhooks fired from after-success
  hooks now run **after the write commits** (and never for a write that
  rolls back) instead of inline.
- Dashboard settings URLs moved under 7 groups (`/settings/email?tab=…`);
  old URLs redirect.
- `@cratebase/client` 0.3.0 / `@cratebase/react` 0.3.0 add `cb.mails`,
  `cb.rpc`, `cb.auth.magicLink`, `cb.auth.totp`, `cb.auth.accounts` and
  `useMagicLinkCallback`.

### Added

- **Full-text search** (`?search=` on any collection's records list, and the `search()` filter
  function usable in any API rule): mark a `text`/`editor`/`email`/`url` field `searchable` from
  the field editor and the collection accepts full-text queries over it — SQLite's FTS5 virtual
  table + triggers, or Postgres's generated `tsvector` column + GIN index, kept in sync
  automatically whenever the searchable field set, the collection name, or (Postgres) the new
  per-collection `searchLanguage` setting changes. Ranks by relevance (SQLite `bm25`, Postgres
  `ts_rank`) unless an explicit `sort` is given. See
  [Database → Full-text search](https://cratebase.dev/docs/database/full-text-search/) for query
  syntax, relevance semantics, and SQLite-vs-Postgres `EXPLAIN`/benchmark numbers (Postgres's GIN
  index is ~9.5x faster than a `~`/ILIKE scan at 50k rows in this repo's own benchmark; SQLite's
  FTS5 predicate is ~31x faster than `~`/LIKE for a rare term, and roughly even for a term matching
  a fifth of the table, where fetching that many rows dominates either way).
- **Presigned direct uploads** (`POST /api/files/presign`, `PUT /api/files/presign-upload/{token}`):
  upload a file straight to storage (S3, signed — or a same-origin route for the local driver)
  instead of round-tripping its bytes through a multipart create/update, then claim it with a
  single-use token as the target field's value. `@cratebase/client`'s `cb.files.upload(file, {
  collection, field })` and `@cratebase/react`'s `useUpload()` hook (progress/pending/error state,
  `abort()`) wrap the flow. Unclaimed tickets expire after 30 minutes and are swept hourly.
- **Image transforms** on the files route: `?w=&h=&fit=cover|contain|inside&format=jpeg|png|webp&q=`,
  independent of and mutually exclusive with the existing `?thumb=`. Gated by the new
  `settings.storage.imageTransformsEnabled` (default on); a non-superuser request's `w`/`h` is
  clamped to `settings.storage.maxTransformDimension` (default 4000px) rather than rejected.
- **Per-user storage quota** (`settings.storage.userQuotaBytes`, default `0`/disabled): caps the
  bytes an auth record may store via the direct-upload flow above, scoped to whichever
  collections set the new `ownerField` schema setting (which field names a record's owner).
  Enforced at presign time with a `400`, before a ticket is minted.
- Dashboard: a "Searchable" checkbox on text/editor/email/url fields, a "Search language" and
  "Owner field" picker in collection settings, and a "Storage limits" section (image transforms +
  quota) under Settings → Email → Delivery → File storage.

- **Settings navigation consolidated to 7 tabbed groups** (`web/admin/src/lib/settings-nav.ts`
  and the `routes/settings-*.tsx`/`components/settings/*-settings-page.tsx` files): the ~24-item
  settings sidebar is now Application, Email, Auth & security, Database, Automation,
  Integrations, and Logs, each a single page with tabs addressed by a `?tab=` URL search param.
  Every old top-level route (`/settings/superusers`, `/settings/mail-log`, ...) redirects to its
  new group + tab, preserving its own filters. The command palette still jumps directly to any
  individual tab.
- **Collection auth options gap-fill** (`components/collections/auth-options-editor.tsx`,
  `lib/collection-form-value.ts`): sign-in and manage rules, `identityFields` as a real
  multi-select (email and/or username), login alerts, magic-link sign-in, OAuth2 mapped fields,
  per-collection verification/reset-password/confirm-email-change templates, and a write-only
  "regenerate secret" action per token kind (never displays a stored secret).
- **Field editor gap-fill** (`components/collections/schema-field-row.tsx`): text
  `autogeneratePattern`/`primaryKey`, the editor field's own max-size/convert-URLs panel
  (previously misrendered with the text field's min/max/pattern controls, which the server
  ignores for `editor` fields), email/URL `onlyDomains`/`exceptDomains`, date `min`/`max`, JSON
  `maxSize`, file `thumbs`/`protected`, relation `cascadeDelete`/`minSelect`, and password `cost`.
- **App-wide settings gap-fill**: `meta.accentColor`/`hideControls` and the batch API's
  `maxBodySize` now have UI in Application → Modules; Teams/Queue/ZIP export module toggles
  (Queue and ZIP export note that they need a restart; Teams applies immediately); Request logs
  gained `minLevel`, `maxDataSize`, and mail-log retention (`mailLogMaxDays`); Backups gained a
  "Scheduled backups" section for the cron expression and `cronMaxKeep`, neither of which had
  any dashboard UI before.
- **Onboarding checklist** on the dashboard home (`components/dashboard/onboarding-checklist.tsx`):
  a dismissible "Getting started" card computed from existing settings/collections APIs —
  app identity, branding, mail delivery + a sent test email, an auth collection's sign-in
  method, backups scheduled, rate limiting, and optional S3 storage.
- **10 new OAuth2 presets, generic OIDC, TOTP 2FA, and linked-account
  management** (`crates/auth/src/{oauth2,oidc,totp}.rs`,
  `crates/server/src/routes/{auth,oidc,totp,oauth2_flow}.rs`):
  - Apple, Microsoft (Entra ID), Discord, GitLab (self-hosted `baseUrl`
    supported), Facebook, X/Twitter, LinkedIn, Slack, Twitch, and
    Spotify join Google/GitHub as built-in `KnownProvider` presets —
    endpoints, scopes, PKCE, and userinfo parsing for each, table-driven
    tested. Apple's client secret is a per-request ES256 JWT signed
    with the team's private key (`extra.teamId`/`keyId`/`privateKey`,
    no static secret at all); its identity comes from a JWKS-verified
    `id_token` (no userinfo endpoint), its authorize URL requires
    `response_mode=form_post`, and its one-time `user` name field is
    only ever sent on the very first authorization — the redirect
    flow's callback now accepts `POST` as well as `GET` for this.
  - **Generic OIDC**: any provider configured with just `extra.issuer`
    is discovered via `/.well-known/openid-configuration` (cached, 1h
    TTL) and verified against the issuer's own JWKS (cached with rotation
    handling — a forced refresh retry on an unrecognized `kid`).
  - **Linked accounts**: `GET`/`DELETE
    .../records/{id}/external-auths[/{provider}]` (PocketBase's
    `listExternalAuths`/`unlinkExternalAuth` shape, owner-or-superuser),
    with a check PocketBase doesn't have — unlinking refuses when it
    would leave the record with no password, no other provider, and
    OTP/magic-link both disabled. SDK: `cb.auth.accounts.list()`/
    `.unlink(provider)`, plus `admin.listExternalAuths`/
    `unlinkExternalAuth` for a superuser acting on another collection.
  - **TOTP 2FA + backup codes**: `POST .../totp/setup` (returns the
    `otpauth://` URI + base32 secret), `.../totp/confirm` (enables,
    returns 10 one-time backup codes shown once), `.../totp/disable`
    (code or password), `.../totp/backup-codes/regenerate`, and `POST
    .../auth-with-totp {mfaId, code}` — integrated with the *existing*
    `_mfas` challenge rather than a second flow: a record with confirmed
    TOTP requires a second factor on every login independently of
    `authOptions.mfa`, and a collection using both still has exactly one
    challenge per login. RFC 6238 (SHA1, 6 digits, 30s, ±1 step) with
    replay protection (a step already used is rejected even if still
    numerically valid). New `_totps` system collection
    (`17_add_totps.rs`), secret encrypted with `CB_ENCRYPTION` when set.
    SDK: `cb.auth.totp.setup/confirm/disable/regenerateBackupCodes`,
    `cb.auth.signIn.totp({mfaId, code})`.
  - Dashboard: the collection auth-settings provider editor gained a
    preset-name picker, per-preset config fields (Apple's Team ID/Key
    ID/private key, Microsoft's tenant, GitLab's base URL, a generic
    OIDC issuer), and hid the raw auth/token/userinfo URL fields for
    any of them (fixing a pre-existing validation bug that demanded
    those three URLs even for a built-in preset). The record drawer now
    shows an auth record's TOTP status (with a superuser "Reset TOTP"
    button) and its linked OAuth2 providers (with per-provider Unlink).
  - Docs: dedicated pages for [Apple](/docs/concepts/authentication/apple/),
    [generic OIDC](/docs/concepts/authentication/oidc/),
    [linked accounts](/docs/concepts/authentication/linked-accounts/),
    and [TOTP 2FA](/docs/concepts/authentication/totp/) (including a
    `qrcode.react` example); a full preset table on the OAuth2 page; a
    "Cratebase vs better-auth" comparison on the auth overview page
    naming what's deliberately deferred (passkeys, SMS 2FA, SAML,
    anonymous auth, organizations/multi-tenancy).
- **Email platform**: an editable `_emailTemplates` system collection
  (`{{var}}` syntax — dotted paths, HTML-escaped, `{{{raw}}}` for
  unescaped, locale fallback, a shared branded base layout styled by new
  `settings.meta.logoUrl`/`brandColor`), seeded with the five built-in
  auth emails (`auth.verification`, `auth.passwordReset`,
  `auth.emailChange`, `auth.otp`, `auth.loginAlert`), `auth.magic-link`,
  and a `welcome` example. Each auth-flow email now resolves through a
  three-step priority chain — a customized `authOptions.*Template` field
  wins, else the matching `_emailTemplates` row, else the same built-in
  default — so an unmodified install's mail is unchanged.
- **Redesigned default email templates**: the branded base layout
  (`layout: true`) is now a centered ~600px card on a soft page
  background, a logo/wordmark header, a muted footer, dark-mode-friendly
  (`color-scheme` + `prefers-color-scheme` with safe fallbacks for
  clients that ignore it), and fully inline-styled/table-based. Every
  seeded template with a call to action now uses a bulletproof button
  (a real `<a>`, a VML fallback for Outlook, and a plain-text "copy this
  link" line so the URL survives even in a client that strips the
  button); the OTP template shows its code in a large, letter-spaced
  monospace block. New migration
  `17_refresh_default_email_templates.rs` rewrites the seven seed rows
  to this new copy/layout, but only for a row whose `subject`/`html`
  still exactly match what was originally seeded — a customized row is
  left untouched. Also adds the `_emailAssets` system collection, an
  unprotected single-file store the dashboard's email-template visual
  editor uploads images to (superuser-only writes, publicly downloadable
  by URL — an emailed `<img src>` has no session to authenticate with).
- **`POST /api/mails/send` / `/api/mails/preview`**: `preview` stays
  superuser/API-key only; `send` by `template` key or raw `subject`/
  `html`/`text`, up to 50 recipients, logged to a new `_mailLog` system
  collection (pruned by `settings.logs.mailLogMaxDays`, default 30),
  delivered inline (one retry) or via the durable job queue when
  `settings.queue.enabled`. `$mails.send(...)` in JS hooks goes through
  the same pipeline (never subject to `sendRule` below);
  `cb.mails.send`/`cb.mails.preview` in `@cratebase/client`.
- **`_emailTemplates.sendRule`**: an optional filter-rule expression (same
  language as a collection API rule) that lets a non-superuser caller —
  an authenticated app user, or even an anonymous one — call
  `POST /api/mails/send`/`cb.mails.send(...)` directly for that template,
  with no backend of its own. `null` (the default) keeps a template
  superuser/API-key only; `""` opens it to anyone. A non-superuser send
  is restricted to `to` + `template` (no raw content, no
  `from`/`cc`/`bcc`/`replyTo` override), capped at 5 recipients, and
  rate-limited on its own tighter `mails:send:user` tag
  (10/minute/auth-record by default) on top of the general `mails:send`
  ceiling. Also new: `design`/`editor` fields on `_emailTemplates`
  (opaque to the server, used by the dashboard's visual editor).
- **`_emailTriggers`**: a superuser-only system collection for sending an
  email template automatically when a record is created, updated, or
  deleted — no code, no redeploy, the same shape as `_webhooks`.
  Optional `condition` (a filter expression evaluated against the
  written record) and `dataMap` (extra literal template data merged onto
  the default `{ record }`). Fires from an after-success hook and can
  never fail the triggering request; a `toField` that resolves to
  nothing usable is logged to `_mailLog` as a failure instead.
- **Dashboard**: new Settings pages — **Email templates** (list, create/
  duplicate/delete, a starter gallery of nine ready-made documents on
  "new", and an editor with HTML mode via CodeMirror plus a full visual
  mode built on [`@react-email/editor`](https://react.email/docs/editor/overview)
  — headings/paragraphs/lists/quotes/code/tables/dividers/buttons/images/
  2-3-4-column layouts/links, a bubble menu, a `/` slash-command menu
  (including a custom `/variable` command), a Theme tab to pick a
  built-in theme and customize brand color/font/background/content
  width/radius, a Variables tab that inserts `{{path}}` chips from the
  sample data, and image uploads to `_emailAssets`, a `sendRule` field,
  a live preview through `POST /api/mails/preview`, and test send),
  **Email triggers** (CRUD over `_emailTriggers`), and **Mail log**
  (filterable, paginated view over `_mailLog`).
- **Magic-link login**: `POST /collections/{c}/request-magic-link` /
  `auth-with-magic-link`, gated by a new
  `authOptions.magicLink.enabled` (default `false`), backed by a new
  `_magicLinks` system collection. Same account-enumeration-resistant
  `request-*` contract as OTP, and the same ban/MFA/verified-on-success/
  login-alert handling as every other login path.
  `cb.auth.magicLink.request(...)` / `cb.auth.signIn.magicLink(...)` and
  `getMagicLinkTokenFromUrl(...)` in `@cratebase/client`;
  `useMagicLinkCallback(...)` in `@cratebase/react`.
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
- **MCP: runtime and superuser developer tools** (`crates/server/src/mcp.rs`),
  on top of the existing per-collection CRUD tools:
  - **Runtime tools** (visible to any caller, enforced exactly like the
    HTTP route they restate): `call_rpc` (`name`/`params`, gated by that
    `_rpc` row's own `rule` — reuses `crate::rpc::call_rpc` verbatim),
    `send_email` (`template`/`to`/`data`/`locale`, gated by that
    `_emailTemplates` row's `sendRule` — reuses a newly-extracted
    `routes::mails::send_mail` shared by `POST /api/mails/send` too, so
    the two never drift), and `search_nearby` (`collection`/`field`/
    `lon`/`lat`/`radiusKm`/`limit` — a `geoDistance(...)` filter/sort
    built for the existing `list_<collection>` tool, so it's enforced by
    that collection's own `listRule` and gets PostGIS acceleration for
    free where available).
  - **Superuser-only developer tools** — absent from `tools/list` and
    refused by name in `tools/call` for anyone else, so nothing confirms
    they exist to a caller who can't use them: `get_schema` (every
    collection's schema, OAuth2 secrets redacted); `apply_schema`
    (`dryRun` defaults to `true`, reuses `routes::schema::plan_and_apply`);
    `test_rule` (evaluate a filter-rule expression for a given auth
    record — or anonymous — and an optional row, returning allow/deny
    and the compiled SQL when it compiles without a database round
    trip); `list_email_templates`/`upsert_email_template` and
    `list_email_triggers`/`upsert_email_trigger` (CRUD on
    `_emailTemplates`/`_emailTriggers` — an `id` argument updates, its
    absence creates); `query_logs` (reuses `routes::logs::list`); and
    `sql_read` (the SQL console's read-only path only — `SELECT`/`WITH`
    only, row-capped, no write path at all).
  - Docs: [AI → MCP server](https://cratebase.dev/docs/ai/mcp-server/)
    documents connecting Claude Code/Claude Desktop with an API key and
    the full tool reference.

- **Docs sync pass for the email platform, database extensibility, and
  dashboard overhaul above**: README's feature list and crate map,
  ARCHITECTURE.md (new "Email platform"/"Database extensibility"
  sections, an updated auth-model section, a corrected single-node-only
  claim about realtime — cross-node Postgres realtime already shipped),
  ROADMAP.md (moved shipped items out of stale descriptions, added an
  explicit deferred-features list), the REST API index (stale path
  counts, a missing Mails row), the dashboard tour page (onboarding
  checklist, the auth-options editor), `.claude/skills/cratebase`
  (recipes for sending mail via `sendRule` from a frontend, magic-link +
  TOTP login, an RPC nearest-location query, and `_emailTriggers`), both
  SDK READMEs (`cb.mails`, `cb.rpc`, `cb.auth.magicLink`/`.totp`/
  `.accounts`, `useMagicLinkCallback`), and the stale `sms` settings
  section (dead code, already removed from `crates/core/src/settings.rs`).

### Fixed

- A `sort=geoDistance(...)` with no accompanying `filter` (or, on
  Postgres, one whose filter bound fewer parameters than the sort did)
  could 500 instead of returning results: `Query`'s `ORDER BY`/`LIMIT`/
  `OFFSET` parameters were appended to the same vec `count_sql()` — which
  has neither clause — was executed against, so the driver rejected the
  mismatched parameter count outright once there was no `filter` around
  to happen to absorb it. `crates/db/src/query.rs`'s `Query` now tracks
  those separately (`Query::all_params()`) so `count_sql()` only ever
  sees the parameters its own placeholders need. Found and fixed while
  building the `search_nearby` MCP tool above, whose happy path exercises
  exactly this shape.
- **`$mails.send` deadlocked when called from `onRecordAfterCreateSuccess`/
  `onRecordAfterUpdateSuccess`** — exactly the pattern the docs recommend
  ("send a welcome email after signup"). Those hooks' `e.app` is a `TxApp`
  bound to the write's still-open transaction; `$mails.send` wrote its
  `_mailLog` row through the plain connection pool, which contends for the
  single SQLite writer connection that transaction already holds — the
  same task waiting forever on a lock only itself could release.
  `_emailTriggers` and `_webhooks` had the same transaction-vs-plain-pool
  mismatch in their reactive dispatch, minus the deadlock (they already
  bypassed the open transaction), but could still fire for a write that
  later rolled back. All three now queue their DB write and delivery/
  network I/O behind a new `TxApp::after_commit` (`crates/server/src/
  app.rs`), which runs the queued work, detached, only once the
  triggering transaction actually commits, and drops it unrun on
  rollback — so `$mails.send` (and `_emailTriggers`/webhook dispatch) from
  an after-success hook never deadlocks, never contends for the writer
  lock, and never sends for a write that didn't happen.
- **A flaky `routes::functions` test** (`missing_hooks_dir_returns_empty_lists`)
  — and, more importantly, every other test built the same way — passed a
  bare `tempfile::tempdir()` root straight to `Config::memory`/
  `Config::for_data_dir`. `Config`'s `hooks_dir`/`migrations_dir` are
  *siblings* of the data dir (`<data_dir>/../pb_hooks`), so a `data_dir`
  that *is* the tempdir root resolves those siblings one level up, outside
  the tempdir entirely — into the real, shared OS temp directory, where
  they collide with every other test (and every other run) computing the
  same path. Fixed at the root: every affected test now nests the data
  dir under the tempdir (`dir.path().join("pb_data")`), so `pb_hooks`/
  `pb_migrations` land inside the tempdir like everything else the test
  creates, instead of leaking into `/tmp`.

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
