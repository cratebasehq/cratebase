## What changed and why

<!--
Describe the change and the motivation. If it's a behavioral change to
the REST API surface, call it out explicitly (it may break existing
clients) and note it in CHANGELOG.md.
-->

## How this was verified

<!--
Cite exact commands and their output/numbers, not "tests pass" — this
matches how this repo's own PRs are described (see CHANGELOG.md/
ROADMAP.md entries, which cite specific commands, pass counts, and
measurements rather than unqualified claims). For example:

    cargo test --workspace
    -> 412 passed; 0 failed

    TEST_POSTGRES_URL=postgres://... cargo test --workspace
    -> 412 passed; 0 failed

    bun run sdk:check
    -> client, react, start and schema packages typecheck and pass
-->

## Checklist

Matches the jobs in `.github/workflows/ci.yml` — check off what you ran
locally; CI will re-run all of it regardless.

- [ ] `cargo fmt --all -- --check` — no formatting diff (job `rust`)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — no warnings (job `rust`)
- [ ] `cargo build --workspace` succeeds (job `rust`)
- [ ] `cargo test --workspace` passes, including against a real Postgres
      instance (`TEST_POSTGRES_URL=...`) if this touches `crates/db` (job `rust`)
- [ ] If `web/admin/src` changed: `bunx tsc -b` in `web/admin`, then
      `bun run admin:build`, then `bun run --cwd web/admin lint`, and the
      rebuilt `web/admin/dist` is committed in this PR (job `dashboard`)
- [ ] If `web/email` changed: `bun run build` in `web/email` succeeds and
      the rebuilt `dist` is committed (job `email`)
- [ ] If this changes dependencies: `cargo audit` and
      `bun audit --audit-level=high` are clean, or the new advisory is
      already tracked/ignored with a written reason (job `dependency-scan`)
- [ ] If this touches the REST API surface: `openapi.yaml` and the site's
      REST reference pages are updated to match

