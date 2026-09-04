<div align="center">

# Cratebase

**A fast, self-hostable backend — dynamic collections, auth, file storage,
and realtime — in one Rust binary.**

SQLite by default. Switch to Postgres by changing one environment variable.
No code changes either way.

</div>

Cratebase is what you reach for instead of Hono + a hand-rolled REST layer +
Postgres/S3/auth wiring every time you start a new project. Define a
collection's shape, get a full CRUD REST API, auth, file uploads, and
realtime subscriptions for it immediately — from your web app, an Expo app,
or a coding agent that just needs a backend.

Same shape end to end — dynamic collections, API rules enforced in SQL,
one binary, embedded admin dashboard — plus **Postgres as a first-class,
zero-code-change option** for when SQLite stops being enough.

## Features

- **Dynamic collections** — define fields (text, number, bool, email, url,
  date, select, JSON, relation, file, password) through the API/dashboard;
  Cratebase creates and migrates the real SQL table for you.
- **SQLite or Postgres** — `DATABASE_URL=sqlite://...` (default) or
  `DATABASE_URL=postgres://...`. Identical API, identical filter syntax,
  identical rules either way.
- **API rules** — a small filter expression language
  (`owner = @request.auth.id`) for list/view/create/update/delete access,
  enforced in SQL, not in application code you have to trust.
- **Auth built in** — any collection can be `type: "auth"` and gets
  `email`/`password`, `POST .../auth-with-password`, and
  `POST .../auth-refresh` for free. Registration is just creating a record.
  Beyond password login: email verification, password reset, email-change
  confirmation, OTP (passwordless) login, MFA, OAuth2 (Google/GitHub),
  superuser impersonation, and new-location login alerts (`_authOrigins`)
  are all built in — see [ROADMAP.md](./ROADMAP.md) for the endpoint list.
- **Batch API** — `POST /api/batch` runs several record
  create/update/upsert/delete calls (JSON or multipart, for file fields)
  in one HTTP round trip and one SQL transaction: all of them commit or
  none do.
- **JS hooks (PocketBase parity)** — drop a `*.pb.js` file in `pb_hooks/`
  next to your data directory and get PocketBase's own hook API:
  `onRecordCreate`/`onRecordUpdate`/-style lifecycle hooks and `routerAdd`
  for custom HTTP endpoints, both backed by an embedded QuickJS runtime
  (`crates/jsvm`) — no separate process, no restart-to-reload. This is the
  most-requested PocketBase capability that was previously missing.
