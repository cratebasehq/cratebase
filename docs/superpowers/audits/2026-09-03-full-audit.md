# Cratebase full audit — 2026-09-03

Goal stated by the owner: a 1:1 PocketBase port that is measurably faster than
PocketBase, trivially extendable via plugins, with a stunning admin UI. This
document records where the codebase actually stands against that goal.

Codebase size: 13,415 lines of Rust across 7 crates; 14,792 lines in
`web/admin`; 722 lines in `sdk/js`. 71 Rust test functions. 35 commits.

---

## Part 1 — PocketBase parity (target: PocketBase v0.23+ API)

### 1. Collections

**Types — PARTIAL.** `base`/`auth`/`view` exist (`crates/core/src/collection.rs:10-14`), view backed by real SQL VIEW. Missing: `system` flag; **no system collections at all**. `_superusers`, `_authOrigins`, `_externalAuths`, `_mfas`, `_otps` are raw SQL tables (`crates/db/src/system.rs:55-112`) with no REST surface. Every PB SDK path touching `_superusers` 404s.

**Field types — PARTIAL.** Present: text, editor, number, bool, email, url, date, autodate, select, json, relation, file, password. **Missing `geoPoint`.** **`password` field type stores and returns plaintext** (`crates/db/src/validate.rs:116`, `crates/db/src/records.rs:57-82`) — security defect.

**Field options — large gaps.** Have: min, max, pattern, values, collectionId, multiple, mimeTypes, maxSelect, maxSize, onlyInt, onCreate, onUpdate. Missing: `cascadeDelete`, `minSelect`, `thumbs` allowlist, `protected`, `hidden`, `presentable`, `system`, `autogeneratePattern`, custom `id`/`primaryKey`, `exceptDomains`/`onlyDomains`, `convertURLs`, `cost`, date `dependent`/`maxDifference`. `editor` is a bare alias for text.

**Indexes — MISSING.** No `indexes[]` on Collection. Only implicit unique-per-field and auth identity indexes (`crates/db/src/collections.rs:335-348`).

**API rules — PARTIAL.** list/view/create/update/delete correct with tri-state semantics. **Missing `manageRule` and `authRule`.**

**Import/export/truncate/scaffolds — MISSING.**

**Response shape divergences (SDK-breaking):** `GET /api/collections` returns bare array; field list named `schema` not `fields`; `authOptions` object instead of PB's flattened `passwordAuth`/`oauth2`/`otp`/`mfa`/`authToken`; ids are 32-char UUID hex not 15-char.

### 2. Records

- **`expand` entirely absent** (only occurrence is a reserved name at `crates/core/src/field.rs:103`). No forward, nested, or back-relation expand.
- **`fields` projection and `:excerpt` missing.**
- **`skipTotal` missing**; `COUNT(*)` always runs (`crates/db/src/records.rs:213-217`).
- `sort`: no `@random`, no relation-field sort.
- Client-supplied `id` on create not supported (`records.rs:212`).
- Multipart `+`/`-` modifiers missing; `@jsonPayload` missing.
- `emailVisibility` missing — auth-record emails always exposed.
- No per-record `tokenKey` → password change does not invalidate sessions.
- Batch: transactional (good), but response shape is `{results:[...]}` not bare array; no `PUT` upsert; no multipart; hard cap 50.

### 3. Filter language

- Operators — DONE, all 16 including `?=` family. `~` always wraps in `%`, no explicit wildcard.
- **Datetime macros — all missing** (`@now`, `@todayStart`, `@monthStart`, …). Resolver at `crates/db/src/resolver.rs:57-110`.
- `@request.data.*` is the pre-0.23 name; **`@request.body.*` missing**; `@request.query/headers/method/context` missing.
- **`@collection.X.*` joins missing.**
- **Back-relations (`_via_`) missing.**
- **Modifiers `:isset`, `:length`, `:each`, `:lower` missing** (`:` is not even lexed).
- `geoDistance()` missing; no function-call grammar.

### 4. Auth

