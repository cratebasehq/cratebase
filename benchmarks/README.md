# Cratebase vs PocketBase benchmark

An executed, reproducible HTTP-level benchmark comparing a Cratebase release
build against PocketBase v0.40.2, run back to back on the same machine by
one script. `benchmarks/run.sh` does the whole thing; this file records the
methodology and the latest numbers.

Read the caveats before drawing conclusions. This is shared, virtualized
hardware, not a benchmark rig.

## Methodology

`bench.ts` is a standalone Bun script that speaks plain `fetch`, so both
servers are driven identically (no SDKs). `run.sh`:

1. Builds a release Cratebase binary and downloads PocketBase if missing.
2. Gives each server its own port and a fresh, empty SQLite database in its
   own temp directory.
3. Creates the same superuser on each via its own CLI.
4. Creates an identical `posts` collection on each via its HTTP API:
   `title` (text, required), `content` (text), `published` (bool), with
   list/view/create public and update/delete superuser-only.
5. Runs `bench.ts` against Cratebase, then against PocketBase, and prints
   the comparison table (`compare.ts`).

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

Both servers ran their default configuration. Cratebase ran with
`AUTH_RATE_LIMIT_ENABLED=false` so the 200 rapid logins are not throttled;
PocketBase has no default login rate limit. Request logging stayed **on**
for both (Cratebase writes its log to a separate SQLite file, PocketBase to
`auxiliary.db`).

### Reproduce it

```bash
benchmarks/run.sh                       # builds, downloads PocketBase, runs everything
benchmarks/run.sh --skip-build          # reuse target/release/cratebase
benchmarks/run.sh --concurrency=1,20    # any bench.ts flag is forwarded
```

Raw results: [`results/cratebase.json`](./results/cratebase.json),
[`results/pocketbase.json`](./results/pocketbase.json).

## Results (2026-09-03, after the Phase 1 performance pass)

16 vCPU / 27 GB shared sandbox. 0 errors on either server. Bold ratio =
Cratebase faster.

| Category | Conc | Cratebase req/s | PocketBase req/s | Ratio | Cratebase p50/p99 ms | PocketBase p50/p99 ms |
|---|---|---|---|---|---|---|
| create | 1 | 5627 | 3459 | **1.63x** | 0.11 / 0.37 | 0.21 / 0.76 |
| create | 20 | 4782 | 6463 | 0.74x | 0.34 / 33.79 | 1.84 / 23.76 |
| create | 50 | 4591 | 6631 | 0.69x | 2.08 / 33.80 | 3.89 / 38.63 |
| create | 100 | 4736 | 6361 | 0.74x | 6.10 / 39.69 | 9.70 / 56.60 |
| auth | 1 | 88 | 22 | **4.04x** | 11.26 / 14.68 | 45.71 / 51.29 |
| auth | 20 | 315 | 261 | **1.21x** | 57.78 / 117.13 | 69.42 / 117.32 |
| auth | 50 | 303 | 268 | **1.13x** | 147.50 / 346.96 | 149.53 / 424.16 |
| auth | 100 | 290 | 263 | **1.10x** | 297.92 / 516.47 | 193.25 / 740.62 |
| search | 1 | 3803 | 1318 | **2.88x** | 0.25 / 0.42 | 0.66 / 1.41 |
| search | 20 | 9874 | 6711 | **1.47x** | 1.92 / 3.49 | 2.17 / 9.35 |
| search | 50 | 9842 | 4604 | **2.14x** | 4.82 / 7.33 | 4.59 / 39.23 |
| search | 100 | 9662 | 4530 | **2.13x** | 9.17 / 12.86 | 7.37 / 65.67 |
| search-wide | 1 | 842 | 643 | **1.31x** | 1.13 / 1.93 | 1.51 / 2.47 |
| search-wide | 20 | 1488 | 2793 | 0.53x | 12.95 / 20.70 | 6.10 / 21.01 |
| search-wide | 50 | 1467 | 2203 | 0.67x | 33.12 / 39.74 | 11.73 / 77.28 |
| search-wide | 100 | 1461 | 1940 | 0.75x | 66.93 / 73.24 | 32.87 / 142.99 |
| search-auth | 1 | 3495 | 1175 | **2.97x** | 0.27 / 0.47 | 0.76 / 1.49 |
| search-auth | 20 | 9419 | 5933 | **1.59x** | 2.00 / 3.16 | 2.54 / 10.87 |
| search-auth | 50 | 9359 | 5192 | **1.80x** | 5.10 / 7.06 | 5.94 / 31.82 |
| search-auth | 100 | 9162 | 3683 | **2.49x** | 10.23 / 12.76 | 12.84 / 77.96 |
| delete | 1 | 3130 | 2678 | **1.17x** | 0.23 / 0.56 | 0.30 / 1.14 |
| delete | 20 | 7237 | 6483 | **1.12x** | 0.61 / 33.91 | 1.70 / 25.15 |
| delete | 50 | 8204 | 5913 | **1.39x** | 3.36 / 21.00 | 4.51 / 36.33 |
| delete | 100 | 7285 | 4783 | **1.52x** | 6.63 / 16.14 | 11.11 / 89.28 |

### Reading these numbers honestly

- **Reads are decisively faster**: 1.5x to 3x on `search` and
  `search-auth` at every concurrency level, with a much tighter tail
  (p99 12.9ms vs 65.7ms at c100).
- **Single-writer throughput is faster** (`create` c1 1.63x, `delete`
  everywhere), and **auth is faster** at every level. The c1 auth gap is
  mostly a hashing-cost difference (Argon2id at OWASP parameters, ~11ms,
  vs PocketBase's bcrypt cost 12, ~45ms), not an engineering win; the
  c20+ auth advantage is real and comes from running the hash on the
  blocking pool instead of the async workers.
- **Two cells still lose, both for the same structural reason.**
  `create` at c20+ (0.7x) and `search-wide` at c20+ (0.5-0.75x). Under
  write contention every pooled SQLite connection spins on
  `busy_timeout` for the single writer lock (the 33ms p99 is that
  spin); PocketBase serializes writes through one dedicated connection
  instead. `search-wide` is bound by per-row decoding cost, which today
  goes through `sqlx::Any` (an eager decode of every column plus a second
  copy into JSON). Both are addressed by the Phase 2 storage engine
  rewrite: a single-writer pool and native drivers. See
  `docs/superpowers/specs/`.

### What changed in Phase 1

Before this pass Cratebase was 0.23-0.30x of PocketBase on `search` and
0.74x on `create` at c20 (see git history of this file for the old table).
The earlier roadmap blamed the `SELECT COUNT(*)`; it was not the problem.
What was:

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
  other processes running. Relative comparisons within one run are more
  trustworthy than absolute numbers.
- **Median of 3, not a distribution.** Enough to filter one-off hiccups,
  not a confidence interval.
- **SQLite on both.** Cratebase's Postgres backend is not measured here.
- **Same superuser for every login**, so per-user cache effects are not
  exercised.
- **Client-side bottleneck**: both servers are driven by the same Bun
  process at the same concurrency, so a client ceiling would cap both
  equally. At c20+ on `search` PocketBase reached ~6.7k req/s while
  Cratebase reached ~9.9k in the same session, so the client was not the
  limit at least up to that point.
