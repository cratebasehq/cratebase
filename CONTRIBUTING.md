# Contributing to Cratebase

Thanks for considering a contribution. This document describes the real,
current development workflow — every command below is what CI actually
runs (see `.github/workflows/ci.yml`), not aspirational tooling.

## Prerequisites

- Rust (stable toolchain — `rustfmt` and `clippy` components)
- [Bun](https://bun.sh) 1.4.0 (pinned in CI; the committed dashboard bundle
  is byte-compared against a fresh build, so a different Bun version can
  produce a spurious diff)
- Postgres 16 running locally (only needed to run the Postgres-parity test
  suite; the SQLite-backed tests run with no extra setup)

## Building

```bash
cargo build --workspace
```

The admin dashboard and email templates are pre-built and committed
(`web/admin/dist`, `web/email/dist`) so a bare `cargo build` produces a
working binary via `rust-embed`. If you change dashboard or email source,
rebuild those bundles and commit the result (see below) — CI fails the
build otherwise.

## Running the Rust tests

```bash
cargo test --workspace                                       # sqlite-backed tests always run
TEST_POSTGRES_URL=postgres://... cargo test --workspace      # + postgres parity tests
TEST_S3_ENDPOINT=http://localhost:9000 cargo test -p cratebase-storage  # + real S3-compatible test
```

CI runs `cargo test --workspace` against a real `postgres:16-alpine`
service container with `TEST_POSTGRES_URL` set, so any change touching
`crates/db` should be exercised against Postgres locally before opening a
PR, not just SQLite.

## Formatting and linting

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

Both are CI gates (job `rust` in `.github/workflows/ci.yml`) — a PR with
either a formatting diff or a clippy warning will not pass. Run
`cargo fmt --all` (without `--check`) to apply formatting locally.

## Admin dashboard

```bash
bun install
bun run admin:dev     # dev server, proxies /api to a local `cratebase serve` on :8090
bun run admin:build   # production build -> web/admin/dist
bun run --cwd web/admin lint
```

`web/admin/dist` is committed to the repository. CI's `dashboard` job
rebuilds it from source and runs `git diff --quiet -- web/admin/dist`: if
you touch anything under `web/admin/src`, run `bun run admin:build` and
commit the resulting `dist` changes, or the PR fails with "web/admin/dist
is stale."

## Email templates

```bash
cd web/email && bun install && bun run build
```

CI's `email` job runs the same build to make sure the templates still
compile; this bundle is also committed and embedded into the binary the
same way as the admin dashboard.

## Conformance suite

```bash
bun run conformance
```