| Endpoint | Status |
|---|---|
| auth-with-password | PARTIAL — no `identityField`, `expand`, `meta` |
| auth-with-oauth2 | PARTIAL — Google+GitHub only, hardcoded; no PKCE, no `createData`, no redirect flow |
| auth-refresh | DONE |
| auth-methods | PARTIAL — no `mfa`/`otp` blocks, wrong shape |
| request-otp / auth-with-otp | PARTIAL — returns 204 not `{otpId}`; takes `{email,otp}` not `{otpId,password}` |
| MFA | DIVERGENT — custom `/mfa/confirm` instead of PB's `401 {mfaId}` + retry |
| request/confirm verification, password-reset, email-change | DONE (email-change confirm lacks password check) |
| impersonate | MISSING |
| external-auths list/unlink | MISSING |
| `/collections/_superusers/*` | MISSING — uses pre-0.23 `/api/admins/*` returning `{token, admin}` |

Also missing: auth alerts / `_authOrigins`, token key rotation, per-collection token durations, per-collection email templates, `identityFields` list, `oauth2.mappedFields`. OAuth2 auto-links by unverified email match.

### 5. Files — PARTIAL
Thumbs work (`WxH`, `f`, `t`, `b`; missing `l`, `r`), no thumb allowlist (CPU amplification vector), no `?download=1`, no `protected` per-field distinction, no orphan GC.

### 6. Realtime — PARTIAL
SSE + `PB_CONNECT` + per-subscriber rule enforcement is genuinely good. **Topic options (`?options={filter,expand,fields,headers,query}`) missing entirely.**

### 7. Settings — MISSING (entire area)
No `/api/settings`, no test S3/email. All config is boot-time env. No `rateLimits`, no trusted-proxy config (`X-Forwarded-For` trusted unconditionally at `routes/auth.rs:30-38`, rate limiter spoofable).

### 8. Logs — PARTIAL
`GET /api/logs` exists; `/logs/:id` and `/logs/stats` missing; row shape is `{method,path,status,durationMs,authId}` not PB's `{level,message,data}`; filter is a plain LIKE on path.

### 9. Backups — PARTIAL
list/create/download/delete; **no upload, no restore**, no schedule, SQLite-only, buffers entire DB in memory, does not include uploaded files.

### 10–14
Health shape differs. Crons API missing. **No hooks/event system, no JSVM.** CLI lacks `superuser update/delete/otp`, `migrate`, serve flags. **No migrations system; schema changes silently drop columns on type change** (`collections.rs:307-319`).

### 15. Error format — DONE
`{code, message, data:{field:{code,message}}}` matches PB. Validation message is always literal "validation failed".

### Top 15 gaps ranked
1. `expand` absent
2. No system collections; pre-0.23 `/api/admins/*`
3. Filter macros missing
4. `@request.body` naming + missing query/headers/method/context
5. `@collection.X` joins missing
6. `manageRule`/`authRule` missing
7. `password` field plaintext leak
8. `emailVisibility` missing
9. `cascadeDelete` missing
10. No settings API; spoofable rate limiter
11. `fields`/`:excerpt`/`skipTotal` missing
12. No hooks / JSVM
13. No migrations; silent column drops
14. Realtime topic options missing
15. Collection JSON shape not SDK-compatible

---

## Part 2 — Performance

### Benchmark (benchmarks/, PocketBase v0.40.2, same machine, n=1)

| Category | Conc | Cratebase req/s | PocketBase req/s | Ratio |
|---|---|---|---|---|
| create | 1 | 3226.8 | 3333.6 | 0.97x |
| create | 20 | 4530.7 | 6111.7 | 0.74x |
| auth | 1 | 82.2 | 22.0 | 3.7x |
| auth | 20 | 265.1 | 255.6 | 1.04x |
| search | 1 | 686.6 | 2278.9 | **0.30x** |
| search | 20 | 1330.7 | 5850.3 | **0.23x** |
| delete | 1 | 2324.9 | 2468.4 | 0.94x |
| delete | 20 | 4230.5 | 5509.5 | 0.77x |

Search c20 p50/p95/p99 = 14.9/16.4/17.0ms: a flat distribution, the signature of a saturated queue, not slow work. The auth "win" is because Argon2 default (~12ms) is cheaper than PB's bcrypt-12 (~45ms), a security-parameter difference, not engineering.

