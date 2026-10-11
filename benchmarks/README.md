# Cratebase benchmark

An executed, reproducible HTTP-level benchmark of a Cratebase release
build. `benchmarks/run.sh` does the whole thing; this file records the
methodology and the latest numbers.

Read the caveats before drawing conclusions. This is shared, virtualized
hardware, not a benchmark rig.

## Methodology

`bench.ts` is a standalone Bun script that speaks plain `fetch` (no SDK).
`run.sh`:

1. Builds a release Cratebase binary (unless `--skip-build`).
2. Boots it on its own port with a fresh, empty SQLite database in a temp
   directory.
3. Creates a superuser via the CLI.
4. Creates a `posts` collection via the HTTP API: `title` (text,
   required), `content` (text), `published` (bool), with list/view/create
   public and update/delete superuser-only.
5. Runs `bench.ts` against it and writes `results/cratebase.json`.

Each **category × concurrency** cell runs a discarded 50-request warm-up and
then **3 measured runs**; the reported row is the run with the median
requests/sec, and every run is kept in the JSON output. Concurrency levels
are 1, 20, 50 and 100 in-flight requests from a small worker pool.

| Category | What it does | N per run |
|---|---|---|
| `create` | `POST .../records` with a 3-field JSON body | 500 |
| `auth` | `auth-with-password` against the superuser | 200 |
| `search` | `GET .../records?page=N&perPage=30`, anonymous | 300 |
| `search-wide` | same with `perPage=200` (weights per-row cost) | 300 |
| `search-auth` | same as `search`, with the superuser bearer token | 300 |
| `delete` | `DELETE .../records/:id` on rows created above | 500 |

Latency is wall-clock per request measured in the benchmark process,
including the body drain. `requests/sec` is `n / batch wall-clock`.

The server ran its default configuration with
`AUTH_RATE_LIMIT_ENABLED=false` so the 200 rapid logins are not throttled.
Request logging stayed **on** (Cratebase writes its log to a separate
SQLite file).

### Reproduce it

```bash
benchmarks/run.sh                       # builds, runs everything
benchmarks/run.sh --skip-build          # reuse target/release/cratebase
benchmarks/run.sh --concurrency=1,20    # any bench.ts flag is forwarded
```

Raw results: [`results/cratebase.json`](./results/cratebase.json).

## Results (2026-09-04, idle host)

16 vCPU / 27 GB shared sandbox, idle (swap at 6.3MiB, load average ~1.0).
0 errors. Median of 3 runs per cell after a discarded warm-up. This is the
run in [`results/`](./results/).

| Category | Conc | req/s | p50 / p95 / p99 ms |
|---|---|---|---|
| create | 1 | 5011 | 0.19 / 0.27 / 0.30 |
| create | 20 | 11972 | 1.65 / 1.86 / 1.92 |
| create | 50 | 11420 | 4.20 / 4.83 / 4.88 |
| create | 100 | 11960 | 8.11 / 8.79 / 8.87 |
| auth | 1 | 85 | 11.56 / 13.40 / 14.35 |
| auth | 20 | 318 | 59.11 / 84.18 / 95.20 |
| auth | 50 | 309 | 143.22 / 294.79 / 304.27 |
| auth | 100 | 283 | 277.45 / 597.68 / 698.23 |
| search | 1 | 5217 | 0.18 / 0.25 / 0.37 |
| search | 20 | 43709 | 0.41 / 0.74 / 0.87 |
| search | 50 | 49986 | 0.95 / 1.36 / 1.60 |
| search | 100 | 44230 | 1.71 / 3.81 / 3.82 |
| search-wide | 1 | 1794 | 0.52 / 0.77 / 0.90 |
| search-wide | 20 | 16848 | 1.07 / 1.83 / 2.58 |
| search-wide | 50 | 17244 | 2.53 / 4.25 / 4.53 |
| search-wide | 100 | 16414 | 4.64 / 9.14 / 10.75 |
| search-auth | 1 | 4284 | 0.23 / 0.30 / 0.36 |
| search-auth | 20 | 32795 | 0.49 / 1.30 / 1.49 |
| search-auth | 50 | 38235 | 1.16 / 1.82 / 2.39 |
| search-auth | 100 | 38768 | 2.08 / 3.20 / 3.22 |
| delete | 1 | 2534 | 0.37 / 0.54 / 0.74 |
| delete | 20 | 6869 | 2.83 / 3.48 / 3.55 |
| delete | 50 | 12483 | 3.88 / 4.54 / 4.60 |
| delete | 100 | 11321 | 8.21 / 9.99 / 11.19 |

