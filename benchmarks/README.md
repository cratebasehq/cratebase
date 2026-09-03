# Cratebase vs PocketBase benchmark

A real, executed HTTP-level benchmark comparing a Cratebase release build
against PocketBase v0.40.2, run back to back on the same machine on
2026-09-03. Numbers below are copied verbatim from
[`results/cratebase.json`](./results/cratebase.json) and
[`results/pocketbase.json`](./results/pocketbase.json), produced by
[`bench.ts`](./bench.ts).

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

## Results (2026-09-03)

500 records for `create`/`delete`, 200 requests for `auth`, 300 requests
for `search`. All runs completed with **0 errors** on both servers.

| Category | Concurrency | Metric | Cratebase | PocketBase |
|---|---|---|---|---|
| create | 1  | req/s   | 197.6 | 3424.8 |
| create | 1  | p50 ms  | 4.8   | 0.2 |
| create | 1  | p95 ms  | 6.0   | 0.4 |
| create | 1  | p99 ms  | 10.3  | 0.8 |
| create | 20 | req/s   | 218.2 | 6638.9 |
| create | 20 | p50 ms  | 91.5  | 1.7 |
| create | 20 | p95 ms  | 93.0  | 7.3 |
| create | 20 | p99 ms  | 100.5 | 23.5 |
| auth   | 1  | req/s   | 81.2  | 21.6 |
| auth   | 1  | p50 ms  | 12.1  | 46.0 |
| auth   | 1  | p95 ms  | 13.6  | 48.4 |
| auth   | 1  | p99 ms  | 17.1  | 52.0 |
| auth   | 20 | req/s   | 81.4  | 228.1 |
| auth   | 20 | p50 ms  | 244.8 | 82.7 |
| auth   | 20 | p95 ms  | 261.9 | 114.3 |
| auth   | 20 | p99 ms  | 263.5 | 131.0 |
| search | 1  | req/s   | 895.7 | 2261.8 |
| search | 1  | p50 ms  | 1.1   | 0.4 |
| search | 1  | p95 ms  | 1.6   | 0.7 |
| search | 1  | p99 ms  | 1.9   | 0.9 |
| search | 20 | req/s   | 924.4 | 7574.3 |
| search | 20 | p50 ms  | 21.4  | 1.7 |
| search | 20 | p95 ms  | 23.8  | 9.7 |
| search | 20 | p99 ms  | 24.8  | 16.3 |
| delete | 1  | req/s   | 204.3 | 2479.4 |
| delete | 1  | p50 ms  | 4.8   | 0.3 |
| delete | 1  | p95 ms  | 5.2   | 0.6 |
| delete | 1  | p99 ms  | 6.6   | 0.9 |
| delete | 20 | req/s   | 214.8 | 4654.7 |
| delete | 20 | p50 ms  | 92.8  | 2.0 |
| delete | 20 | p95 ms  | 95.8  | 24.9 |
| delete | 20 | p99 ms  | 102.6 | 28.4 |

Full raw numbers (including min/max/mean and per-batch wall-clock time)
are in [`results/cratebase.json`](./results/cratebase.json) and
[`results/pocketbase.json`](./results/pocketbase.json).

### Reading these numbers honestly

- **create/search/delete: PocketBase is substantially faster**, roughly
  10-30x higher throughput at both concurrency levels, and correspondingly
  lower latency. This traces to a concrete, verifiable architectural
  difference, not noise: Cratebase's SQLite connection pool is capped at
  `max_connections(1)` (`crates/db/src/pool.rs`), so every request that
  touches the database — reads included — serializes on that single
  connection. Throughput barely moves between concurrency 1 and 20
  (e.g. create: 197.6 → 218.2 req/s) because there is effectively no
  parallelism to exploit; the extra in-flight requests just queue, which
  is why p50 latency roughly scales with concurrency (4.8ms → 91.5ms is
  close to `20 × 4.8ms`). PocketBase's request handling parallelizes far
  more effectively across concurrency 20 (throughput roughly doubles or
  more instead of staying flat).
- **auth: Cratebase is faster at concurrency 1** (81.2 vs 21.6 req/s;
  12.1ms vs 46.0ms p50) but **PocketBase pulls ahead at concurrency 20**
  (228.1 vs 81.4 req/s) for the same single-connection-serialization
  reason above — Cratebase's admin auth path still funnels through that
  one SQLite connection to look up the admin record, so raising
  concurrency mostly adds queueing delay (p50 12.1ms → 244.8ms) rather
  than raising throughput. PocketBase's per-login cost is higher at low
  concurrency (its password hashing is more expensive per call) but it
  scales with concurrency where Cratebase does not.
- This is exactly the kind of result that is expected to flip with a
  larger connection pool on Cratebase's side; this benchmark ran the
  default configuration of both products (see below), not a tuned one.

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
- **Default configuration on both.** Neither server received any
  non-default performance tuning: Cratebase ran with `cargo build
  --release` defaults and its documented default SQLite pool size
  (`max_connections(1)`, see above); PocketBase ran `pocketbase serve`
  with no flags beyond `--http`/`--dir`. Rate limiting was disabled on
  Cratebase's auth endpoint (`AUTH_RATE_LIMIT_ENABLED=false`) purely so
  the benchmark's 200 rapid logins wouldn't get artificially throttled —
  PocketBase's benchmark run received no equivalent adjustment because it
  has no default login rate limit to disable.
  This means the `create`/`search`/`delete` gap in particular is very
  likely to shrink significantly if Cratebase's pool size were raised —
  that wasn't tested here, since the task was to benchmark default
  configuration, not to re-tune Cratebase mid-benchmark.
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
  itself were the limiting factor, PocketBase couldn't have reached 6-7x
  higher throughput in the same run.
