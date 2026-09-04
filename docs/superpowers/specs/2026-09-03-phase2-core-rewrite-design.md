# Phase 2 — Core rewrite: PocketBase v0.23+ parity, storage engine, hooks, JS runtime

Date: 2026-09-03. Status: approved (owner delegated to recommendations).
Inputs: `docs/superpowers/audits/2026-09-03-full-audit.md`, the PocketBase
v0.40.2 response fixtures in `docs/superpowers/specs/pb-fixtures/`, and
`docs/superpowers/specs/2026-09-03-phase1-perf-quick-wins-design.md`.

## 1. Goal and non-goals

**Goal.** After this phase, the official PocketBase JS SDK (`pocketbase`
npm, v0.28) works against Cratebase unchanged for every documented
endpoint, and Cratebase is faster than PocketBase on every cell of
`benchmarks/run.sh`. Extension is possible three ways: Rust (typed hooks
on an `App`), JavaScript (`pb_hooks/*.pb.js`, PocketBase's JSVM contract),
and HTTP webhooks (a built-in plugin). Postgres remains a first-class
backend selected by `DATABASE_URL`.

**Non-goals for this phase.** The admin dashboard (Phase 3). Value-adds
beyond parity (Postgres multi-node realtime, MCP server, vector/FTS,
schema-as-code, WASM plugins). Upgrading databases created before this
phase: the on-disk layout changes (tables are named after collections,
ids are 15 chars, system collections replace `_admins`), and the project
is pre-1.0 with no external users, so there is **no migration from the
old `cb_*` layout**. A fresh data directory is required.

## 2. Compatibility contract

The contract is PocketBase v0.40.2's HTTP behavior as observed, not the
docs. Concretely:

- **Every URL, method, query parameter, request body, response body, and
  status code** in PocketBase's public API. The fixtures directory holds
  captured responses; implementers add fixtures whenever they need a
  shape not yet captured (start the PocketBase binary in
  `/tmp/pb-bench/pb`, hit the endpoint, save the JSON).
- **Error body** is `{"status": <int>, "message": "...", "data": {...}}`
  (v0.23+ renamed `code` to `status`). Validation errors put
  `{"code": "validation_required", "message": "..."}` per field under
  `data`. Messages match PocketBase's wording where the fixture shows it
  (`"Failed to create record."`, `"Missing collection context."`,
  `"Something went wrong while processing your request."`).
- **Dates** serialize as `2026-09-03 12:44:06.146Z` (space separator,
  millisecond precision, `Z`). Inputs accept that form and RFC3339.
- **Ids**: records `[a-z0-9]{15}`; collections `pbc_` + crc32(name) as
  decimal (`pbc_2279338944`); the built-in users collection keeps
  PocketBase's literal id `_pb_users_auth_`; fields `<type><crc32(name)>`
  (`text3208210256`). Superuser records are ordinary auth records.
- **Tables are named after collections** (`posts`, `users`,
  `_superusers`), so `viewQuery` SQL written for PocketBase works.
- **Tokens** are HS256 JWTs with claims `{id, type, collectionId,
  refreshable, exp}` signed with `settings.secret + record.tokenKey`
  (auth tokens) or `secret + tokenKey + purpose` for action tokens,
  exactly as PocketBase, so rotating `tokenKey` invalidates a record's
  sessions.
- **JSVM globals** match PocketBase's `pb_hooks` API for the subset in
  §12.

Where PocketBase behavior is arguably a bug, we still match it; a
deliberate divergence is listed in §15 with the reason.

## 3. Crate layout

```
crates/
  core     domain model (PB shapes), ids, dates, errors, events, settings
           types. No I/O.
  filter   full PB filter grammar → parameterized SQL. No I/O.
  db       storage engine: `Engine` trait, SQLite (rusqlite, read pool +
           single writer) and Postgres (tokio-postgres + deadpool)
           implementations, schema sync (DDL from Collection), migrations
           ledger, record queries (list/view/expand), writes, validation.
  auth     Argon2id + bcrypt-verify (imported PB hashes), JWT with
           tokenKey, OTP codes, PKCE helpers.
  storage  unchanged public API + streaming `put`.
  mailer   unchanged + `MailBackend` trait made public so plugins can
           replace it; templates rendered from collection-level
           `{APP_NAME}`/`{TOKEN}` templates.
  jsvm     QuickJS runtime (rquickjs) hosting pb_hooks and pb_migrations.
  server   `App` (builder, lifecycle, hooks, cron, store), services
           (records, collections, auth, files, realtime, settings,
           backups, logs), axum routes, plugin registry, dashboard
           embed, CLI.
```

Dependencies flow `server → jsvm → server`? No: `jsvm` depends on
`server`'s public `App` API (it is a plugin). `server` depends on
`db/auth/storage/mailer/filter/core`. `db` depends on `filter/core`.

## 4. Domain model (`crates/core`)

### 4.1 Collection

```rust
pub struct Collection {
    pub id: String,
    pub name: String,
    pub r#type: CollectionType,          // base | auth | view
    pub system: bool,
    pub fields: Vec<Field>,
    pub indexes: Vec<String>,            // raw CREATE INDEX statements
    pub list_rule: Option<String>, view_rule, create_rule, update_rule, delete_rule,
    pub created: DateTime, pub updated: DateTime,
    // view
    pub view_query: String,              // serialized only for views
    // auth (serialized only for auth collections)
    pub auth_rule: Option<String>,
    pub manage_rule: Option<String>,
    pub auth_alert: AuthAlert { enabled, email_template },
    pub oauth2: OAuth2 { enabled, providers: Vec<OAuth2Provider>, mapped_fields },
    pub password_auth: PasswordAuth { enabled, identity_fields: Vec<String> },
    pub mfa: Mfa { enabled, duration, rule },
    pub otp: Otp { enabled, duration, length, email_template },
    pub auth_token, password_reset_token, email_change_token,
        verification_token, file_token: TokenConfig { duration: i64 },
    pub verification_template, reset_password_template,
        confirm_email_change_template: EmailTemplate { subject, body },
}
```

Serde: camelCase; `type` via rename; auth-only and view-only blocks
serialized conditionally (custom `Serialize` that matches the fixture:
base collections carry none of them). Defaults for a new auth collection
are the fixture's `users` values (templates included, verbatim).

Rules: `None` = superuser only, `Some("")` = public, `Some(expr)` =
filtered. Rule strings are validated against the filter grammar at save.

### 4.2 Field

Internally tagged enum, one struct per type, options flat (fixture):

| type | options |
|---|---|
| text | min, max, pattern, autogeneratePattern, primaryKey |
| editor | maxSize, convertURLs |
| number | min, max, onlyInt |
| bool | — |
| email | exceptDomains, onlyDomains |
| url | exceptDomains, onlyDomains |
| date | min, max |
| autodate | onCreate, onUpdate |
| select | values, maxSelect |
| file | maxSelect, maxSize, mimeTypes, thumbs, protected |
| relation | collectionId, cascadeDelete, minSelect, maxSelect |
| json | maxSize |
| password | min, max, pattern, cost |
| geoPoint | — (stored as JSON `{lon, lat}`) |

Common: `id, name, system, hidden, presentable, required, help`. A field
is multi-valued when `maxSelect > 1` (select/file/relation). Serialize
every option even when zero (fixture shows `"max": 0`).

System fields PocketBase adds by default and we add on create when
absent: `id` (text, primaryKey, `[a-z0-9]{15}`), and for auth
collections `password` (hidden), `tokenKey` (hidden, autogenerate
`[a-zA-Z0-9]{50}`), `email`, `emailVisibility`, `verified`. `created` /
`updated` autodate fields are added by the scaffold, not enforced.

### 4.3 Record

`Record` is an owned map `IndexMap<String, Value>` plus
`collection: Arc<Collection>`, with helpers: `id()`, `get(field)`,
`set(field, value)`, `expand`, `custom_data`, `hidden` handling
(`password`/`tokenKey`/any `hidden` field never serialized except via
`with_hidden(true)` for superusers with `?fields` requesting it — PB
never returns them). `email` stripped unless `emailVisibility` or the
caller is the record itself/superuser/`manageRule` passes.

Serialization order follows field order, then `collectionId`,
`collectionName`, then `expand` when present (fixture order is
alphabetical because PB uses a sorted map; we sort keys too).

### 4.4 Settings

Exactly the fixture (`settings.json`): `meta, smtp, s3, backups,
batch, rateLimits, trustedProxy, logs, superuserIPs`. Secrets (`smtp.password`,
`s3.secret`, `backups.s3.secret`) are write-only: accepted on PATCH,
never returned. Persisted in `_params` as JSON under key `settings`,
optionally AES-256-GCM encrypted when `CB_ENCRYPTION` (32 chars) is set.
Env vars from the current `Config` seed the settings on first boot and
override at runtime only when the env var is explicitly set (documented
per var).

### 4.5 Ids, dates, errors

- `ids::record()` 15 lowercase alphanumerics from a CSPRNG;
  `ids::collection(name)`, `ids::field(type, name)` via crc32.
- `DateTime` newtype over `chrono::DateTime<Utc>` with PB formatting on
  serialize and lenient parse.
- `AppError` gains `status()` and renders `{status, message, data}`.
  Validation codes follow PB (`validation_required`,
  `validation_invalid_email`, `validation_values_mismatch`,
  `validation_min_text_constraint`, ...). A table lives in
  `core/src/validation_codes.rs`.

### 4.6 Events

Defined in `core` so `db` can emit them without depending on `server`:

```rust
pub enum RecordAction { Create, Update, Delete }
pub struct RecordChanged { action, collection_id, record: Record, previous: Option<Record> }
```

## 5. Storage engine (`crates/db`)

### 5.1 `Engine` trait

```rust
pub enum Sql { Null, Int(i64), Real(f64), Text(String), Blob(Vec<u8>) }
pub struct Row { pub columns: Arc<[String]>, pub values: Vec<Sql> }

#[async_trait]
pub trait Engine: Send + Sync {
    fn dialect(&self) -> Dialect;
    async fn query(&self, sql: &str, params: &[Sql]) -> DbResult<Vec<Row>>;
    async fn execute(&self, sql: &str, params: &[Sql]) -> DbResult<u64>;
    async fn begin(&self) -> DbResult<Transaction>;   // write transaction
}
pub struct Transaction { /* same query/execute, plus commit()/rollback() */ }
```

Placeholders are `$1..$n` in both dialects (rusqlite accepts `?NNN`; the
SQLite implementation rewrites `$n` → `?n` once per statement string and
caches the rewrite alongside the prepared statement).

**SQLite** (`rusqlite`, bundled, features `bundled`, `json`, `load_extension`):
- `readers`: N connections (N = cores clamped 4..16) behind a semaphore;
  each query runs on `spawn_blocking` holding one connection. Every
  connection gets the Phase 1 PRAGMAs plus `query_only=ON` on readers.
- `writer`: exactly one connection behind a `tokio::sync::Mutex`. Every
  write statement and every transaction goes through it, so there is
  never `SQLITE_BUSY` between our own connections; `busy_timeout` stays
  for external processes. A `Transaction` holds the guard; statements
  inside it run on `spawn_blocking` with the connection borrowed through
  an `Arc<std::sync::Mutex<Connection>>`. Hooks may `await` inside the
  transaction because the guard is async.
- `prepare_cached` for every statement; cache size 256 per connection.
- `logs`: a second `SqliteEngine` on `<data>/auxiliary.db` (PB's name),
  used only by the logs service, on every backend.

**Postgres** (`tokio-postgres` + `deadpool-postgres`): one pool; `begin`
checks out a connection. `Sql` maps to `text/double precision/bigint/bytea`.
Physical column types are the same three shapes as today for the record
tables (TEXT, DOUBLE PRECISION, INTEGER-as-bool) so filter SQL is shared;
JSON columns are TEXT with `::jsonb` casts in the compiler as now.

### 5.2 Schema sync

`schema::sync(engine, previous: Option<&Collection>, next: &Collection)`
computes DDL: create table, add/drop/rename columns (rename detected by
field id), type changes via drop+add (documented), then `indexes`:
every statement in `collection.indexes` is executed after dropping
indexes that no longer appear. Index statements are parsed enough to
extract the index name and validate the table name (PB does the same).
Views: `DROP VIEW; CREATE VIEW name AS <viewQuery>`, and the view's
`fields` are derived from `PRAGMA table_info`/`information_schema` on
save, as PB does.

System tables: `_collections` (id, name, type, system, fields JSON,
indexes JSON, rules, options JSON, created, updated), `_params` (key,
value, created, updated), `_migrations` (file, applied), and the logs
database's `_logs` (id, level, message, data JSON, created, updated)
with an index on `created` and one on `level`.

### 5.3 Collections cache

Phase 1's `CollectionCache` becomes the only read path:
`Db::collections()` returns `Arc<Vec<Arc<Collection>>>` (arc-swap), fully
loaded at boot and replaced atomically on any change. Lookups by id or
name are hash lookups on an `Arc<CollectionIndex>`.

### 5.4 Records read path

`records::list(db, ctx, collection, ListParams { page, per_page, sort,
filter, expand, fields, skip_total }) -> ListResult`:

1. Rule → compiled WHERE (cached AST, see §6).
2. User filter → compiled WHERE.
3. Sort: `-created`, `@random`, `@rowid`, relation dot-notation sort
   (`author.name`) via the same subquery mechanism as filters.
4. `SELECT <table>.* ... LIMIT/OFFSET`; `COUNT(*)` only when
   `skip_total` is false (`totalItems: -1` otherwise).
5. Rows decoded straight into `Record` values by field type.
6. Expand: `expand::resolve(db, ctx, records, expand_spec, depth ≤ 6)`.
   Per level, group ids by relation field, one `WHERE id IN (...)`
   query per target collection applying that collection's `viewRule`
   for non-superusers (PB behavior: unexpandable records are simply
   omitted). Back-relations `comments_via_post` query the referencing
   collection with `post = ? OR post LIKE '%"id"%'` semantics (multi
   relation), capped at 1000 per PB.
7. `fields` projection with `*` wildcard and `:excerpt(N, withEllipsis)`
   applied at serialization.

`records::view` is `list` with `id = ?` and `viewRule`.

### 5.5 Records write path

`records::create/update/delete(tx, collection, record, opts)` operate on
a `Record` already validated by `validate::record(collection, record,
is_new)` (all field types, `autogeneratePattern` for empty text with a
pattern, `password` hashing, `tokenKey` autogeneration, `emailVisibility`
default false, relation existence checks, unique index violations mapped
to `validation_not_unique`, `cascadeDelete` executed inside the same
transaction, file field bookkeeping returned to the caller as
`FileOps { to_delete: Vec<key> }`). Multi-value modifiers (`field+`,
`field-`, `+field`) are resolved before validation by
`records::apply_modifiers(previous, input)`.

Nothing in `db` sends realtime events or calls hooks; it returns the
final `Record` (and previous) and the service layer does the rest.

### 5.6 Migrations ledger

`migrations::Runner { engine, dir }`: `_migrations(file TEXT PRIMARY KEY,
applied BIGINT)`. Core migrations are Rust functions registered in
order (`1_init_system.rs`, ...). User migrations are files in
`pb_migrations/` (also `cb_migrations/`) run by the JS runtime (§12).
`up()` applies unapplied in filename order inside one transaction each;
`down(n)` reverts the last n; `history_sync()` prunes ledger rows whose
file is gone. Automigrate: every collection create/update/delete through
the API writes `pb_migrations/<unix>_<created|updated|deleted>_<name>.js`
containing the collection JSON snapshot (PB's format) when
`settings.meta.hideControls` is false and the process is started with
`--automigrate` (default true, like the prebuilt PocketBase).

## 6. Filter language (`crates/filter`)

Grammar (superset of today's, matching PocketBase):

```
expr     := or
or       := and ( "||" and )*
and      := unary ( "&&" unary )*
unary    := "(" expr ")" | compare
compare  := operand OP operand
OP       := = != > >= < <= ~ !~ ?= ?!= ?> ?>= ?< ?<= ?~ ?!~
operand  := literal | ident modifiers? | call
literal  := string | number | true | false | null
ident    := [@]?[A-Za-z_][A-Za-z0-9_]* ( "." segment )*    segment may be "collection_via_field"
modifiers:= ":isset" | ":length" | ":each" | ":lower"
call     := "geoDistance" "(" operand "," operand "," operand "," operand ")"
```

Identifiers resolve through an extended `Resolver`:

| Ident | Resolves to |
|---|---|
| `field`, `relation.field` (any depth), `coll_via_field.x` | column / correlated subquery / EXISTS join |
| `@request.auth.*`, `@request.auth` | bound value (record fields, `id`, `collectionId`, `collectionName`, `isSuperuser` is not exposed; superusers short-circuit) |
| `@request.body.*` (alias `@request.data.*` accepted with a deprecation log), `@request.query.*`, `@request.headers.*` (lowercased, `-`→`_`), `@request.method`, `@request.context` | bound value |
| `@collection.name.field` | join against another collection (compiled as `EXISTS (SELECT 1 FROM name WHERE ...)`, with the whole compare inside) |
| `@now`, `@second`, `@minute`, `@hour`, `@weekday`, `@day`, `@month`, `@year`, `@yesterday`, `@tomorrow`, `@todayStart`, `@todayEnd`, `@monthStart`, `@monthEnd`, `@yearStart`, `@yearEnd` | bound value computed at compile time in UTC, PB formats |
| `x:isset` | true if key present in `@request.body` (create/update) |
| `x:length` | `json_array_length(x)` / `jsonb_array_length` |
| `x:each` | element iteration (forces the EXISTS form even for bare ops) |
| `x:lower` | `LOWER(x)` |

`~` matches PB: if the literal contains `%` it is used as-is, otherwise
wrapped in `%…%`; `_` is never escaped (PB doesn't).

`parse(src) -> Arc<Expr>` goes through a bounded LRU (1024 entries) so
rules are parsed once per process. Compilation to SQL still happens per
request (bound values differ) but is a linear walk.

`Resolver` gains `fn collection(&self, name) -> Option<Arc<Collection>>`
so `@collection.X` and back-relations resolve without async prefetch:
the resolver holds the process-wide `CollectionIndex`.

## 7. Service layer, hooks and events (`crates/server`)

### 7.1 `App`

```rust
pub struct App { inner: Arc<AppInner> }
impl App {
    pub fn new(config: Config) -> App;                       // no I/O
    pub async fn bootstrap(&self) -> Result<()>;            // db, settings, cache, migrations
    pub async fn serve(&self) -> Result<()>;                // bootstrap + listen
    pub fn db(&self) -> &Db;  settings(); storage(); mailer(); logger(); store(); cron();
    pub fn on_bootstrap() / on_serve() / on_terminate() -> &Hook<...>;
    pub fn on_record_enrich(tags) / on_record_validate(tags)
      / on_record_create(tags) / _create_execute / _after_create_success / _after_create_error
      / same for update & delete
      / on_record_create_request / update_request / delete_request / list_request / view_request
      / on_collection_create/update/delete (+request variants)
      / on_record_auth_request / on_record_auth_with_password_request / oauth2 / otp / refresh
      / on_mailer_send / on_mailer_record_verification_send / ... 
      / on_realtime_connect_request / subscribe_request / message_send
      / on_file_download_request / on_file_token_request
      / on_backup_create / on_backup_restore
      / on_settings_list_request / update_request
      / on_batch_request
    pub fn router(&self) -> &mut RouterBuilder;              // available inside on_serve
    pub async fn run_in_transaction(&self, f) -> Result<T>; // scoped TxApp
    pub fn find_collection_by_name_or_id(); find_record_by_id(); find_records_by_filter(); save(); delete(); ...
}
```

`Hook<E>`: ordered handlers `Handler { id: Option<String>, priority: i32,
func: Arc<dyn Fn(&mut E) -> BoxFuture<Result<()>>> }`, `bind`, `bind_func`,
`unbind(id)`, `trigger(e, finalizer)`. Every event type implements
`Event { fn next(&mut self) -> BoxFuture<Result<()>> }` which invokes
the remainder of the chain and finally the framework's own action. A
handler that does not call `e.next()` stops the chain, as in PocketBase.
Tagged hooks (`on_record_create("posts")`) filter by collection name or
id.

Record events carry `app: TxApp` (the transactional app inside a write)
so handler DB calls join the transaction. `RequestEvent` carries the
axum request parts, resolved auth, `request_info` (`body`, `query`,
`headers`, `method`, `context`) and helpers `json()`, `string()`,
`bad_request()`, `not_found()`, ...

### 7.2 Services

One place per concern, called by routes, batch, JSVM, and Rust plugins:

- `RecordService`: `list/view/create/update/delete` with the full
  PB event sequence (`*Request` → validate → `*Execute` → `AfterSuccess`
  / `AfterError` → enrich), file upload staging/commit/rollback, realtime
  publish after commit, `manageRule`/auth field rules (`email`,
  `password`, `verified`, `emailVisibility` only by superuser, manage rule,
  or the record itself with `oldPassword`).
- `CollectionService`: create/update/delete/import/truncate/scaffolds,
  reserved names (`_` prefix only for system), rule validation, DDL via
  `db::schema`, cache refresh, automigrate file write.
- `AuthService`: password / OAuth2 (PKCE, providers table:
  google, github, gitlab, discord, microsoft, apple, facebook, twitter,
  spotify, kakao, twitch, strava, gitee, livechat, gitea, oidc, oidc2,
  oidc3, patreon, mailcow, bitbucket, planningcenter, notion, monday,
  vk, yandex, instagram, linear, wakatime, trakt) / OTP / MFA / refresh /
  impersonate / verification / password reset / email change /
  external auths / auth origins + auth alert emails.
- `FileService`: upload validation, key layout `{collectionId}/{recordId}/{name}`,
  thumbs (`WxH`, `WxHt/b/f/l/r`, `0xH`, `Wx0`) restricted to the field's
  `thumbs` list plus `100x100` default? (PB: any size allowed only if in
  `thumbs`; the dashboard requests `100x100` which PB adds implicitly for
  images). Protected files via file token. `?download=1`.
- `RealtimeService`: PB wire protocol; topics with `?options=` (JSON
  with `query`, `headers`, and PB applies `filter/expand/fields` from
  `query`); event name = topic; `PB_CONNECT` on connect; per-subscriber
  `listRule`/`viewRule`/filter evaluation using an in-process evaluator
  for the cached AST against the record snapshot (no SQL round trip per
  subscriber); publish is spawned after commit; connection cap 30 min
  with a reconnect hint like PB (`settings`-tunable).
- `SettingsService`: get/patch/test S3/test email/apple secret.
- `LogService`: writes `_logs` rows (`type: request` with PB's data keys)
  through the Phase 1 batching writer; `list/view/stats`; retention cron.
- `BackupService`: zip of the data dir (both db files + `storage/`),
  local or S3 (`settings.backups.s3`), create/upload/download (file
  token)/delete/restore (extract to a temp dir, swap, then `exec` a
  restart of the current binary, PB style); cron per settings.
- `CronService`: `add(id, expr, fn)`, `remove`, `list`, `run(id)`; system
  jobs `__pbLogsCleanup__`, `__pbOTPCleanup__`, `__pbMFACleanup__`,
  `__pbDBOptimize__` keep PB's ids so the dashboard/API match.
- `BatchService`: PB's `/api/batch` contract (array response, `PUT`
  upsert, multipart with `requests.N.body` / `@jsonPayload`), gated by
  `settings.batch`.
- `RateLimiter`: `settings.rateLimits` rules (`label` = `/path` prefix
  or `tag` like `*:auth`, `audience` guest/auth/all, `maxRequests`,
  `duration`) with `trustedProxy` header handling. Replaces the
  hard-coded governor.

### 7.3 Routes

`routes/` mirrors PB's `apis` package: `health`, `collections`,
`records` (incl. `records_auth`), `files`, `realtime`, `batch`,
`settings`, `logs`, `backups`, `crons`, plus `/_/` for the dashboard and
`/api/plugins/...` for plugin routes, both inside the logging and
rate-limit layers. Plugin routes registered via `on_serve` land wherever
the plugin says.

### 7.4 Plugins

A plugin is `fn(&App) -> Result<()>`. `PluginRegistry` keeps the three
existing built-ins (rewritten against the new API: `cron_jobs` becomes a
thin wrapper over `CronService` with a `_cron_jobs` collection, `queue`
gets an atomic claim via `UPDATE ... WHERE status='pending'`,
`feature_flags` filters in SQL) and adds `webhooks` (a `_webhooks`
collection: url, events, collection, secret; delivered through the
queue with HMAC signatures). `App::store()` is a typed
`TypeId → Arc<dyn Any>` map.

## 8. Auth model

- `_superusers` is an auth collection with `system: true`; `RequireSuperuser`
  checks `record.collection.name == "_superusers"`. Superuser IP allowlist
  (`settings.superuserIPs`) enforced on superuser-token requests.
- `tokenKey` regenerated on password change and on
  `POST .../auth-refresh` when `?tokenKey=` hmm: PB regenerates only on
  password change / email change; keep that.
- Token kinds: `auth`, `file`, `verification`, `passwordReset`,
  `emailChange`, `mfa`, `otp` with per-collection durations.
- MFA flow: `auth-with-password` succeeds → if `mfa.enabled` and rule
  matches and no `mfaId` in body → create `_mfas` row, return
  `401 {"mfaId": "..."}`. Second call with `mfaId` and a different method
  (OTP/OAuth2) completes.
- OTP: `request-otp {email}` → `{otpId}` (always 200, fake id for
  unknown emails); `auth-with-otp {otpId, password}`.
- `impersonate/{id}` (superuser) → non-refreshable token with custom
  `duration`.
- `_authOrigins` fingerprint = sha256(IP-less UA + collection + record);
  new origin → `authAlert` email when enabled.

## 9. Files

Storage key layout and filename rules unchanged. Thumbs cached under
`{collectionId}/{recordId}/thumbs_{filename}/{size}_{filename}`. Orphaned
files deleted after commit; failed writes roll back staged uploads.
`fileToken.duration` per collection.

## 10. Logs

`_logs` in `auxiliary.db`, shape per fixture. Request rows: `level 0`,
`message "GET /api/..."`, `data {type, method, url, status, execTime,
auth, userIP, remoteIP, referer, userAgent, error?, details?}`.
`settings.logs.{maxDays, minLevel, logIP, logAuthId, maxDataSize}`.
`GET /api/logs?filter=&sort=&page=&perPage=` uses the filter grammar
against a virtual schema (`level, message, data.*, created`).
`GET /api/logs/stats?filter=` returns `[{date, total}]` hourly buckets.

## 11. CLI

```
cratebase serve [--http=host:port] [--https=...] [--dir=pb_data] [--publicDir]
                [--origins=*] [--dev] [--automigrate=true] [--indexFallback]
cratebase superuser create|update|upsert|delete|otp <email> [password]
cratebase superuser ips  (manage settings.superuserIPs)
cratebase migrate up|down [n]|create <name>|collections|history-sync
cratebase admin ...       (alias of superuser, PB compat)
```

`--dir` defaults to `./pb_data` for drop-in parity; `CRATEBASE_DATA_DIR`
and `DATABASE_URL` still override. Env vars keep working.

## 12. JavaScript runtime (`crates/jsvm`)

- Engine: `rquickjs` (QuickJS-ng), one runtime per worker thread, pool
  size = cores/2 min 2. Hooks are dispatched to a worker by message
  passing; the worker owns its `Context`. Each `*.pb.js` in `pb_hooks/`
  is loaded on every worker at boot (and reloaded on change in `--dev`).
- Globals implemented in this phase (PocketBase names):
  `$app` (findCollectionByNameOrId, findRecordById, findRecordsByFilter,
  findFirstRecordByFilter, findAllRecords, save, delete, settings,
  runInTransaction, newMailClient, logger, store, cron), `routerAdd`,
  `routerUse`, `cronAdd`, `cronRemove`, every `on*` hook function with
  tag filters, `$http.send`, `$os.getenv/readFile/writeFile`, `$security`
  (randomString, sha256, hs256, parseJWT, encrypt/decrypt), `$tokens`,
  `$mails` (sendRecordVerification, ...), `$filesystem` (fileFromPath,
  fileFromBytes, fileFromURL), `$apis` (requireAuth, requireSuperuserAuth,
  enrichRecord), `$dbx` (exp, hashExp, params), `require` (CommonJS,
  relative files under `pb_hooks`, JSON), `console`, `sleep`, `migrate`.
- Event objects mirror PB: `e.record`, `e.collection`, `e.auth`,
  `e.requestInfo()`, `e.json(status, body)`, `e.string()`, `e.next()`,
  `e.app` (transactional app inside record hooks).
- Errors thrown in JS map to `ApiError` (BadRequestError etc. constructors
  exposed).
- `pb_migrations/*.js` run through the same runtime with `migrate(up,
  down)` and `app` argument helpers.
- Not in this phase: `$template` HTML rendering, `$geo`, `$files` serve
  helpers beyond download, `$app.newBackupsFilesystem`.

## 13. Single binary

`web/admin/dist` is **committed** (PocketBase commits `ui/dist`). A
`build.rs` in `server` embeds it with `rust-embed` and fails the build
with a clear message if the directory is missing. CI rebuilds the
dashboard and fails if the committed `dist` is stale. Docker compose is
unchanged apart from the data dir default.

## 14. Testing

- Rust: unit tests per crate; `db` integration tests on SQLite and
  Postgres (`TEST_POSTGRES_URL`); `server` API tests ported from the
  current `tests/api.rs` to the new shapes.
- **Conformance**: `tests/conformance/` is a Bun project using the
  official `pocketbase` npm SDK against a spawned `cratebase serve` and,
  when `PB_BIN` is set, the same suite against PocketBase to prove the
  suite itself is right. Covers collections CRUD/import, records CRUD,
  filter/sort/expand/fields/skipTotal, auth flows (password, OTP, MFA,
  refresh, impersonate, verification, reset, email change), files +
  thumbs + protected, realtime subscribe/events with options, batch,
  settings, logs, backups list/create, crons, health. CI runs it.
- JSVM: fixture `pb_hooks` directory with hooks exercising each global;
  asserted through HTTP.
- Benchmarks: `benchmarks/run.sh` in CI on a schedule.

### 14b. The benchmark gate (release blocker)

Being faster than PocketBase is the reason to switch, so this is a gate,
not a metric. `benchmarks/run.sh` must show **no cell below 1.0x** and
the two categories Phase 1 still lost must flip:

| Cell | Phase 1 | Cause | What Phase 2 does about it |
|---|---|---|---|
| `create` c20/50/100 | 0.69-0.74x | Every pooled SQLite connection spun on `busy_timeout` for the one writer lock (the 33ms p99 is that spin) | §5.1's single dedicated writer connection: our own statements never contend, so the spin disappears |
| `search-wide` c20/50/100 | 0.53-0.75x | `sqlx::Any` decoded every column eagerly and allocated a `String` per text cell, then the record layer allocated it again | §5.1 native drivers + §5.4's single-pass row decode |

If a cell still loses after the rewrite, profile it before shipping —
the answer is a specific round trip or allocation, as it was every time
in Phase 1, not "Rust is already fast enough". Record the numbers in
`benchmarks/README.md` with the same honesty as the Phase 1 table,
including anything that got worse.

## 15. Deliberate divergences from PocketBase

1. Password hashes are Argon2id (PB: bcrypt). bcrypt hashes are still
   verified, so imported PB users can log in; they are rehashed on next
   login.
2. Postgres backend exists; `viewQuery` SQL must be valid for the
   selected backend.
3. `LOG_REQUESTS=false` exists; PB always logs.
4. `@request.data.*` is accepted as an alias for `@request.body.*` (logged
   as deprecated).
5. Request logs on Postgres deployments still go to a local SQLite
   `auxiliary.db`; a Postgres logs table is a later option.

## 15b. Known parity risks in the filter layer (must be closed or accepted)

Surfaced while implementing §6. These are **not** approved divergences —
each one is a place where a rule copied from a PocketBase app could
behave differently, so the conformance suite (§14) has to cover them and
we either match PocketBase or record the decision here.

1. **Multi-valued fields without `:each`.** PocketBase compares the raw
   JSON text of the column, so `tags = "a"` is false and `tags ~ "a"` is
   effectively "some element contains a". We always apply element
   semantics, so `tags ~ "a"` means "every element contains a". Ours is
   more principled; PocketBase's is what existing rules were written
   against. **Decision: match PocketBase** — the point of this phase is
   that a ported app behaves identically.
2. **Bare operators through a join** (`@collection.X.f = v`,
   `posts_via_author.f = v`). PocketBase adds a multi-match `NOT EXISTS`
   so a bare `=` means "all joined rows match", which is why its docs
   push `?=`. Ours resolves to "any joined row", i.e. more permissive —
   and *more permissive on an access rule is a security bug*. **Must
   match PocketBase.**
3. **`~` escaping.** PocketBase escapes `%` and `_` in the operand and
   appends `ESCAPE '\'`; we escape neither. A rule filtering on a value
   containing `_` silently matches too much. **Must match PocketBase.**
4. `@request.auth.<relation>.<field>` does not traverse into the related
   record (PocketBase joins). Accepted for now: rules needing it can use
   `@collection`.
5. Column-vs-column `=` uses null-safe equality (`IS` /
   `IS NOT DISTINCT FROM`) rather than PocketBase's
   `COALESCE(a,'') = COALESCE(b,'')`, so `''` and `NULL` compare unequal
   in that one shape. Accepted; Postgres type safety makes the
   `COALESCE` form awkward.

## 16. Work breakdown (for the implementation plan)

W0 core contracts (model, ids, dates, errors, events, settings types,
`Engine` trait, `Resolver` trait) → W1 db engines + schema + migrations
ledger ∥ W2 filter grammar → W3 records read/write/validate/expand →
W4 server (App/hooks, services, routes, auth, files, realtime, settings,
logs, backups, crons, batch, rate limits) → W5 jsvm ∥ W6 conformance +
ported tests ∥ W7 CLI + migrations runner + automigrate + plugins.
Each W is independently reviewable; W4 splits into four parallel tracks
(records+collections+files, auth, realtime+batch+settings+logs+backups+
crons, App/hooks/plugins/rate-limit).