The `auth` rows are bound by Argon2id password hashing (OWASP
parameters), not by request handling: they measure the cost of the
security parameters, not throughput work.

### The `delete` p99 and the WAL checkpoint

An earlier version of this benchmark recorded a `delete` regression with a
p99 of 361ms against a p50 of 1.77ms. It was not the double read, and not
the explicit transaction — both plausible, both measured, both wrong. It
was SQLite's automatic WAL checkpoint. `wal_autocheckpoint` runs
*inline, on the connection that commits the transaction which crosses
the 1000-page threshold*, so roughly one request in five hundred paid to
fold four megabytes of log back into the database while holding the
single writer connection. On an idle host that is a ~20ms stall; on a
host under memory and I/O pressure it was 350-450ms, and a single such
stall inside a 500-request batch is enough on its own to report 1040
req/s. The checkpoint now runs on a background connection on a timer;
[`crates/db/src/sqlite.rs`](../crates/db/src/sqlite.rs) carries the
measurement and the rules that keep the log bounded.

Isolated A/B, same binary, same host, minutes apart, with the old
behaviour restored by a temporary switch — the controlled measurement,
and the one to trust over any single cell above:

| | delete c1 | c20 | c50 | c100 |
|---|---|---|---|---|
| before, req/s | 2698 | 6422 | 6960 | 5422 |
| after, req/s | 2999 | 10609 | 12191 | 12547 |
| before, p99 | 0.75ms | 25.33ms | 20.79ms | 37.97ms |
| after, p99 | 0.53ms | 3.06ms | 4.48ms | 8.58ms |

`create` in the same pair went 7030 to 8840 req/s at c20 (p99 17.60ms to
3.64ms) and 7196 to 9373 at c100 (p99 25.75ms to 12.08ms). `update` is
not a benchmark category but shares the writer and the same checkpoint.

A smaller second fix is worth roughly 10% of the delete path: a delete
with no cascade and no bound hook is one `DELETE ... WHERE id = ?`, which
SQLite already runs atomically; wrapping it in `BEGIN IMMEDIATE`/`COMMIT`
only adds two round trips onto the blocking pool with the writer locked
across all three. Measured on the engine alone: 83µs against 59µs per
delete, a writer ceiling of ~12.0k against ~17.0k per second. A cascade
or a bound hook still opens the transaction, and a hook that aborts still
rolls the row back (`crates/server/tests/records_api.rs` pins both).

### What changed in Phase 1

Earlier builds were far slower on `search` and `create` (see git history of
this file for the old table). The roadmap blamed the `SELECT COUNT(*)`; it
was not the problem. What was:

1. **Every API request wrote a log row into the primary SQLite file**, so
   read-only endpoints took the writer lock. Logs now go to a separate
   `*.logs.db` file via a batched background writer.
2. **sqlx pinged the connection before every acquire** (`test_before_acquire`
   default). Disabled for SQLite.
3. **Pool capped at 5 connections.** Now one per core (4..16), with the
   missing PRAGMAs (`cache_size`, `temp_store`, `mmap_size`).
4. **Collection metadata was re-read and JSON-parsed on every request.**
   Now an in-memory cache invalidated on schema change.
5. **Auth was resolved twice per request** (logging middleware + handler).
   Now once, cached on the request.
6. **Argon2 ran on tokio worker threads.** Now `spawn_blocking`.
7. **Create did INSERT then SELECT.** The response is now built from the
   values just written.
8. **No index on `created`**, the default sort. Added.
9. **glibc malloc.** Now mimalloc.

## Caveats

- **Not a dedicated benchmark machine.** Shared/virtualized sandbox with
  other processes running. Trust ratios within a run and the p99 column
  over the third digit of any single cell.
- **Median of 3, not a distribution.** Enough to filter one-off hiccups,
  not a confidence interval.
- **SQLite only.** Cratebase's Postgres backend is not measured here.
- **Same superuser for every login**, so per-user cache effects are not
  exercised.
- **Client-side bottleneck**: the Bun driver is a single process, so a
  client ceiling would cap the numbers; `search` at c50 reached ~50k req/s,
  so the client was not the limit at least up to that point.
