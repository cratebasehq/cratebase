# PocketBase-compatibility conformance suite

This package drives a live server with the **official `pocketbase` npm SDK
(v0.28.x)** and asserts the exact HTTP behaviour PocketBase v0.40.2 exhibits:
status codes, response envelopes, validation codes and error wording.

The suite is written against PocketBase and validated against it, so it acts as
the oracle for Cratebase: **the same suite must pass unchanged against
`cratebase serve`.**

Only the public SDK surface is used — no private endpoints, no undocumented
hacks — so anything asserted here is something a real PocketBase app can rely
on.

## Running

```bash
bun install                        # from the repo root (this is a workspace)

# against the real PocketBase binary (the reference implementation)
PB_BIN=/tmp/pb-bench/pb/pocketbase bun run conformance

# against Cratebase
SERVER_BIN=target/release/cratebase bun run conformance

# against an already-running server (nothing is spawned)
BASE_URL=http://127.0.0.1:8090 ADMIN_EMAIL=... ADMIN_PASSWORD=... bun run conformance
```

`bun run conformance` is a root-level script for `bun run --cwd
tests/conformance test`. Inside this directory `bun test` (optionally
`bun test records auth`) works too.

### Environment

| Variable | Default | Meaning |
| --- | --- | --- |
| `SERVER_BIN` / `PB_BIN` | – | Server binary to spawn. When set, the harness picks a free port, creates a fresh temp data dir, creates the superuser through the binary's CLI, waits for `/api/health`, and kills the server when the run ends. |
| `BASE_URL` | `http://127.0.0.1:8090` | Used when no binary is given. |
| `ADMIN_EMAIL` | `admin@conformance.test` | Superuser created/used by the harness. |
| `ADMIN_PASSWORD` | `conformance123` | |
| `SERVER_DIR` | temp dir | Reuse a specific data dir (it is then not deleted). |
| `KEEP_DATA` | – | Keep the temp data dir after the run (for debugging). |
| `MAIL_SINK` | auto | `0` disables the SMTP sink; `1` forces it on for an external `BASE_URL`. |
| `CONFORMANCE_VERBOSE` | – | Stream the server's stdout/stderr. |

Binary flavour is detected from the file name: a name containing `pocketbase`
uses `superuser create <email> <pass> --dir <dir>` and
`serve --http=127.0.0.1:<port> --dir=<dir> --automigrate=false`; anything else
is treated as Cratebase and gets `superuser create <email> <pass>` + `serve`
with `CRATEBASE_DATA_DIR`, `DATABASE_URL` and `PORT` in the environment.

> PocketBase's automigrate would otherwise write JS migration files next to the
> data dir and replay them into the *next* fresh run, which breaks isolation —
> hence `--automigrate=false` plus explicit `--migrationsDir/--hooksDir/--publicDir`
> inside the temp dir.

## What the harness provides

* **Server lifecycle** — spawn, health-wait, superuser creation, teardown.
* **SMTP sink** (`harness.ts`) — an in-process SMTP server wired into
  `settings.smtp`, so emails (verification, password reset, email change, OTP,
  auth alerts) can be read back with `waitForMail()` / `tokenFromMail()`. This
  is what makes the OTP and MFA *happy paths* testable against PocketBase
  itself, without any test-only API. Disable with `MAIL_SINK=0`; those tests
  then skip.
* **EventSource polyfill** — Bun's test runtime has no global `EventSource`,
  which the SDK's realtime service requires. `harness.ts` installs a ~100-line
  fetch/stream implementation (named events, `data`, `lastEventId`,
  `addEventListener`, `close()`), so the `eventsource` npm package is *not* a
  dependency. If you run the suite on a runtime that has a native
  `EventSource`, that one is used instead.
* **Helpers** — `client()`, `adminClient()`, `uniq()`, `expectError()`,
  `dropCollection()`, `waitFor()`.

## Layout

| File | Covers |
| --- | --- |
| `health.test.ts` | `/api/health` envelope (`code`, not `status`), superuser-only `data`. |
| `collections.test.ts` | Collection CRUD with the v0.23 `fields` shape, every field type + options, auth/view collections, indexes, import/`deleteMissing`, truncate, scaffolds, validation codes. |
| `records.test.ts` | CRUD, pagination, `skipTotal`, sorting (incl. `@random`, relation sort), the filter language (operators, `:each`, `:length`, `:lower`, date macros, back-relations, `@collection.X`, `@request.*`), `expand`, `fields` (incl. `:excerpt`), `+`/`-` modifiers, `@jsonPayload`, per-field validation codes, API rules. |
| `auth.test.ts` | Password login, `identityFields`, refresh, token invalidation, `emailVisibility`, `manageRule`, `authRule`, verification / password reset / email change (incl. happy paths through the mail sink), impersonation, `listAuthMethods`, `_authOrigins`, external auths, OTP, MFA. |
| `files.test.ts` | Uploads (single/multi/`+`/`-`), stored-name shape, thumbs, `?download=1`, protected files and file tokens, size/mime validation. |
| `realtime.test.ts` | SSE subscribe to collection and record topics, create/update/delete events, topic options (`filter`, `fields`, `expand`), list-rule enforcement, unsubscribe. |
| `batch.test.ts` | `/api/batch`: create/update/upsert/delete, rollback, per-request error shape, uploads, limits, disabled state. |
| `settings.test.ts` | `getAll` key set, partial update round trip, secrets never serialized, `testS3`/`testEmail`. |
| `logs.test.ts` | Request log rows (`level`, `message`, `data.*`), filters, `getOne`, `getStats` buckets. |
| `backups.test.ts` | Create/list/download/delete, name validation, auth. |
| `crons.test.ts` | Built-in job ids and expressions, `run`, the backup cron. |
| `errors.test.ts` | The `{status, message, data}` envelope for 400/401/403/404, `ClientResponseError` fields, aborts. |

## Notes for implementers

Assertions record **observed** PocketBase behaviour, not the documentation.
Where PocketBase does something surprising the test carries a `NOTE:` comment
explaining it — those comments are the specification. See
[`KNOWN_DIVERGENCES.md`](./KNOWN_DIVERGENCES.md) for the list of surprises and
for the single skipped test.