- **First-run setup, no CLI required** — point a browser at a fresh
  instance and the dashboard shows an inline superuser-creation form
  (`GET/POST /api/setup`) instead of a bare login screen. The
  `superuser create` CLI command still works for scripted/headless setup.
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
- **PocketBase-compatible API** — the official [`pocketbase`](https://www.npmjs.com/package/pocketbase) JS/TS client (and PocketBase's other official SDKs) work against Cratebase unchanged. Verified against the
  real SDK: 180/181 conformance tests pass (1 test is skipped because it
  restarts the server mid-run to test backup restore, which would kill the
  test harness itself — see
  [tests/conformance/KNOWN_DIVERGENCES.md](./tests/conformance/KNOWN_DIVERGENCES.md)).

## Quickstart

### Docker (recommended)

```bash
docker compose up
```

That's SQLite + local disk storage, listening on `:8090`. Open
`http://localhost:8090` and the dashboard's first-run setup form creates
your superuser account — no CLI needed. (You can still script it instead:
`docker compose exec cratebase cratebase superuser create you@example.com yourpassword`.)

Want Postgres and S3-compatible storage instead of the defaults?

```bash
docker compose --profile postgres --profile s3 up
```

See [.env.example](./.env.example) for every configuration option.

### From source

```bash
cargo build --release -p cratebase-server
./target/release/cratebase serve
```

This gets you the API immediately. Open `http://localhost:8090` and the
dashboard walks you through creating a superuser — or run
`./target/release/cratebase superuser create you@example.com yourpassword`
first if you'd rather skip the form. The admin dashboard needs its
frontend built once first (`bun install && bun run admin:build` at the
repo root, then rebuild the Rust binary so it picks up the new
`web/admin/dist`) — see [ARCHITECTURE.md](./ARCHITECTURE.md) for why it
works this way.

## Using it

```bash
# create a collection
curl -X POST localhost:8090/api/collections \
  -H "authorization: Bearer $ADMIN_TOKEN" -H 'content-type: application/json' \
  -d '{"name":"posts","type":"base",
       "schema":[{"id":"f1","name":"title","type":"text","required":true},
                 {"id":"f2","name":"published","type":"bool"}],
       "listRule":"","viewRule":"","createRule":"","updateRule":"","deleteRule":null}'

# create a record — no auth needed, createRule is public ("")
curl -X POST localhost:8090/api/collections/posts/records \
  -H 'content-type: application/json' -d '{"title":"Hello","published":true}'

# list, with a filter
curl "localhost:8090/api/collections/posts/records?filter=published%20%3D%20true&sort=-created"
```

From TypeScript/JavaScript:

```ts
import PocketBase from "pocketbase";

const cb = new PocketBase("http://localhost:8090");
const posts = await cb.collection("posts").getList(1, 20, { filter: "published = true" });
const unsubscribe = await cb.collection("posts").subscribe("*", (e) => console.log(e.action, e.record));
```

Full API reference: [openapi.yaml](./openapi.yaml). Quick orientation for
humans or LLMs: [llms.txt](./llms.txt).

## Server-side rendering

No code gap here either: the official [`pocketbase`](https://www.npmjs.com/package/pocketbase)
SDK already supports the standard SSR pattern used by meta-frameworks
like TanStack Start, Next.js, and SvelteKit — a fresh client instance
per request, with the auth store hydrated from (and re-serialized back
into) a cookie. Nothing Cratebase-specific is required beyond pointing
`PocketBase` at your server URL.

```ts
// inside a TanStack Start server function / loader (same shape for any
// Node-based SSR framework — swap getCookie/setCookie for your
// framework's request/response cookie helpers)
import PocketBase from "pocketbase";

const pb = new PocketBase(process.env.CRATEBASE_URL);
pb.autoCancellation(false); // see below
pb.authStore.loadFromCookie(getCookie("pb_auth") ?? "");

const posts = await pb.collection("posts").getList(1, 20);

setCookie("pb_auth", pb.authStore.exportToCookie());
```

> **`autoCancellation(false)` is required in every server context.** By
> default the SDK cancels an in-flight request when an identical one is
> issued again — a de-duplication behavior meant for client-side UI
> (e.g. a component re-rendering mid-fetch). On the server, each
> incoming request gets its own `PocketBase` instance, but the SDK has
> no way of knowing that two loaders calling the same endpoint at the
> same time are actually two independent requests, not one UI component
> re-firing — with auto-cancellation left on, it can silently cancel one
> of them. Call `pb.autoCancellation(false)` on every server-side client
> you construct.

## Examples

Four runnable apps in [examples/](./examples), each with its own
`setup.sh` that provisions the collections it needs:

- [examples/todo](./examples/todo) — the minimal register → login →
  authenticated CRUD path, zero-build (a single `app.js` loaded via an
  import map, no bundler).
- [examples/realtime-chat](./examples/realtime-chat) — a shared chat room
  built on realtime subscriptions.
- [examples/realtime-cursors](./examples/realtime-cursors) — live cursor
  positions broadcast between open tabs over realtime.
- [examples/kanban](./examples/kanban) — the flagship demo: a shared,
  realtime, drag-and-drop Kanban board (Vite + React + TypeScript,
  optimistic updates, FLIP-animated card reflow, and a presence bar)
  demonstrating auth, API rules, and realtime working together end to
  end, not each in isolation.

`bun run examples:serve` serves the three zero-build examples from the
repo root; `examples/kanban` has its own Vite dev server (`npm run dev`
inside `examples/kanban`) since it has an actual build step.

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

### Plugins

Extend the server without forking it: implement the `Plugin` trait
(`crates/server/src/plugin.rs`) for one-time setup, extra HTTP routes
under `/api/plugins/<name>`, and fixed-interval background jobs, then
register it in a `PluginRegistry` — including from a downstream binary
that depends on `cratebase-server` as a library, not just this repo's own
`cratebase` binary. See `crate::plugin`'s module doc for the trait shape;
there is no bundled example plugin in this repo yet — `crates/server/src/cron_jobs.rs`
is the closest reference for the "reactive system collection" shape a
plugin author would follow, even though it isn't built on the `Plugin`
trait itself.

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
