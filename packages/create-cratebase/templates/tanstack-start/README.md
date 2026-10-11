# {{PROJECT_NAME}}

A [TanStack Start](https://tanstack.com/start) app (server-rendered, TypeScript, Tailwind) for Cloudflare Workers on [Cratebase](https://cratebase.dev). Sessions are HttpOnly cookies handled by [`@cratebase/start`](https://github.com/cratebasehq/cratebase/tree/main/sdk/js/start#readme).

## Quickstart

```sh
bun install      # or: npm install / pnpm install / yarn
cp .env.example .env
bun run dev      # or: npm run dev / pnpm dev / yarn dev
```

`dev` starts two processes side by side:

1. **`cratebase dev`** — a local Cratebase server on <http://localhost:8090>. It
   applies [`schema.json`](./schema.json), seeds [`cb_seed/`](./cb_seed) on the first
   run, regenerates [`cratebase-types.d.ts`](./cratebase-types.d.ts) when the schema
   changes, and prints a superuser login. If the `cratebase` binary is missing, the
   script prints the install command:
   `curl -fsSL https://cratebase.dev/install.sh | sh`.
2. **`vite dev`** — the app at <http://localhost:3000> (runs in a local Workers runtime).

Sign in with the seeded demo account, `demo@example.com` / `password123`, or create
your own. The Cratebase admin dashboard is at <http://localhost:8090/_/>.

## What's included

- **Cookie sessions:** [`src/lib/cratebase.ts`](./src/lib/cratebase.ts) creates `cb` with
  `createCratebaseStart`. Server functions call `cb.client()` (request-scoped, reads the session cookie) and
  sign-in/sign-up/sign-out set or clear the `cb_session` cookie (`HttpOnly`, `SameSite=Lax`, `Secure` over HTTPS).
- **Pages:** `/` (public latest notes, rendered on the server), `/sign-in`, `/sign-up`, `/dashboard` (create and
  delete your notes). The dashboard redirects when signed out, and its server functions are guarded by
  `cb.authMiddleware`, which answers 401 without a valid session.
- **`schema.json`**: a `posts` collection. Unlike the other templates, anyone may *read* notes (so the home page
  has content); only the owner may update or delete.
- **`cb_seed/`**, **`cb_hooks/welcome.cb.js`**: a demo user and notes, and a welcome email on signup.
- `cb.adminClient()` for server-side calls that bypass user rules (needs `CRATEBASE_API_KEY`).
- Light and dark themes (follows the OS), labelled forms.

## Configuration

Copy `.env.example` to `.env`:

- `CRATEBASE_URL` — where Cratebase listens (default `http://127.0.0.1:8090`). Server-only.
- `CRATEBASE_API_KEY` — optional `cb_…` key for `cb.adminClient()`. Server-only.

The Worker calls Cratebase from the server, so the browser never needs to reach it (no CORS setup).

## Deploying

Cratebase is a single binary (SQLite by default, Postgres via `DATABASE_URL`).

- **Run it:** `cratebase serve` behind Caddy or nginx for TLS, or use the
  [`docker-compose.yml`](https://github.com/cratebasehq/cratebase/blob/main/docker-compose.yml)
  in the Cratebase repo.
- **Worker:** `bun run deploy` (builds, then `wrangler deploy`). Set secrets first:
  `npx wrangler secret put CRATEBASE_URL` (and `CRATEBASE_API_KEY` if you use it). `CRATEBASE_URL` must be
  reachable from Cloudflare, so use a public HTTPS URL.
- Behind a proxy that terminates TLS, forward `X-Forwarded-Proto: https` so cookies get the `Secure` flag.

- Push `schema.json` to a deployed instance with `cratebase schema push` (or
  `POST /api/schema/apply`) — see [Schema as code](https://cratebase.dev/docs/extending/schema-as-code/).

## Project layout

```
src/routes/        __root, index, sign-in, sign-up, dashboard (file-based; src/routeTree.gen.ts is generated)
src/server/        server functions: auth.functions.ts, posts.functions.ts
src/lib/cratebase.ts   the `cb` helper (`@cratebase/start`)
schema.json        collections and rules (source of truth)
cb_seed/ cb_hooks/ demo data and server-side JS hooks
```

## Learn more

- [Cratebase docs](https://cratebase.dev/docs/)
- [`@cratebase/client`](https://github.com/cratebasehq/cratebase/tree/main/sdk/js/client#readme) — the typed SDK
- [TanStack Start docs](https://tanstack.com/start/latest)
- [Filter syntax](https://cratebase.dev/docs/concepts/api-rules/) — for editing rules in `schema.json`
