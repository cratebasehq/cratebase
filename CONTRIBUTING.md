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

## The API surface

The REST API is Cratebase's own contract: `openapi.yaml` and the site's
REST reference are the source of truth. When you add or change an
endpoint, update both, add a test in `crates/server/tests`, and note a
breaking change in CHANGELOG.md. Prefer rejecting unknown or legacy input
with a clear error over silently ignoring it.

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
  `web/admin`, `web/email`, and `packages/create-cratebase`.
- **`docker`** — builds the Docker image (`docker/build-push-action`,
  `push: false`) to make sure the `Dockerfile` still produces a working
  image; it runs after `rust`, `dashboard`, and `email` all pass.

SDK packages are checked locally with `bun run sdk:check` (typecheck,
build and tests for `sdk/js/{client,react,start,schema}`); it is not a
CI job of its own, so run it before opening a PR that touches `sdk/`.

## Pull requests

- Keep changes scoped; call out any change to the REST API surface (it
  may break existing clients) in the PR description and CHANGELOG.md.
- If you touch `web/admin/src`, commit the rebuilt `web/admin/dist` in the
  same PR.
- Match the existing crate boundaries described in the README's "Project
  layout" section and `ARCHITECTURE.md` — new I/O-bearing code doesn't
  belong in `crates/core`, new SQL doesn't belong outside `crates/db`,
  etc.
