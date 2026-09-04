# Cratebase — work handoff, 2026-09-04

Written so each work package below can be handed to a separate agent with
no further context. Every number here was measured on this commit, not
recalled: re-run the commands in [§7](#7-how-to-verify-anything) to check
any of it.

---

## 1. Where we actually are

**Conformance against the official `pocketbase` JS SDK v0.28: 155 pass /
25 fail / 1 skip, of 181.**

```
collections   25/25    records  54/54    files  12/12
realtime       7/7     backups   7/7     crons   5/5
settings       7/7     logs      6/6     errors  9/9    health 3/3
auth          13/31  ← 18 failures
batch          0/7   ← 7 failures
```

The 25 failures are **two features**, not scattered bugs. Everything else
is byte-compatible with PocketBase v0.40.2.

### Performance vs PocketBase

22 of 24 benchmark cells are faster. Ratio is PocketBase-relative; lower
is better for us. Full table: `bun run benchmarks/compare.ts
benchmarks/results/pocketbase.json benchmarks/results/cratebase.json`.

| Workload | Conc | PocketBase req/s | Cratebase req/s | Speedup |
|---|---:|---:|---:|---:|
| search | 50 | 3,598 | 48,155 | **13.4x** |
| search | 20 | 4,522 | 36,037 | **8.0x** |
| search-auth | 20 | 5,467 | 25,124 | **4.6x** |
| search-wide | 50 | 1,945 | 9,062 | **4.7x** |
| auth | 1 | 21 | 79 | **3.8x** |
| create | 100 | 5,965 | 8,823 | 1.5x |
| delete | 1 | 2,534 | 2,264 | **0.9x (slower)** |
| delete | 20 | 6,408 | 5,715 | **0.9x (slower)** |

The tail is the more interesting half: `create` at 100 concurrent is
p99 **69.07ms → 12.59ms**, and `search` at 50 is p99 **57.25ms → 1.62ms**.
PocketBase's p99 blows out under contention; ours stays flat.

⚠️ **These were measured on a host that was under memory pressure**
(swap exhausted mid-run). They are directionally right and the read
numbers are reproducible, but **re-run on an idle machine before putting
any of them in a README or a launch post.** That re-run is W6 below.

---

## 2. Work packages

Independent unless stated. Each is sized for one agent.

### W1 — Batch API (7 conformance failures) · no dependencies

`POST /api/batch`. The route is not mounted; see the stub comment in
`crates/server/src/routes/mod.rs`.

Tests to make pass — `tests/conformance/batch.test.ts`:
1. create / update / upsert / delete in one request
2. a failing request rolls the whole batch back
3. batch honours API rules of the executing client
4. file uploads inside a batch (multipart `@jsonPayload`)
5. exceeding `maxRequests` → 400
6. batch disabled → 403
7. empty batch → 400

Notes:
- Settings already carry `batch: {enabled, maxRequests, timeout, maxBodySize}`.
  Check `crates/core/src/settings.rs` before adding fields.
- The whole batch is **one transaction**. `App::run_in_transaction` exists.
- Realtime events must fire **after** commit, once per record, via
  `crate::realtime::publish`. Read that module's contract comment first.
- Multipart `@jsonPayload` is PocketBase's convention for sending the
  JSON body alongside files in one request; `crates/server/src/routes/records.rs`
  already parses multipart for the single-record path.

### W2 — Auth: verification, password reset, email change (7 failures)

`tests/conformance/auth.test.ts`, describe block
"verification / password reset / email change".

Endpoints: `request-verification`, `confirm-verification`,
`request-password-reset`, `confirm-password-reset`,
`request-email-change`, `confirm-email-change`.

Notes:
- Every `request-*` **always returns 204**, even for an unknown email —
  it must not leak whether an account exists. Three tests check this.
- Tokens are JWTs signed with `secret + record.tokenKey`, so changing a
  password invalidates outstanding tokens for free. That mechanism is
  already implemented for auth tokens in `crates/auth`.
- `confirmEmailChange` takes **token + the current password**; it is the
  one flow that re-checks the caller.
- Three tests assert on the *email that was sent* through the harness's
  in-process SMTP sink. `crates/mailer` and the templates in `web/email`
  already exist.

### W3 — Auth: OTP + MFA (8 failures)

Describe blocks "auth: OTP" and "auth: MFA".

- `request-otp` returns an `otpId` for unknown emails too (same
  non-enumeration rule as W2).
- The `_otps` and `_mfas` system collections already exist, and the cron
  jobs that sweep them are already registered and visible in the
  dashboard (`__pbOTPCleanup__`, `__pbMFACleanup__`).
- MFA first factor answers **401 with `{mfaId}`**; the *same* method
  again is a 400. PocketBase does **not** echo `mfaId` back on the second
  call — `tests/conformance/auth.test.ts:629` documents this.
- Enabling MFA with fewer than two auth methods must be rejected.

### W4 — Auth: impersonate, `_authOrigins`, auth-alert mail (3 failures)

- `impersonate` returns a **non-refreshable** token; `authRefresh` echoes
  it back unchanged rather than minting a new one.
- `_authOrigins` gets a row per login and is visible **only to its
  owner**.
- An auth-alert email goes out on login from a new origin.

### W5 — Dashboard gaps (no conformance coverage; UI only)

Server-side support exists for all of these; the screens do not.
- Auth providers + token options — belongs in the auth-collection schema
  editor, **not** global settings (that's where PocketBase puts it).
- Rate limits, trusted proxy, superuser IPs.
- Backup **restore** and **upload** (`POST /api/backups/upload`,
  `POST /api/backups/{key}/restore` are both implemented and unused).
- Collection export / import.
- Request-log facets.
- `geoPoint` field editor.

`web/admin` has **zero tests** and ships as a single ~1MB chunk. Both are
worth fixing in this package.

### W6 — Re-run benchmarks on an idle machine

Close everything, confirm swap is free, then `bash benchmarks/run.sh`.
Replace `benchmarks/results/*.json` and update every number in §1.
Investigate `delete` at concurrency 1–20, the only two cells we lose.

The previously suspected cause — an inline WAL checkpoint — was already
fixed (p99 20ms → 2ms, +45% throughput); see the module doc in
`crates/db/src/sqlite.rs`. Whatever remains is something else, and it may
simply be noise from the degraded host.

### W7 — Documentation pass

`README.md`, `ARCHITECTURE.md`, `llms.txt`, `openapi.yaml` all predate the
core rewrite. Do this **after** W6 so the numbers are real.

### W8 — Finish `crates/jsvm` (JS hooks)

Compiles and is in the workspace, but is not wired into the server. It
gives Cratebase PocketBase's `pb_hooks` story. Read `crates/jsvm/src/bridge.rs`
first — the prelude implements every global on one native entry point.

---

## 3. What we offer over PocketBase

This is the part that answers "why would anyone switch". Grouped by how
defensible it is today.

### Shipping now

**1. Speed, especially under load.** 22 of 24 benchmark cells, and the
p99 numbers are the real story: PocketBase's tail degrades badly with
concurrency (create p99 69ms at 100 conc) where ours stays flat (12.6ms).
Reads are 3–13x. A read-heavy app is the clearest win.

**2. Same deal on deployment.** One static binary, no runtime, no CGO —
plus a Docker path. Nobody has to give anything up to switch.

**3. Drop-in compatibility.** The official `pocketbase` JS SDK works
unchanged; that is exactly what the conformance suite proves, and it is
why we deleted the in-house SDK. Migration is a one-line base-URL change,
not a rewrite. Existing bcrypt password hashes are verified on login and
transparently upgraded to Argon2id.

### The strategic gap: **Postgres**

PocketBase is SQLite-only, by design and permanently. That is its single
biggest ceiling, and we already have a native `tokio-postgres` + `deadpool`
backend beside the `rusqlite` one.

That unlocks, in order of value:
- **Horizontal scale** — more than one app node against one database.
- **Managed database** — RDS/Cloud SQL/Neon, with backups, PITR and
  failover someone else operates.
- **Multi-node realtime** via `LISTEN`/`NOTIFY`, so the SSE fan-out is not
  confined to a single process. Today's realtime is single-node; this is
  the natural follow-on.

This is the "grow out of PocketBase without leaving the framework" story,
and no amount of PocketBase tuning gets there.

### Planned, in agreed order

1. **Postgres multi-node realtime** (above) — makes the Postgres backend
   a real answer rather than a checkbox.
2. **MCP server + vector search + full-text search** — one endpoint that
   makes a Cratebase instance directly usable as an AI application
   backend. Nothing in the PocketBase ecosystem covers this.
3. **Schema-as-code + type generation** — collections defined in a
   checked-in file, with generated TypeScript types. Turns migrations
   into a reviewable diff instead of dashboard clicks.
4. **Plugin registry + WASM plugins** — Rust plugins already work
   in-process (`crates/server/src/plugin.rs`); WASM makes them
   distributable and sandboxed.
5. **Presence** — who is online / who is editing, on top of realtime.
6. **Observability** — Prometheus metrics, OpenTelemetry traces.

### Honest weaknesses to keep in view

- PocketBase's ecosystem, docs and community are years ahead.
- JS hooks (`pb_hooks`) are not wired up yet (W8). For many PocketBase
  users that is *the* extension point.
- We are unproven in production; PocketBase is not.

---

## 4. Traps that have already cost time

Read this before starting. Each of these was a real bug in this repo.

**The API is PocketBase v0.23+, not v0.22.** `/api/admins/*` is gone —
superusers are an ordinary auth collection, `POST
/api/collections/_superusers/auth-with-password` with **`identity`**, not
`email`. Collections take **`fields`**, not `schema`. Per-field `unique`
no longer exists; use a collection-level index. Any snippet you find
online is probably pre-0.23.

**Collection ids are derived, not random:** `pbc_<crc32(type + name)>` —
note the *type* is in the hash. `base` + `_mfas` → `pbc_2279338944`.

**The JS SDK always prefixes the collection onto a realtime topic.**
`collection("posts").subscribe("*", cb)` puts **`posts/*`** on the wire,
never bare `posts`. Reading `*` as a record id cost four passing tests.

**Realtime access is decided in memory, not by a query.** `listRule`
governs (not `viewRule`), it is applied per record, and a `delete` of a
matching row is still delivered *after* the row is gone — so
`SELECT ... WHERE id = ?` cannot answer. Use
`cratebase_db::rules::check_rule_against_row`.

**Verify against the real PocketBase binary rather than reasoning.**
`/home/diwa/Downloads/pocketbase_0.40.2_linux_amd64/pocketbase`. Every
realtime semantic above came from a 100-line probe script that ran both
servers and diffed the answers. Two separate guesses of mine were wrong
and the binary corrected them. Do this whenever behaviour is unclear.

**Check exit codes, not filtered output.** `cargo clippy ... | grep -E
"^error"` reported clean while clippy was failing with 4 errors, because
the shell reported the *pager's* status. Run CI's exact command and read
`$?`.

**Bun is pinned to 1.4.0 in CI** because `web/admin/dist` is committed and
CI byte-compares a fresh build against it. A different Bun emits different
content hashes. If you change the dashboard, rebuild and commit `dist`.

**Beware inherited env vars when running the server by hand.**
`CRATEBASE_DATA_DIR` / `DATABASE_URL` leaking from an earlier shell will
silently override `--dir`, and you will debug an empty database that isn't
the one you think you're looking at.

---

## 5. Architecture orientation

```
crates/core     domain types: Collection, Record, Settings, errors, ids
crates/filter   PocketBase filter grammar: lexer, parser, SQL compiler,
                and an in-process evaluator (eval.rs) used for rules
                against a record that isn't in the database
crates/db       rusqlite (1 writer + N readers) and tokio-postgres;
                rules.rs is the API-rule decision point
crates/auth     Argon2id + bcrypt verify, JWTs signed with
                secret + record.tokenKey
crates/storage  local filesystem or S3, via object_store
crates/mailer   SMTP
crates/jsvm     JavaScript hooks (rquickjs) — not wired up yet
crates/server   axum routes, extractors, SSE realtime, cron, embedded
                dashboard (rust-embed)
web/admin       React 19 + TanStack Router/Query + Tailwind v4, served
                at /_/ ; dist is committed
tests/conformance  Bun tests driving the official pocketbase npm SDK
```

## 6. Rules of the house

- **Match PocketBase byte for byte** unless there is a written reason not
  to. Divergences go in `tests/conformance/KNOWN_DIVERGENCES.md` with the
  measurement that justifies them.
- **Performance is a feature.** A change that costs throughput needs a
  number defending it.
- Comments explain *why*, never *what*. Match the surrounding density.
- Never claim something passes without running it.

## 7. How to verify anything

```bash
# Full conformance suite against Cratebase
cargo build --release -p cratebase-server
cd tests/conformance && SERVER_BIN=$PWD/../../target/release/cratebase bun test

# The same suite against real PocketBase, as the oracle
cd tests/conformance && PB_BIN=/path/to/pocketbase bun test

# One file
SERVER_BIN=... bun test realtime.test.ts

# Exactly what CI runs — check every exit code
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
cargo test --workspace
cd web/admin && bunx tsc -b && cd ../.. && bun run admin:build && bun run --cwd web/admin lint

# Benchmarks (idle machine only)
bash benchmarks/run.sh
bun run benchmarks/compare.ts benchmarks/results/{pocketbase,cratebase}.json
```

## 8. Suggested order

W1 (batch) and W2/W3/W4 (auth) are independent — run them in parallel.
That clears all 25 remaining failures and gets conformance to 180/181.

Then W6 (benchmark re-run) → W7 (docs with real numbers) → W5 (dashboard)
→ W8 (JS hooks), and only then the value-add roadmap in §3.