`ROADMAP.md:59-65` blames `COUNT(*)`. **That is not the main cause** (~20-40µs on 1000 rows).

### DB round trips per request (benchmark shape)

| Request | Sync statements | Pool acquires | Write txns |
|---|---|---|---|
| GET list (anon) | 3 + 1 async | 4 | 1 |
| POST create (anon) | 3 + 1 async | 4 | 2 |
| DELETE (admin) | 5 + 1 async | 6 | 2 |
| GET list (auth-record token) | 7 + 1 async | 8 | 1 |

PocketBase: list ≈ 2, create ≈ 1, delete ≈ 2, collection from in-memory cache.

### Findings ranked
1. **Every request writes a `_request_logs` row into the primary DB** (`crates/server/src/request_log.rs:44-56`, `crates/db/src/system.rs:169-198`). Turns read workload into write workload, takes SQLite's writer lock per request, burns a pool connection, and the every-256 prune is a full scan. PB uses a separate `auxiliary.db` with batched writes.
2. **`test_before_acquire` left at default `true`** (`crates/db/src/pool.rs:69-99`). Every acquire does a cross-thread `ping()` round trip for zero benefit. One-line fix.
3. **SQLite pool capped at 5, no read/write split** (`pool.rs:62-67`). 20 in-flight × 4 acquires / 5 conns = the c20 cliff. PB uses a CPU-sized read pool + single-writer pool.
4. **Argon2 on tokio worker threads, no `spawn_blocking` anywhere** (`crates/auth/src/password.rs`, called from `routes/auth.rs:127,181,449,650`, `auth_fields.rs:99,164`). A login burst starves every other endpoint.
5. **Collection metadata re-queried and JSON re-parsed on every request** (`helpers.rs:9-22`, `collections.rs:57-64`). No cache in `AppState`. `load_collection` tries name then id (a failed query first for id lookups).
6. **Auth resolved twice per request** — logging middleware and handler both run `CurrentAuth` (`extract.rs:34-90`), which is up to 2 DB queries each for auth-record tokens. No token→auth cache.
7. **sqlx `Any` driver double-copies every row**: eager decode + `String` per text cell, then `row_to_record` hash-lookup + second allocation (`records.rs:33-83`). Also blocks `SqliteConnectOptions::pragma()/read_only()/statement_cache_capacity()`.
8. **INSERT then SELECT read-back** on create/update (`records.rs:430-436, 511`), on different connections, not in a transaction.
9. **Default sort `ORDER BY created DESC` with no index on `created`** (`records.rs:92`, `collections.rs:335-347`).
10. No `#[global_allocator]`; glibc malloc on an allocation-heavy path.
11. `TraceLayer` global; cheap at info, footgun at debug.

Fine already: release profile, tokio multi-thread, no compression, CORS, body limit, rule short-circuit for empty rules, no locks in AppState, realtime read-lock snapshot.

Realtime: zero cost at zero subscribers, but **O(N) DB queries per write with N subscribers** on a non-empty rule, each re-lexing/re-parsing the rule, awaited inline in the request.

### Prioritized changes
Quick wins (hours): Q1 `test_before_acquire(false)`; Q2 batch request logs via mpsc → periodic multi-row insert, ideally into a separate aux DB, with `LOG_REQUESTS` flag; Q3 pool size = cpus clamp(4,16); Q4 add `cache_size=-16000`, `temp_store=MEMORY`, `mmap_size=256MB`, `journal_size_limit`; Q5 `spawn_blocking` around Argon2 + explicit params; Q6 skip read-back after INSERT/UPDATE; Q7 mimalloc; Q8 single `name OR id` lookup; Q9 index `created`.

Structural (days): S1 in-memory collection cache (`arc-swap`), pass `Arc<Collection>`; S2 auth resolved once per request + short-TTL token cache; S3 read pool + single-writer pool; S4 drop `sqlx::Any` for native Sqlite/Postgres behind an enum, decode straight to `Value`; S5 cache parsed rule/filter ASTs; S6 realtime publish spawned + rules grouped by (rule, auth) + in-process evaluator; S7 `skipTotal`.

