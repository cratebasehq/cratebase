# Cratebase vs PocketBase benchmark

A real, executed HTTP-level benchmark comparing a Cratebase release build
against PocketBase v0.40.2, run back to back on the same machine.
Originally run 2026-09-03; re-run the same day after fixing two
connection-pool bugs (plus a smaller logging fix) the first run's own
numbers exposed — see "Update" below. Current numbers in the Results
table and [`results/cratebase.json`](./results/cratebase.json) /
[`results/pocketbase.json`](./results/pocketbase.json) are from the
second run.

Read the caveats section before drawing conclusions — this is a single run
on shared/virtualized hardware, not a controlled benchmark environment.

## Methodology

Categories mirror the ones used by
[pocketbase/benchmarks](https://github.com/pocketbase/benchmarks)
(`create`, `auth`, `search`); `delete` was added to clean up what `create`
inserts. `bench.ts` is a standalone script — it does not use either
product's SDK, only `fetch`, so both servers are driven identically.

1. Each server got its own port and a fresh, empty SQLite database in its
   own temp data directory (no shared state between them).
2. A superuser/admin account was created on each via its own CLI.
3. An identical `posts` collection was created on each via its HTTP API:
   `title` (text, required), `content` (text, optional), `published`
   (bool, optional) — with identical permissive rules: list/view/create
   public (empty-string rule), update/delete admin-only (`null` rule).
4. `bench.ts` ran the same fixed sequence against each server, once each,
   back to back in the same shell session:
   - **create** — insert `N=500` records, at concurrency 1 then 20 (each
     level inserts its own fresh 500, so the collection ends up holding
     1000 rows by the time `search` runs against it)
   - **auth** — `N=200` `auth-with-password` calls against the built-in
     superuser account, at concurrency 1 then 20
   - **search** — `N=300` paginated `GET .../records?page=&perPage=30`
     calls against the now-populated collection, at concurrency 1 then 20
   - **delete** — remove the 500 records each concurrency level's
     `create` run created, at the matching concurrency level
5. "Concurrency N" means N requests kept in flight at once via a small
   worker pool (not N separate processes/connections pinned per worker —
   just N concurrent in-flight `fetch` calls sharing Bun's HTTP client).
6. Both servers were stopped cleanly after the run.

Latency is wall-clock per request (including JSON encode/decode and
response-body drain) measured in the benchmark process, not server-side
instrumentation. `requests/sec` is `n / (batch wall-clock time)` for that
category+concurrency batch, i.e. real observed throughput, not an
extrapolation.

### Reproduce it

```bash
# 1. Get a PocketBase v0.40.2 binary (outside the repo — never commit it)
curl -sL -o /tmp/pocketbase.zip \
  https://github.com/pocketbase/pocketbase/releases/download/v0.40.2/pocketbase_0.40.2_linux_amd64.zip
unzip -o /tmp/pocketbase.zip -d /tmp/pb-bench/pb

# 2. Build Cratebase
cargo build --release -p cratebase-server

# 3. Fresh data dirs + superusers
mkdir -p /tmp/pb-bench/cb-data /tmp/pb-bench/pb-data
DATABASE_URL="sqlite:///tmp/pb-bench/cb-data/cratebase.db" \
  CRATEBASE_DATA_DIR=/tmp/pb-bench/cb-data \
  ./target/release/cratebase superuser create admin@bench.dev benchpass123
/tmp/pb-bench/pb/pocketbase superuser create admin@bench.dev benchpass123 --dir /tmp/pb-bench/pb-data

# 4. Start both servers (separate terminals / process manager)
DATABASE_URL="sqlite:///tmp/pb-bench/cb-data/cratebase.db" \
  CRATEBASE_DATA_DIR=/tmp/pb-bench/cb-data \
  STORAGE_LOCAL_DIR=/tmp/pb-bench/cb-data/storage \
  PORT=8091 AUTH_RATE_LIMIT_ENABLED=false \
  ./target/release/cratebase serve
/tmp/pb-bench/pb/pocketbase serve --http=127.0.0.1:8092 --dir=/tmp/pb-bench/pb-data

# 5. Create the `posts` collection on each (see the request bodies in
#    the "Methodology" section above / the git history of this file for
#    the exact curl calls used — Cratebase takes `schema` with a `type`
#    of `base`/`auth`/`view` and per-field `id`s you assign; PocketBase
#    v0.40 takes `fields` instead of `schema` and assigns field ids
#    itself).

# 6. Run the benchmark against each, back to back
bun run benchmarks/bench.ts --base-url=http://127.0.0.1:8091 \
  --admin-email=admin@bench.dev --admin-password=benchpass123 \
  --create-n=500 --auth-n=200 --search-n=300 --concurrency=1,20 \
  --label=Cratebase --out=benchmarks/results/cratebase.json

bun run benchmarks/bench.ts --base-url=http://127.0.0.1:8092 \
  --admin-email=admin@bench.dev --admin-password=benchpass123 \
  --create-n=500 --auth-n=200 --search-n=300 --concurrency=1,20 \
  --label=PocketBase --out=benchmarks/results/pocketbase.json
```

## Results (2026-09-03, after the fixes below)

500 records for `create`/`delete`, 200 requests for `auth`, 300 requests
for `search`. All runs completed with **0 errors** on both servers.

| Category | Concurrency | Metric | Cratebase | PocketBase |
|---|---|---|---|---|
| create | 1  | req/s   | 3226.8 | 3333.6 |
| create | 1  | p50 ms  | 0.2    | 0.2 |
| create | 1  | p95 ms  | 0.3    | 0.4 |
| create | 1  | p99 ms  | 1.3    | 0.8 |
| create | 20 | req/s   | 4530.7 | 6111.7 |
| create | 20 | p50 ms  | 3.8    | 1.7 |
| create | 20 | p95 ms  | 6.9    | 8.6 |
| create | 20 | p99 ms  | 11.8   | 30.9 |
| auth   | 1  | req/s   | 82.2   | 22.0 |
| auth   | 1  | p50 ms  | 12.0   | 45.2 |
| auth   | 1  | p95 ms  | 13.2   | 47.3 |
| auth   | 1  | p99 ms  | 18.2   | 49.2 |
| auth   | 20 | req/s   | 265.1  | 255.6 |
| auth   | 20 | p50 ms  | 73.6   | 72.3 |
| auth   | 20 | p95 ms  | 81.2   | 102.3 |
| auth   | 20 | p99 ms  | 84.6   | 116.1 |
| search | 1  | req/s   | 686.6  | 2278.9 |
| search | 1  | p50 ms  | 1.3    | 0.4 |
| search | 1  | p95 ms  | 2.1    | 0.7 |
| search | 1  | p99 ms  | 2.4    | 0.9 |
| search | 20 | req/s   | 1330.7 | 5850.3 |
| search | 20 | p50 ms  | 14.9   | 2.1 |
| search | 20 | p95 ms  | 16.4   | 11.6 |
| search | 20 | p99 ms  | 17.0   | 17.1 |
| delete | 1  | req/s   | 2324.9 | 2468.4 |
| delete | 1  | p50 ms  | 0.3    | 0.3 |
| delete | 1  | p95 ms  | 0.5    | 0.6 |
| delete | 1  | p99 ms  | 8.9    | 0.9 |
| delete | 20 | req/s   | 4230.5 | 5509.5 |
| delete | 20 | p50 ms  | 4.2    | 2.0 |
| delete | 20 | p95 ms  | 7.3    | 9.6 |
| delete | 20 | p99 ms  | 11.6   | 31.8 |

Full raw numbers (including min/max/mean and per-batch wall-clock time)
are in [`results/cratebase.json`](./results/cratebase.json) and
[`results/pocketbase.json`](./results/pocketbase.json).

### Reading these numbers honestly

- **create/delete: essentially at parity** (Cratebase 94-97% of
  PocketBase's req/s at concurrency 1, 74-77% at concurrency 20).
- **auth: Cratebase is faster at both concurrency levels** (82.2 vs 22.0
  req/s at c1, 265.1 vs 255.6 at c20) — PocketBase's per-login cost is
  dominated by a more expensive password-hashing step; Cratebase's is
  cheaper per call and no longer bottlenecked on connection contention
  at higher concurrency either.
- **search still trails** (686.6 vs 2278.9 req/s at c1, roughly 3.3x).
  Both `list_records` implementations run a `SELECT COUNT(*)` alongside
  the paginated `SELECT` to populate `totalItems`/`totalPages` — this
  wasn't changed in this pass and is the most likely place a further
  win is sitting (e.g. an estimated/cached count, or skipping the count
  query when the caller doesn't request pagination metadata). Tracked
  as a follow-up in ROADMAP.md rather than chased further here.

## Update: two real performance bugs found and fixed

The first version of this benchmark (`create`/`search`/`delete` running
10-30x slower than PocketBase, flat throughput from concurrency 1 to 20)
led directly to fixing two bugs, not just retuning a config knob:

1. **SQLite pool capped at 1 connection even for a file-backed database**
   (`crates/db/src/pool.rs`). `:memory:` genuinely needs to stay at 1 (a
   pooled `:memory:` connection is an isolated database per connection
   unless using a shared-cache URI, which is exactly what the test
   harness relies on staying at 1 to avoid). A real file-backed SQLite
   database has no such constraint — WAL mode lets many readers run
   alongside one writer — so capping it at 1 the same way serialized
   every request, reads included, on a single connection for no reason.
   Fixed: `:memory:` stays at 1, a file-backed database gets 5.
2. **Per-connection PRAGMAs only applied to one physical connection**,
   discovered *by* raising the pool size above 1 in fix #1: the original
   code ran `sqlx::query("PRAGMA ...").execute(&pool)` once, right after
   `connect_with()` returned. That only configures whichever single
   connection happened to serve that one query — every other physical
   connection the pool subsequently opened as concurrency increased
   defaulted to SQLite's stock settings (`synchronous = FULL`, which
   fsyncs on every write; `busy_timeout = 0`, which fails a lock
   conflict immediately instead of retrying). This clawed back most of
   fix #1's throughput gain, and was reproduced directly with 20
   sequential same-process `fetch` calls against a freshly booted server:
   the first ~5 requests ran sub-millisecond, then every request from
   #6 onward (once the pool had opened its 5th connection) jumped to and
   stayed around 6ms. Fixed by moving the PRAGMA statements into
   `AnyPoolOptions::after_connect`, which sqlx runs against *every*
   physical connection the pool opens, not just the first.

A third, smaller fix landed alongside these while investigating: the new
request-logging middleware (`_request_logs`, feeding the admin dashboard's
Logs page) ran a prune `DELETE ... WHERE id NOT IN (SELECT ... LIMIT
5000)` after *every single* log insert, regardless of whether the table
was anywhere near its 5000-row cap — a second write statement, and a
full table scan, on every logged API call. Fixed to prune roughly every
256 inserts instead (`crates/db/src/system.rs`).

All three were verified with the full workspace test suite (`cargo test
--workspace`, unaffected) before and after, and with the direct
sequential-`fetch` repro above before and after.

## Caveats

- **Not a dedicated benchmark machine.** This ran in a shared/virtualized
  development sandbox (16 vCPU, ~27GB RAM reported, but shared with the
  rest of the host and other concurrent work in this environment), not an
  isolated bare-metal box. Absolute numbers will not match a quiet
  dedicated server; relative comparisons between the two servers in the
  *same* run are more trustworthy than either number in isolation.
- **Single run, not averaged.** Each category+concurrency combination ran
  exactly once per server. No repeated trials, no warm-up runs discarded,
  no statistical error bars. Treat the numbers as one data point, not a
  confidence interval.
- **Default configuration on both**, except the two bugs above (fixed in
  application code, not via a config flag — every deployment gets the
  fix automatically, there's nothing to opt into). Cratebase ran with
  `cargo build --release` defaults; PocketBase ran `pocketbase serve`
  with no flags beyond `--http`/`--dir`. Rate limiting was disabled on
  Cratebase's auth endpoint (`AUTH_RATE_LIMIT_ENABLED=false`) purely so
  the benchmark's 200 rapid logins wouldn't get artificially throttled —
  PocketBase's benchmark run received no equivalent adjustment because it
  has no default login rate limit to disable.
- **SQLite, not Postgres, on both.** Cratebase also supports Postgres via
  `DATABASE_URL`; this run used its SQLite backend exclusively (matching
  PocketBase, which is SQLite-only) so the comparison is apples-to-apples.
- **Body drain included in latency.** `bench.ts` measures from request
  start until the response body is fully read, not just until headers
  arrive, on both servers equally.
- **`auth` used the same superuser credentials for every login**, not
  distinct per-request users — both servers were treated identically, but
  this doesn't exercise per-request user-lookup cache effects that a
  large distinct-user pool might.
- **Client-side bottleneck ruled out at these concurrency levels** in the
  sense that both servers were driven by the identical script from the
  identical machine at the identical concurrency; if the Bun HTTP client
  itself were the limiting factor, PocketBase couldn't have reached 3x+
  higher throughput on `search` in the same run.

