# Changelog

All notable changes to this project are documented in this file. The
format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project is pre-1.0 (currently `0.1.0`, per `Cargo.toml`) with no
tagged releases yet — entries below are grouped by merged pull request
rather than by release tag, reconstructed from the actual merge history
(`git log --merges` / `gh pr list --state merged`) on this repository.

## [Unreleased]

- Third-party adoption pass: contribution/security/changelog docs,
  published `@cratebase/extras` npm package, GHCR Docker image publishing,
  and a PocketBase migration tool/guide.

## 0.1.0 — 2026-09-04

### Added

- **Value-add Wave 4** (PR [#10](https://github.com/nicoaudy/cratebase/pull/10)):
  fixed cross-node realtime on Postgres, incoming webhooks, OAuth2 login
  (Google/GitHub/custom providers), a DocMind example app, admin
  dashboard code-splitting, and an in-dashboard API docs tab.
- **Value-add Wave 3** (PR [#7](https://github.com/nicoaudy/cratebase/pull/7)):
  superuser-minted API keys, an MCP (Model Context Protocol) dashboard
  and client helpers, push notifications (Web Push/FCM/APNs), Postgres
  multi-node realtime via `LISTEN`/`NOTIFY`, schema-as-code, presence,
  and a security hardening pass.
- **Value-add Wave 1+2** (PR [#6](https://github.com/nicoaudy/cratebase/pull/6)):
  custom SQL cron jobs, outgoing webhooks, an in-dashboard SQL console,
  a file manager, vector search, MCP support, an LLM chat gateway, and
  team/workspace membership.
- **Full auth surface and remaining conformance gaps** (PR [#5](https://github.com/nicoaudy/cratebase/pull/5)):
  batch API, email verification/password reset/email-change confirmation,
  OTP passwordless login, MFA, superuser impersonation, new-location
  login alerts, first-run setup (no CLI required), and JS hooks
  (PocketBase-parity `pb_hooks/*.pb.js` support) — closing conformance to
  180/181 against the real PocketBase test suite.
- **Realtime (SSE)** (PR [#3](https://github.com/nicoaudy/cratebase/pull/3)):
  `GET /api/realtime` subscriptions, reaching 7/7 realtime conformance
  tests and 155/181 overall against the PocketBase SDK conformance suite.
- Manual macOS ARM64 build workflow (PR [#4](https://github.com/nicoaudy/cratebase/pull/4)).
- Initial worktree/project revamp establishing the current crate layout
  (`crates/core`, `crates/db`, `crates/filter`, `crates/storage`,
  `crates/auth`, `crates/jsvm`, `crates/mailer`, `crates/server`) and
  admin dashboard (PR [#1](https://github.com/nicoaudy/cratebase/pull/1)).

### Fixed

- Rebuilt a stale committed admin dashboard bundle and added the CI check
  that catches this drift going forward (PR [#9](https://github.com/nicoaudy/cratebase/pull/9)).
- CI: rebuilt dashboard `dist`, and ignored `RUSTSEC-2023-0071` (a Marvin
  Attack RSA timing side-channel with no upstream fix, reachable only
  through an unused RSA code path pulled in transitively for VAPID/Web
  Push signing — see the inline comment in
  `.github/workflows/ci.yml` for the full justification) (PR [#8](https://github.com/nicoaudy/cratebase/pull/8)).
- CI: fixed `jsvm` lifetime issues, a one-argument `cb.send` regression,
  and pinned an previously-unpinned Bun version so the committed dashboard
  bundle byte-comparison stopped failing spuriously (PR [#2](https://github.com/nicoaudy/cratebase/pull/2)).

[Unreleased]: https://github.com/nicoaudy/cratebase/compare/main...HEAD