Benchmark methodology fixes first: repeated runs with warm-up, concurrency 50/100/200, `perPage=200` case, authenticated search case, `LOG_REQUESTS=false` isolation.

**Bottom line:** Cratebase is slower because it does 3-7 round trips where PB does 1-2, pings before each, writes a log row into the same DB per request, caps at 5 connections, and copies rows twice through `Any`. None of it is inherent to Rust.

---

## Part 3 — Plugin / extensibility

### What exists
`Plugin` trait with `name()`, `setup(&Db)`, `routes()`, `scheduled_tasks()` (`crates/server/src/plugin.rs:62-87`). `ScheduledTask.run` is a bare `fn` pointer (`plugin.rs:51`) — cannot capture state. `PluginRegistry` with register/setup_all/router/spawn_tasks.

Plugins can: provision collections at boot, mount routes under `/api/plugins/<name>`, reach all of `AppState` in handlers, run fixed-interval jobs, publish realtime events, call the db directly (faking a superuser `AuthContext`).

### What plugins cannot do
Record lifecycle hooks (deliberately declined, `plugin.rs:59-61`), validate/mutate payloads, enrich/redact responses, auth lifecycle hooks, mailer hooks/backends (private enum), request middleware (`build_app` closed, `lib.rs:74-89`), custom field types (closed enum), CLI commands, dashboard UI (spec only, zero code), settings, migrations, server-side realtime subscription, filter macros.

### Real defects
- Plugin routes are merged as a sibling of `/api` nest (`lib.rs:81-88`) → **bypass request logging and rate limiting**.
- No duplicate/reserved plugin name guard.
- `setup()` receives `&Db` not `&AppState`.
- `cratebase-server` not publishable (path deps without versions).
- No downstream Rust example; library path untested.
- No non-Rust extension path: no JS VM, no WASM, no webhooks. `cron_jobs`/`queue` job bodies are hardcoded `match` arms with one arm each — data plane is dynamic, execution plane is not.

### Reference plugin quality
Good products, poor references: use `crate::` paths downstream can't, copy-paste `system_auth()` twice and `ensure_X_collection` three times, no `CollectionBuilder`. `feature_flags` does an O(n) scan over 500 records per check. `queue` claim is not atomic (read-then-write, no `WHERE status='pending'`), throughput ceiling 0.5 jobs/sec.

### Structural blockers
1. `AppState` closed struct, no extension store.
2. `ScheduledTask.run` is `fn` not `Box<dyn Fn>`.
3. db layer has no event emission point.
4. **Record create is spread across route + batch + db** (`routes/records.rs:192-229`, `routes/batch.rs`, `db/records.rs`). No single function to wrap. #1 blocker for hooks.
5. Routes built in one closed private function.
6. Realtime is publish-only, no broadcast channel.

### Recommended architecture
- `RecordEvent {action, collection, record, previous, auth}` in `crates/core`.
- `RecordHook` trait with `before(&mut e) -> HookOutcome{Continue|Abort}`, `after(&e)`, `serialize(&e, &mut Value)`; `collections()` filter, `priority()`.
- Single `RecordService::{create,update,delete}` seam in server, parameterized over executor so batch reuses it.
- `AppState { store: PluginStore(TypeMap), events: broadcast::Sender<RecordEvent>, hooks: HookRegistry }`.
- `App` builder enforcing `build_state → setup_all → spawn_tasks → build_app → serve`.
- Order: store + broadcast bus first (~100 lines), then RecordService refactor on its own commit with tests green, then hooks, then webhooks plugin, then JS VM as a plugin forwarding bus events.

---

## Part 4 — Admin UI (`web/admin`)

### Scale
82 files, 14,792 LOC. `components/interior/` = 29 files, **9,860 LOC (67%)**, a bespoke motion library; **14 files / 4,586 LOC never imported**; 8 of 17 shadcn primitives dead. Real product code ≈ 3,400 LOC.

### Stack
React 19.2, TanStack Router (code-based) + Query, Tailwind v4 CSS-first, shadcn `radix-nova`, lucide, Geist + Geist Mono self-hosted, motion, dnd-kit, next-themes, sonner. **No charts, no form lib, no tests, no Storybook.** `strict: true` absent from tsconfig.

