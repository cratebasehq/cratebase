## What changed and why

<!--
Describe the change and the motivation. If it's a behavioral change to
the REST API surface, note whether it matches real PocketBase behavior
or is an intentional divergence — and if it's a divergence, where you
documented it in tests/conformance/KNOWN_DIVERGENCES.md (see
CONTRIBUTING.md's "compatibility rule").
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

    bun run conformance
    -> 180 passed, 1 skipped against real PocketBase; 181 pass against Cratebase
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
- [ ] If this touches the REST API surface: ran `bun run conformance`
      locally against real PocketBase-compatible behavior (not wired into
      CI yet — see CONTRIBUTING.md)
- [ ] Any intentional PocketBase divergence is documented in
      `tests/conformance/KNOWN_DIVERGENCES.md`

