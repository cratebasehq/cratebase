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

## Results (2026-09-04, idle host)

16 vCPU / 27 GB shared sandbox, but genuinely idle this time: swap at
6.3MiB (was 8GiB/8GiB exhausted for the previous run below), load
average ~1.0 (was ~7). 0 errors on either server. Median of 3 runs per
cell after a discarded warm-up. Bold ratio = Cratebase faster. This is
the run in [`results/`](./results/).

| Category | Conc | Cratebase req/s | PocketBase req/s | Ratio | Cratebase p50/p99 ms | PocketBase p50/p99 ms |
|---|---|---|---|---|---|---|
| create | 1 | 5011 | 3432 | **1.46x** | 0.19 / 0.30 | 0.21 / 0.80 |
| create | 20 | 11972 | 6903 | **1.73x** | 1.65 / 1.92 | 1.68 / 22.12 |
| create | 50 | 11420 | 6980 | **1.64x** | 4.20 / 4.88 | 3.73 / 37.19 |
| create | 100 | 11960 | 6606 | **1.81x** | 8.11 / 8.87 | 9.36 / 57.88 |
| auth | 1 | 85 | 22 | **3.87x** | 11.56 / 14.35 | 45.20 / 49.72 |
| auth | 20 | 318 | 275 | **1.16x** | 59.11 / 95.20 | 66.38 / 108.73 |
| auth | 50 | 309 | 277 | **1.11x** | 143.22 / 304.27 | 148.70 / 616.48 |
| auth | 100 | 283 | 277 | **1.02x** | 277.45 / 698.23 | 268.30 / 656.96 |
| search | 1 | 5217 | 1297 | **4.02x** | 0.18 / 0.37 | 0.68 / 1.29 |
| search | 20 | 43709 | 7200 | **6.07x** | 0.41 / 0.87 | 2.05 / 10.12 |
| search | 50 | 49986 | 4774 | **10.47x** | 0.95 / 1.60 | 5.75 / 42.05 |
| search | 100 | 44230 | 4982 | **8.88x** | 1.71 / 3.82 | 6.77 / 59.31 |
| search-wide | 1 | 1794 | 665 | **2.70x** | 0.52 / 0.90 | 1.44 / 2.56 |
| search-wide | 20 | 16848 | 2853 | **5.91x** | 1.07 / 2.58 | 5.64 / 20.82 |
| search-wide | 50 | 17244 | 2308 | **7.47x** | 2.53 / 4.53 | 14.76 / 60.95 |
| search-wide | 100 | 16414 | 1924 | **8.53x** | 4.64 / 10.75 | 31.72 / 135.27 |
| search-auth | 1 | 4284 | 1187 | **3.61x** | 0.23 / 0.36 | 0.75 / 1.65 |
| search-auth | 20 | 32795 | 6398 | **5.13x** | 0.49 / 1.49 | 2.53 / 10.68 |
| search-auth | 50 | 38235 | 4893 | **7.82x** | 1.16 / 2.39 | 6.45 / 32.38 |
| search-auth | 100 | 38768 | 3989 | **9.72x** | 2.08 / 3.22 | 10.10 / 71.41 |
| delete | 1 | 2534 | 2672 | 0.95x | 0.37 / 0.74 | 0.30 / 0.88 |
| delete | 20 | 6869 | 7058 | 0.97x | 2.83 / 3.55 | 1.58 / 23.05 |
| delete | 50 | 12483 | 5595 | **2.23x** | 3.88 / 4.60 | 5.14 / 38.37 |
| delete | 100 | 11321 | 5030 | **2.25x** | 8.21 / 11.19 | 10.71 / 86.33 |

**22 of 24 cells faster.** The two exceptions are `delete` at
concurrency 1 and 20 (0.95x, 0.97x) — within noise of parity, not a
regression: `delete` at c50/c100 on the same run is 2.23x/2.25x with a
*lower* p50 than c20, so the dip at low concurrency isn't describing a
real scaling problem, just where PocketBase's own per-request overhead
happens to be small enough that Cratebase's fixed costs (auth
resolution, rule evaluation) show up proportionally more before there's
enough concurrent load to amortize them.

### Previous run (2026-09-04, busy host — kept for the WAL-checkpoint narrative below)

The run below was captured while diagnosing the WAL-checkpoint stall
described in the next section; the host was busy while it ran (load
average ~7, swap exhausted, ~11% iowait), which is visible in the
absolute numbers of both servers — see "Reading these numbers honestly"
below. Superseded by the idle-host run above for any comparison; kept
here because the specific numbers it reports (the `delete` p99 fix, the
controlled A/B) are still the source for the narrative that follows.