### Design system (`index.css`, 153 LOC)
"Rust on ink" palette (light `#d9530f`, dark `#ef7d3e` primary), full light/dark/sidebar/chart tokens, `--radius` scale, three custom easings nobody uses. **No type scale, spacing, shadow, z-index, or motion tokens.** ~9 arbitrary font sizes (`text-[10.5px]`…`text-[15px]`). `interior/` uses zero tokens, hardcodes stone/white/`#1D1D1A`; every usage fights it with a 10-`!important` blob copy-pasted in 5 files.

### Screens (6 total)
`/login`, `/` (empty placeholder), `/collections/$name` (grid + `?tab=settings`), `/settings/logs`, `/settings/backups`, `/settings/cron`. No pending/error/notFound components anywhere.

### Login (`routes/login.tsx`)
Centered `max-w-sm` card, favicon, two fields, button, CLI hint. No background treatment, no brand. Password field is a raw `<input>` styled differently from the email field. **Enter does not submit** (`onSubmit={preventDefault}`, button is `onAction`). Errors are toasts only; every failure reads "Invalid email or password" including 500/network. No show-password, no OAuth/OTP/MFA paths despite server support. Dark forced (`enableSystem={false}`).

### App shell
Fixed `w-64` sidebar, **not responsive** (15 responsive classes in the whole app). No top bar, no breadcrumbs, no user identity/account menu, no visible ⌘K entry, three footer actions with three treatments. No dashboard home.

### Records grid
Good: dense gridline table, URL state, inline editing, rich value cells (relation popover, chips, lightbox, JSON popover), realtime new-items pill.
Bad: no virtualization, page size hardcoded 25, no row selection/bulk ops (`/api/batch` unused), no column controls, no filter expression input, sorting double-implemented, **delete has no confirmation**, no optimistic updates, no error state, empty state is a bare row, no keyboard nav.

### Record drawer
No client validation, JSON is a plain textarea, relation pickers are native `<select multiple>` from first 100 records, file input bare, no metadata, no unsaved guard, naive singularization ("New addres").

### Schema editor
Good validation and helper copy. Native `<select>` and checkboxes while shadcn equivalents sit dead; cramped row at 480px; **no unsaved-changes guard, background refetch wipes edits**; save not gated on validity; field delete instant with no column-drop warning.

### Settings
Logs: filter form has no submit affordance, no chart, no facets, no detail. Backups: delete unconfirmed, no restore/upload. Cron: delete unconfirmed, free-text job key, >25 jobs invisible.

### Data layer
SDK `fetch` wrapper, `localStorage` token, **no 401 handler** (expired token = toasts forever), no global error handler, no error boundary, no route loaders (client waterfalls), realtime only on the collection route.

### Missing vs PocketBase admin
Settings: Application, Mail, Files storage, Backups restore/upload, Export/Import collections, Auth providers, Token options. Superusers page. API preview panel. Logs chart + facets. Records filter/sort UI, column toggles, bulk ops. Auth tools (verify, reset, impersonate). Indexes editor. Duplicate collection. Dashboard home. Responsive layout.

### Keep / replace
Keep: token palette, radius scale, Geist pair, easings, cn/cva, shadcn base, lucide, sonner, next-themes, dnd-kit, react-query, router, SDK layer, `record-value-cell.tsx` behavior, inline editing, new-items pill.
Add: typography/spacing/shadow/motion/z tokens; ~18 missing shadcn primitives (Sidebar, Sheet, Dialog, Tabs, Tooltip, Command, Form, Popover, Empty, Field, Spinner, Kbd, Breadcrumb, Alert, Collapsible, ToggleGroup, Pagination, Item); a chart lib; a form lib.
Replace/delete: the entire `interior/` folder (port `hold-to-confirm`, `new-items-pill`, `lightbox`, `useSortableRows` by hand).

### In-flight uncommitted changes
`collection-form.tsx` (duplicate-name validation), `collection-settings.tsx` (protected `users` explainer), `interior/popover.tsx` (portal), `records-grid.tsx` + `routes/collection.tsx` (empty state moved into table, old better one deleted).
