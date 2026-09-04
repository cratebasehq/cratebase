# Phase 1 — Performance quick wins + honest benchmark harness

Date: 2026-09-03. Status: approved (owner delegated to recommendations).
Source: `docs/superpowers/audits/2026-09-03-full-audit.md`, Part 2.

## Goal

Close the measured gap against PocketBase on the existing benchmark with
changes that are local, low-risk, and survive the later core rewrite
(Phase 2). Establish a benchmark harness whose numbers can be trusted.

Non-goals: dropping `sqlx::Any`, read/write pool split, hooks, any API
shape change. Those are Phase 2.

## Changes

### 1. Pool (`crates/db/src/pool.rs`)
- `test_before_acquire(false)`. SQLite connections are local; the ping is a
  cross-thread round trip per acquire for nothing. Postgres keeps the
  default (a dead TCP connection is a real possibility there).
- File-backed SQLite `max_connections` = `available_parallelism()`
  clamped to 4..=16, overridable by `DATABASE_MAX_CONNECTIONS`.
  `:memory:` stays at 1 (test harness relies on it).
- Extra PRAGMAs per connection: `cache_size = -16000`, `temp_store = MEMORY`,
  `mmap_size = 268435456`, `journal_size_limit = 200000000`.

### 2. Request log off the hot path (`crates/server/src/request_log.rs`, `crates/db/src/system.rs`)
- `AppState` gets a `RequestLogWriter` holding an
  `mpsc::UnboundedSender<RequestLogEntry>`. The middleware only does
  `try_send`.
- One background task drains the channel, buffering up to 256 entries or
  500ms, then issues one multi-row `INSERT`. Prune runs once per flush
  when the running insert count crosses the 256 threshold, unchanged
  semantics.
- On SQLite the log table lives in a **separate database file**
  (`<main>.logs.db` next to the main file, same PRAGMAs) so the primary
  writer lock is never taken for logging. `Db` gains `logs: AnyPool`
  (same pool when Postgres or `:memory:`).
- `LOG_REQUESTS=false` disables logging entirely (config flag,
  default true). Benchmarks run with it on, to be honest.

### 3. Auth resolved once per request (`crates/server/src/extract.rs`)
- `CurrentAuth::from_request_parts` checks `parts.extensions` for a cached
  `CurrentAuth` first and stores its result there after resolving. The
  logging middleware runs outermost, so the handler's extractor becomes a
  map lookup. No behavior change.

### 4. In-memory collection cache (`crates/db/src/collections.rs`, `pool.rs`)
- `Db` gains `collections: Arc<RwLock<CollectionCache>>` with two maps
  (by id, by name) of `Arc<Collection>`.
- `get_collection_by_id/name` read the cache first; on miss they query and
  populate. `list_collections` still queries (ordering, rare).
- `create_collection`, `update_collection`, `delete_collection`
  invalidate/refresh the entries. `_tx` variants untouched.
- `helpers::load_collection` becomes: cache-by-name, then cache-by-id, then
  one DB query `WHERE name = $1 OR id = $1`.
- Known limitation: two server processes on one SQLite file will not see
  each other's schema changes until restart. Same as PocketBase.

### 5. Argon2 off the async runtime (`crates/auth/src/password.rs`, call sites)
- Add `hash_password_async` / `verify_password_async` wrappers using
  `tokio::task::spawn_blocking`. `cratebase-auth` gets `tokio` (`rt`).
- Params stay Argon2id m=19456 KiB, t=2, p=1 (OWASP), but spelled out
  explicitly instead of `Argon2::default()`.
- Replace the six sync call sites in `routes/auth.rs`, `auth_fields.rs`,
  `routes/otp_auth.rs` if any, with the async versions.

### 6. No read-back after INSERT (`crates/db/src/records.rs`)
- `create_record_with_id` and `create_record_with_id_tx` build the
  response `Value` from what they just wrote: `id`, `created`, `updated`,
  `collectionId`, `collectionName`, auth identity + `verified: false`, and
  each schema field passed through `ColumnValue::from_json(..).to_json(..)`
  so the JSON shape matches a DB round trip exactly.
- `update_record` keeps its read-back (partial updates need the full row);
  Phase 2 removes it when the service layer owns `previous`.

### 7. Allocator (`crates/server/src/main.rs`)
- `mimalloc` as `#[global_allocator]`.

### 8. Index on `created` (`crates/db/src/collections.rs::sync_table`)
- `CREATE INDEX IF NOT EXISTS idx_<name>_created ON <table> (created DESC)`
  for every base/auth collection, on create and on every sync.

### 9. Benchmark harness (`benchmarks/bench.ts`, `benchmarks/run.sh`)
- Warm-up: 50 requests per category discarded.
- Repeats: each category × concurrency runs 3 times; report the median
  run's req/s and latency percentiles, and record all runs in the JSON.
- Concurrency levels default to `1,20,50,100`.
- New categories: `search-wide` (`perPage=200`) and `search-auth`
  (same as search, with the admin bearer token).
- `run.sh` scripts the whole thing: build release, fresh data dirs, start
  both servers, create superusers + collection on each, run bench, stop
  servers, write `results/*.json` and print the comparison table.
- README rewritten to the new methodology with the new numbers.

## Verification
- `cargo test --workspace` stays green (77 passed baseline).
- New unit tests: collection cache invalidation; request-log batching
  flushes on count and on timer; `CurrentAuth` cached in extensions
  resolves once (counted via a test-only hook or by asserting a single
  `_admins` query is impossible without instrumentation — instead assert
  behavior parity in the existing api tests).
- Before/after benchmark numbers in `benchmarks/README.md`.

## Success criteria
Cratebase ≥ PocketBase req/s on `search` c1 and c20, and within 10% or
better on `create`/`delete` c20, on the same machine, same run, median of
3. If not met, the remaining delta is documented with a profile and
carried into Phase 2's structural items.