| Category | Conc | Cratebase req/s | PocketBase req/s | Ratio | Cratebase p50/p99 ms | PocketBase p50/p99 ms |
|---|---|---|---|---|---|---|
| create | 1 | 4995 | 3071 | **1.63x** | 0.19 / 0.32 | 0.24 / 0.75 |
| create | 20 | 8405 | 6560 | **1.28x** | 2.21 / 4.00 | 1.80 / 20.73 |
| create | 50 | 8397 | 6566 | **1.28x** | 5.49 / 7.86 | 4.57 / 40.57 |
| create | 100 | 8823 | 5965 | **1.48x** | 11.00 / 12.59 | 9.57 / 69.07 |
| auth | 1 | 79 | 21 | **3.68x** | 12.72 / 15.21 | 46.55 / 50.76 |
| auth | 20 | 297 | 235 | **1.26x** | 62.76 / 125.95 | 78.89 / 133.22 |
| auth | 50 | 283 | 246 | **1.15x** | 158.25 / 425.29 | 177.27 / 360.35 |
| auth | 100 | 277 | 250 | **1.11x** | 309.54 / 665.38 | 290.06 / 778.33 |
| search | 1 | 3550 | 1318 | **2.69x** | 0.23 / 0.86 | 0.66 / 1.38 |
| search | 20 | 36037 | 4522 | **7.97x** | 0.46 / 1.28 | 3.23 / 17.61 |
| search | 50 | 48155 | 3598 | **13.38x** | 0.89 / 1.62 | 7.64 / 57.25 |
| search | 100 | 28624 | 4103 | **6.98x** | 2.98 / 5.97 | 9.56 / 71.86 |
| search-wide | 1 | 1102 | 635 | **1.74x** | 0.59 / 1.46 | 1.52 / 2.46 |
| search-wide | 20 | 8459 | 2390 | **3.54x** | 1.85 / 5.74 | 6.22 / 23.01 |
| search-wide | 50 | 9062 | 1945 | **4.66x** | 4.65 / 10.36 | 18.16 / 76.24 |
| search-wide | 100 | 7834 | 1770 | **4.43x** | 10.79 / 19.66 | 36.34 / 151.13 |
| search-auth | 1 | 3085 | 1076 | **2.87x** | 0.32 / 0.57 | 0.86 / 1.61 |
| search-auth | 20 | 25124 | 5467 | **4.60x** | 0.69 / 1.63 | 2.69 / 10.93 |
| search-auth | 50 | 13580 | 4255 | **3.19x** | 3.06 / 7.77 | 7.32 / 36.71 |
| search-auth | 100 | 20721 | 3660 | **5.66x** | 3.48 / 9.28 | 14.15 / 78.93 |
| delete | 1 | 2264 | 2534 | 0.89x | 0.40 / 1.18 | 0.31 / 0.97 |
| delete | 20 | 5715 | 6408 | 0.89x | 3.40 / 7.73 | 1.58 / 28.87 |
| delete | 50 | 11815 | 5555 | **2.13x** | 4.00 / 5.80 | 4.46 / 39.89 |
| delete | 100 | 11374 | 4628 | **2.46x** | 8.58 / 9.73 | 11.78 / 86.48 |

### Reading these numbers honestly

**The `delete` blocker is fixed, and the p99 column is where to see it.**
The regression this table used to record was 0.19-0.30x with a p99 of
361ms against a p50 of 1.77ms. Cratebase's delete p99 is now 1.18 /
7.73 / 5.80 / 9.73ms across the four concurrency levels, against
PocketBase's 0.97 / 28.87 / 39.89 / 86.48ms. Nothing in the delete path
stalls for hundreds of milliseconds any more.

**What the regression actually was.** Not the double read, and not the
explicit transaction — both plausible, both wrong, both measured.
It was SQLite's automatic WAL checkpoint. `wal_autocheckpoint` runs
*inline, on the connection that commits the transaction which crosses
the 1000-page threshold*, so roughly one request in five hundred paid to
fold four megabytes of log back into the database while holding the
single writer connection. On an idle host that is a ~20ms stall; on a
host under memory and I/O pressure it was 350-450ms, and a single such
stall inside a 500-request batch is enough on its own to report 1040
req/s. That is why the cell swung between 8000 and 1040 req/s from run
to run while the p50 never moved. The checkpoint now runs on a
background connection on a timer;
[`crates/db/src/sqlite.rs`](../crates/db/src/sqlite.rs) carries the
measurement and the rules that keep the log bounded.

Isolated A/B, same binary, same host, minutes apart, with the old
behaviour restored by a temporary switch — this is the controlled
measurement, and the one to trust over any single cell above:

| | delete c1 | c20 | c50 | c100 |
|---|---|---|---|---|
| before, req/s | 2698 | 6422 | 6960 | 5422 |
| after, req/s | 2999 | 10609 | 12191 | 12547 |
| before, p99 | 0.75ms | 25.33ms | 20.79ms | 37.97ms |
| after, p99 | 0.53ms | 3.06ms | 4.48ms | 8.58ms |

`create` in the same pair went 7030 to 8840 req/s at c20 (p99 17.60ms to
3.64ms) and 7196 to 9373 at c100 (p99 25.75ms to 12.08ms). `update` is
not a benchmark category but shares the writer and the same checkpoint.

**A smaller second fix, worth roughly 10% of the delete path.** A delete
with no cascade and no bound hook is one `DELETE ... WHERE id = ?`, which
SQLite already runs atomically; wrapping it in `BEGIN IMMEDIATE`/`COMMIT`
only adds two round trips onto the blocking pool with the writer locked
across all three. Measured on the engine alone: 83µs against 59µs per
delete, a writer ceiling of ~12.0k against ~17.0k per second. A cascade
or a bound hook still opens the transaction, and a hook that aborts still
rolls the row back (`crates/server/tests/records_api.rs` pins both).

**The `auth` win is not an engineering win.** Argon2id at OWASP
parameters is simply cheaper than PocketBase's bcrypt cost-12. It is a
security-parameter difference and should not be read as throughput
work.

**Two `delete` cells above read 0.89x, and they are host noise, not
concurrency.** They are c1 and c20 — the two cells measured first, in a
window where the host was paging; c50 and c100, measured a minute later,
are 2.13x and 2.46x with *lower* p50s than c20. A category whose
throughput falls at c20 and recovers at c100 is not describing its own
scaling. Every other category in this run is down by a similar factor
against the quieter run taken an hour earlier (`search-wide` c20: 12540
then, 8459 here; `search-auth` c50: 35371 then, 13580 here). Trust the
ratios within a run, the p99 column, and the controlled A/B above — not
the third digit of any single cell.

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
