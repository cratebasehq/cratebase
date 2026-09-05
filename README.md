<div align="center">

# Cratebase

**A fast, self-hostable backend — dynamic collections, auth, file storage,
and realtime — in one Rust binary.**

</div>

```bash
curl -fsSL https://cratebase.dev/install.sh | sh
```

That downloads the right prebuilt binary for your OS/arch straight from
GitHub Releases, verifies its checksum, and installs it to
`~/.local/bin` — no Rust toolchain, no Docker, no build step. Prefer
Docker, building from source, or you're on Windows? See
[Installation](https://cratebase.dev/docs/getting-started/install/) on the
docs site.

Cratebase is what you reach for instead of Hono + a hand-rolled REST layer
+ Postgres/S3/auth wiring every time you start a new project. Define a
collection's shape, get a full CRUD REST API, auth, file uploads, and
realtime subscriptions for it immediately — from your web app, an Expo
app, or a coding agent that just needs a backend.

## What you get in the next 30 seconds

```bash
cratebase serve
```

starts listening on `:8090` immediately — SQLite and local-disk storage,
zero configuration. Open `http://localhost:8090` and the dashboard itself
shows an inline setup form instead of a login screen: fill in an email and
password there and that's your superuser account, no CLI step required.
(Prefer to script it instead? `cratebase superuser create you@example.com
yourpassword` works too, before or after that first visit.)

From there, create a collection and start reading/writing records over
HTTP or from the official PocketBase SDK:

```ts
import PocketBase from "pocketbase";

const cb = new PocketBase("http://localhost:8090");
const posts = await cb.collection("posts").getList(1, 20, { filter: "published = true" });
const unsubscribe = await cb.collection("posts").subscribe("*", (e) => console.log(e.action, e.record));
```

Full walkthrough (creating that first collection, the record/auth/realtime
APIs) is in [Getting started](https://cratebase.dev/docs/getting-started/first-collection/)
on the docs site. Quick orientation for humans or LLMs poking at a running
instance: [llms.txt](./llms.txt). Full endpoint reference:
[openapi.yaml](./openapi.yaml).

## Why Cratebase

- **PocketBase-wire-compatible.** The official
  [`pocketbase`](https://www.npmjs.com/package/pocketbase) JS/TS client
  (and PocketBase's other official SDKs) work against Cratebase
  unchanged — verified against the real SDK: 180/181 conformance tests
  pass (the one skip restarts the server mid-run to test backup restore,
  which would kill the test harness itself; see
  [tests/conformance/KNOWN_DIVERGENCES.md](./tests/conformance/KNOWN_DIVERGENCES.md)).
  Already on PocketBase? [Migrating from PocketBase](https://cratebase.dev/docs/migrating/migration-tool/)
  covers the actual migration tool.
- **Rust performance.** Same API, same filter syntax, same rules —
  meaningfully faster under load than PocketBase's Go implementation.
  Numbers and methodology (hardware, what's measured, honest caveats):
  [benchmarks/README.md](./benchmarks/README.md).
- **SQLite or Postgres, zero code changes.** `DATABASE_URL=sqlite://...`
  (default) or `DATABASE_URL=postgres://...` — identical API, identical
  filter syntax, identical rules either way.
- **An AI bundle that isn't a bolt-on.** Vector search, an LLM chat
  gateway, auto-embedding on write, and MCP server support, all backed by
  the same collections and API rules as everything else. See
  [AI](https://cratebase.dev/docs/ai/overview/) on the docs site.

## Full feature list

- **Dynamic collections** — define fields (text, number, bool, email, url,
  date, select, JSON, relation, file, password) through the API/dashboard;
  Cratebase creates and migrates the real SQL table for you.
- **API rules** — a small filter expression language
  (`owner = @request.auth.id`) for list/view/create/update/delete access,
  enforced in SQL, not in application code you have to trust.
- **Auth built in** — any collection can be `type: "auth"` and gets
  `email`/`password`, `POST .../auth-with-password`, and
  `POST .../auth-refresh` for free. Registration is just creating a
  record. Beyond password login: email verification, password reset,
  email-change confirmation, OTP (passwordless) login, MFA, OAuth2
  (Google/GitHub), superuser impersonation, and new-location login alerts
  (`_authOrigins`) are all built in — see
  [ROADMAP.md](./ROADMAP.md) for the endpoint list.
- **Batch API** — `POST /api/batch` runs several record
  create/update/upsert/delete calls (JSON or multipart, for file fields)
  in one HTTP round trip and one SQL transaction: all of them commit or
  none do.
- **JS hooks (PocketBase parity)** — drop a `*.pb.js` file in `pb_hooks/`
  next to your data directory and get PocketBase's own hook API:
  `onRecordCreate`/`onRecordUpdate`/-style lifecycle hooks and `routerAdd`
  for custom HTTP endpoints, both backed by an embedded QuickJS runtime
  (`crates/jsvm`) — no separate process, no restart-to-reload.
- **File storage** — local disk by default; switch to any S3-compatible
  bucket (AWS S3, Cloudflare R2, Backblaze B2, or self-hosted
  [RustFS](https://rustfs.com)/MinIO) with three environment variables.
- **Realtime** — subscribe to a collection or a single record over SSE;
  get `create`/`update`/`delete` events as they happen.
- **Custom cron jobs** — schedule a raw SQL statement to run on a cron
  expression from the dashboard's Cron jobs screen, no redeploy: a
  `_cron_jobs` record is the whole job (name, expression, SQL), reactive
  (a dashboard edit takes effect immediately), with the last run's
  status and error written back for you to see.
- **One binary** — the admin dashboard is embedded at compile time
  (`rust-embed`). `cratebase serve` is the whole deployment.
- **Server-side rendering** — the official SDK's standard SSR pattern
  (a fresh client per request, auth store hydrated from a cookie) works
  unmodified against Cratebase; no framework-specific glue needed.

See the [docs site](https://cratebase.dev/docs/) for the complete,
up-to-date reference — this list is intentionally a summary, not the full
pitch.

## Examples

Six runnable apps in [examples/](./examples), each with its own
`setup.sh` that provisions the collections it needs — from the minimal
register → login → authenticated CRUD path to a realtime, drag-and-drop
Kanban board. Full list with descriptions:
[Examples](https://cratebase.dev/docs/getting-started/examples/).

## Project layout

```
crates/core     domain types (Collection, Field, AppError) — no I/O
crates/filter   filter expression parser + SQL compiler
crates/db       storage engine: sqlx over sqlite/postgres, collection<->table sync
crates/storage  file storage: local disk or any S3-compatible bucket
crates/auth     Argon2id password hashing + JWT sessions, OAuth2, OTP/MFA
crates/jsvm     embedded QuickJS runtime — pb_hooks/, routerAdd, cronAdd
crates/mailer   pluggable mail backend (Resend API, SMTP, or log-only for dev)
crates/server   axum HTTP API, CLI, plugin system, embedded admin dashboard
web/admin       admin dashboard source (React + Vite + TanStack + shadcn/ui)
```

See [ARCHITECTURE.md](./ARCHITECTURE.md) for how the pieces fit together
and the trade-offs behind them, and [ROADMAP.md](./ROADMAP.md) for what's
shipped and what's still ahead.

## More

- [Configuration reference](https://cratebase.dev/docs/deploy/configuration-reference/)
  — every environment variable, including Postgres/S3/rate-limiting setup.
- [Docker](https://cratebase.dev/docs/deploy/docker/),
  [reverse proxy/TLS](https://cratebase.dev/docs/deploy/reverse-proxy-tls/),
  and other deployment topics.
- [Extending Cratebase](https://cratebase.dev/docs/extending/js-hooks/) —
  JS hooks, Rust plugins, cron jobs, webhooks.
- [Contributing, security policy, license](https://cratebase.dev/docs/project/contributing-security-license/)
  ([CONTRIBUTING.md](./CONTRIBUTING.md) and [LICENSE](./LICENSE) in this
  repo).

## Development

```bash
# Rust workspace
cargo test --workspace                 # sqlite-backed tests always run
TEST_POSTGRES_URL=postgres://... cargo test --workspace   # + postgres parity tests
TEST_S3_ENDPOINT=http://localhost:9000 cargo test -p cratebase-storage  # + real S3-compatible test

# admin dashboard (proxies /api to a local `cratebase serve` on :8090)
bun install && bun run admin:dev
```

## License

MIT — see [LICENSE](./LICENSE).
</content>
<parameter name="i">Rewrite README around install-first flow