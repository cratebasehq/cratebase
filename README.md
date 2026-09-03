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
- **File storage** — local disk by default; switch to any S3-compatible
  bucket (AWS S3, Cloudflare R2, Backblaze B2, or self-hosted
  [RustFS](https://rustfs.com)/MinIO) with three environment variables.
- **Realtime** — subscribe to a collection or a single record over SSE;
  get `create`/`update`/`delete` events as they happen.
- **One binary** — the admin dashboard is embedded at compile time
  (`rust-embed`). `cratebase serve` is the whole deployment.
- **Official TypeScript SDK** — `npm install cratebase`.

## Quickstart

### Docker (recommended)

```bash
docker compose up
```

That's SQLite + local disk storage, listening on `:8090`. Create your first
superuser and open the dashboard:

```bash
docker compose exec cratebase cratebase superuser create you@example.com yourpassword
open http://localhost:8090
```

Want Postgres and S3-compatible storage instead of the defaults?

```bash
docker compose --profile postgres --profile s3 up
```

See [.env.example](./.env.example) for every configuration option.

### From source

```bash
cargo build --release -p cratebase-server
./target/release/cratebase superuser create you@example.com yourpassword
./target/release/cratebase serve
```

This gets you the API immediately. The admin dashboard needs its frontend
built once first (`bun install && bun run admin:build` at the repo root,
then rebuild the Rust binary so it picks up the new `web/admin/dist`) — see
[ARCHITECTURE.md](./ARCHITECTURE.md) for why it works this way.

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
import { Cratebase } from "cratebase";

const cb = new Cratebase("http://localhost:8090");
const posts = await cb.collection("posts").getList(1, 20, { filter: "published = true" });
const unsubscribe = await cb.realtime.subscribe("posts", (e) => console.log(e.action, e.record));
```

Full API reference: [openapi.yaml](./openapi.yaml). Quick orientation for
humans or LLMs: [llms.txt](./llms.txt).

## Project layout

```
crates/core     domain types (Collection, Field, AppError) — no I/O
crates/filter   filter expression parser + SQL compiler
crates/db       storage engine: sqlx over sqlite/postgres, collection<->table sync
crates/storage  file storage: local disk or any S3-compatible bucket
crates/auth     Argon2id password hashing + JWT sessions, OAuth2, OTP/MFA
crates/mailer   pluggable mail backend (Resend API, SMTP, or log-only for dev)
crates/server   axum HTTP API, CLI, plugin system, embedded admin dashboard
sdk/js          official TypeScript client ("cratebase" on npm)
sdk/dart        Dart client
sdk/go          Go client
sdk/python      Python client
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
`cratebase` binary. Three real examples ship in
`crates/server/src/plugins/`: `cron_jobs.rs` (calendar cron via
`croner`, reading a `_cron_jobs` collection), `feature_flags.rs` (a
`_feature_flags` collection + the existing rule engine), and
`queue.rs` (a durable job queue with retry/reclaim) — `example.rs` is
the minimal reference to copy from.

## Development

```bash
# Rust workspace
cargo test --workspace                 # sqlite-backed tests always run
TEST_POSTGRES_URL=postgres://... cargo test --workspace   # + postgres parity tests
TEST_S3_ENDPOINT=http://localhost:9000 cargo test -p cratebase-storage  # + real S3-compatible test

# admin dashboard (proxies /api to a local `cratebase serve` on :8090)
bun install && bun run admin:dev

# JS SDK
bun run sdk:build
```

## License

MIT — see [LICENSE](./LICENSE).