This runs `tests/conformance` (a Bun test suite against the real
[`pocketbase`](https://www.npmjs.com/package/pocketbase) JS SDK) against a
Cratebase server. It's the suite that verifies byte-for-byte compatibility
with real PocketBase behavior — see the next section for what that means
in practice, and `tests/conformance/KNOWN_DIVERGENCES.md` for its current
pass/fail status (180 passing, 1 skipped against a real PocketBase server;
181 pass against Cratebase, since two assertions widen for Cratebase's
extra system collections/settings keys).

`tests/conformance` deliberately keeps its `pocketbase`-SDK-driven suites
even now that `@cratebase/client` exists — they're what prove wire
compatibility stays true, and that's a claim worth defending on its own.
The same directory's `client.test.ts` and `client-auth.test.ts` exercise
`@cratebase/client` instead; they cover ergonomics and behavior the
official SDK has no surface for (cookie sessions, impersonation, session
revocation, and so on). Both suites matter, for different reasons —
neither replaces the other, and a change to either client's behavior
should be checked against both.

## The compatibility rule

The project's stated engineering rule, from the existing
`docs/superpowers/specs/2026-09-04-next-work-handoff.md` handoff doc:

> **Match PocketBase byte for byte** unless there is a written reason not
> to. Divergences go in `tests/conformance/KNOWN_DIVERGENCES.md` with the
> measurement that justifies them.

In practice this means: if you're implementing or touching anything on
the REST API surface, your first source of truth is real PocketBase
behavior (status codes, error message wording, JSON field names/casing,
response shapes), not what seems more "correct" or what the PocketBase
docs describe — the docs and the actual server disagree in places, and
the actual server wins. `tests/conformance/KNOWN_DIVERGENCES.md` is full
of exactly these surprises (e.g. `=` being case-sensitive while `~` isn't,
a failing `listRule` returning an empty list rather than an error, batch
errors nesting a level deeper than documented).

### Adding a documented divergence

If your change intentionally differs from real PocketBase behavior (for
example, a new Cratebase-only feature, or a case where matching upstream
exactly isn't worth the cost), add an entry to
`tests/conformance/KNOWN_DIVERGENCES.md`. Its existing structure:

- **"Skipped tests"** — a table of test name → reason, for conformance
  tests that can't run against this harness at all (e.g. a restore test
  that would kill the harness mid-run).
- **"PocketBase behaviours the Rust implementation must match"** — a
  numbered list of specific, sometimes non-obvious upstream behaviors the
  suite asserts, grouped by area (Collections, Records/filters, Auth,
  Files, Batch/settings/logs/crons/backups). Add a new numbered item here
  if you discover (and now match) another one.
- **"Cratebase-only additions (not present in PocketBase)"** — capabilities
  that don't exist upstream at all, so they widen an expected-value list
  in the test suite rather than describing a behavioral difference on a
  shared surface (e.g. the extra system collections, extra settings keys).
- **"Suite-side workarounds (not PocketBase behaviour)"** — quirks of the
  test harness itself (e.g. Bun's test runtime lacking a global
  `EventSource`), not of either server.

Pick the section that matches your case, and justify it with the same
kind of evidence the existing entries use (a measurement, a concrete
error response, a specific endpoint/test name) — not just a prose
assertion that the new behavior is fine.

## What CI actually checks

From `.github/workflows/ci.yml`, a PR must pass all of:

- **`rust`** — `cargo fmt --all -- --check`, then
  `cargo clippy --workspace --all-targets -- -D warnings`, then
  `cargo build --workspace`, then `cargo test --workspace` (with a real
  Postgres 16 service container wired up via `TEST_POSTGRES_URL`).
- **`dashboard`** — `bunx tsc -b` in `web/admin`, `bun run admin:build`,
  `bun run --cwd web/admin lint`, then a check that the freshly built
  `web/admin/dist` exactly matches what's committed.
- **`email`** — builds `web/email` to make sure the templates compile.
- **`dependency-scan`** — `cargo audit` against the committed `Cargo.lock`
  (with one documented, unfixable RSA timing-sidechannel advisory
  ignored — see the workflow's inline comment for why it doesn't apply to
  any code path Cratebase actually exercises) and `bun audit
  --audit-level=high` against the committed `bun.lock` across
  `web/admin`, `web/email`, and `tests/conformance`.
- **`docker`** — builds the Docker image (`docker/build-push-action`,
  `push: false`) to make sure the `Dockerfile` still produces a working
  image; it runs after `rust`, `dashboard`, and `email` all pass.

The conformance suite (`bun run conformance`) is not currently wired into
`ci.yml` as its own job, but any change to the REST API surface should
still be run against it locally before opening a PR — it's the ground
truth for the compatibility rule above.

## Pull requests

- Keep changes scoped; note any intentional behavioral divergence from
  PocketBase (and where you documented it) in the PR description.
- If you touch `web/admin/src`, commit the rebuilt `web/admin/dist` in the
  same PR.
- Match the existing crate boundaries described in the README's "Project
  layout" section and `ARCHITECTURE.md` — new I/O-bearing code doesn't
  belong in `crates/core`, new SQL doesn't belong outside `crates/db`,
  etc.
