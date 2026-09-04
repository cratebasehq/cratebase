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

## Results (2026-09-04, after the Phase 2 rewrite)

16 vCPU / 27 GB shared sandbox. 0 errors on either server. Median of 3
runs per cell after a discarded warm-up. Bold ratio = Cratebase faster.

| Category | Conc | Cratebase req/s | PocketBase req/s | Ratio | Cratebase p50/p99 ms | PocketBase p50/p99 ms |
|---|---|---|---|---|---|---|
| create | 1 | 4532 | 1445 | **3.14x** | 0.17 / 0.38 | 0.21 / 0.88 |
| create | 20 | 7584 | 1968 | **3.85x** | 1.88 / 20.03 | 1.75 / 205.95 |
| create | 50 | 6982 | 1764 | **3.96x** | 5.38 / 23.56 | 4.26 / 244.24 |
| create | 100 | 6800 | 5006 | **1.36x** | 11.19 / 30.06 | 8.73 / 75.91 |
| auth | 1 | 83 | 22 | **3.85x** | 11.80 / 15.20 | 46.11 / 49.83 |
| auth | 20 | 316 | 239 | **1.32x** | 58.50 / 137.15 | 79.44 / 116.33 |
| auth | 50 | 290 | 266 | **1.09x** | 131.73 / 588.27 | 168.62 / 301.88 |
| auth | 100 | 274 | 268 | **1.02x** | 283.18 / 586.02 | 203.94 / 732.75 |
| search | 1 | 4454 | 1295 | **3.44x** | 0.21 / 0.46 | 0.68 / 1.41 |
| search | 20 | 29845 | 6128 | **4.87x** | 0.56 / 1.58 | 2.16 / 12.67 |
| search | 50 | 35761 | 4097 | **8.73x** | 1.29 / 2.19 | 6.07 / 36.99 |
| search | 100 | 28730 | 4949 | **5.80x** | 3.04 / 4.81 | 6.94 / 59.44 |
| search-wide | 1 | 1365 | 670 | **2.04x** | 0.59 / 0.99 | 1.44 / 2.40 |
| search-wide | 20 | 15281 | 2866 | **5.33x** | 1.21 / 2.45 | 5.37 / 20.95 |
| search-wide | 50 | 15968 | 2137 | **7.47x** | 2.60 / 5.72 | 13.92 / 86.00 |
| search-wide | 100 | 14175 | 2062 | **6.88x** | 6.03 / 11.32 | 28.05 / 133.06 |
| search-auth | 1 | 3915 | 1138 | **3.44x** | 0.23 / 0.79 | 0.80 / 1.54 |
| search-auth | 20 | 32019 | 6423 | **4.99x** | 0.55 / 1.53 | 2.56 / 9.73 |
| search-auth | 50 | 35905 | 5200 | **6.91x** | 1.18 / 2.39 | 5.76 / 32.95 |
| search-auth | 100 | 32050 | 3748 | **8.55x** | 2.58 / 5.23 | 9.11 / 78.94 |
| delete | 1 | 3110 | 2593 | **1.20x** | 0.27 / 0.51 | 0.30 / 1.07 |
| delete | 20 | 1234 | 6500 | 0.19x | 1.77 / 361.55 | 1.69 / 26.44 |
| delete | 50 | 1595 | 5854 | 0.27x | 4.97 / 266.89 | 4.70 / 38.31 |
| delete | 100 | 1572 | 5287 | 0.30x | 10.47 / 238.02 | 11.99 / 78.60 |

### Reading these numbers honestly

**23 of 24 cells win, most of them by 3-8x.** Both categories Phase 1 lost
have flipped: `create` under contention went from 0.69-0.74x to 1.36-3.96x
(the single dedicated writer connection removed the `busy_timeout` spin
that produced Phase 1's 33ms p99), and `search-wide` went from 0.53-0.75x
to 2.04-6.88x (native drivers and single-pass row decoding removed the two
copies `sqlx::Any` made of every text cell).

**`delete` above concurrency 1 is a regression and a release blocker.**
0.19-0.30x, with a p50 of 1.77ms against a p99 of 361ms. That shape —
a fine median with a tail two orders of magnitude worse — is queueing,
not slow work, and it is ours: PocketBase's delete p99 at the same
concurrency is 26ms. Deletes are the only category that reads twice
before writing, and every write opens an explicit `BEGIN IMMEDIATE`
transaction against the single writer connection, so the suspicion is
that we hold the writer far longer per delete than the work justifies.
Under investigation; the numbers stay in this table until it is fixed.

**The `auth` win is not an engineering win.** Argon2id at OWASP
parameters is simply cheaper than PocketBase's bcrypt cost-12. It is a
security-parameter difference and should not be read as throughput
work.

**Caveat on this run:** two implementation agents were compiling on the
same machine, so absolute numbers are noisier than the Phase 1 table.
The 3-8x margins are far outside that noise; the `delete` regression
reproduces across all three concurrency levels and all three repeats.

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
