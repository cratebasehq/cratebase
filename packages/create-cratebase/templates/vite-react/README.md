# {{PROJECT_NAME}}

A [Vite](https://vitejs.dev) + React (TypeScript, Tailwind) single-page app
scaffolded by
[`create-cratebase`](https://cratebase.dev/docs/getting-started/starter-kits/),
wired up to [Cratebase](https://cratebase.dev) — a self-hostable,
PocketBase-compatible Rust backend.

## 60-second quickstart

```bash
bun install      # or npm install / pnpm install / yarn
bun run dev
```

`dev` runs two things at once:

1. **`cratebase dev`** — starts a local Cratebase server, provisioning a
   superuser, applying [`schema.json`](./schema.json), seeding
   [`pb_seed/`](./pb_seed) (a demo user and two notes) on first run, and
   writing/watching [`cratebase-types.d.ts`](./cratebase-types.d.ts). If the
   `cratebase` binary isn't installed yet, this prints the one-line install
   command instead of failing silently.
2. **`vite`** — the frontend, at <http://localhost:5173>.

Sign in with the seeded demo account — `demo@example.com` / `password123` —
or create your own from `/sign-up`. The Cratebase dashboard is at
<http://localhost:8090/_/> (superuser credentials are printed in the
terminal on first run).

## What's included

- **Auth** (`/sign-in`, `/sign-up`) — password, email OTP ("email code"),
  magic link, OAuth buttons (rendered only for providers actually
  configured on the server), and TOTP/MFA challenge handling if the
  signed-in account has two-factor enabled.
- **Protected dashboard** (`/dashboard`, guarded by
  [`RequireAuth`](./src/components/RequireAuth.tsx)) — a realtime list of
  your own notes (`useRecords(..., { realtime: true })`), create/edit/delete
  via `useMutation`, a full-text search box, cover-image upload via
  `useUpload`, and a notifications bell (`useNotifications`).
- **`schema.json`** — one `posts` collection (title/content, both
  searchable; an optional `cover` file; an `owner` relation), scoped with
  `owner = @request.auth.id` list/view/update/delete rules.
- **`pb_seed/`** — a demo user and two notes, applied once on an empty
  database.
- **`pb_hooks/welcome.pb.js`** — emails the built-in `welcome` template to
  every new signup via `$mails.send`.

## Deploying

Cratebase ships as a single binary — no separate database server required
for a small app (SQLite by default; point `DATABASE_URL` at Postgres for
more headroom).

- **Single binary behind a reverse proxy** — run `cratebase serve` on the
  box, put [Caddy](https://caddyserver.com) or nginx in front of it for
  TLS.
- **Docker** — see the
  [`docker-compose.yml`](https://github.com/cratebasehq/cratebase/blob/main/docker-compose.yml)
  in the Cratebase repo. Point this app's `VITE_CRATEBASE_URL` at your
  deployed server and `vite build` the static output anywhere (Netlify,
  Cloudflare Pages, a static file server next to Cratebase itself, etc).
- Push `schema.json` to the deployed instance with `cratebase schema push`
  (or `POST /api/schema/apply`) as part of your deploy — see
  [Schema as code](https://cratebase.dev/docs/extending/schema-as-code/).

## Learn more

- [Cratebase docs](https://cratebase.dev/docs/) — start with
  [Getting started](https://cratebase.dev/docs/getting-started/install/).
- [`@cratebase/client`](https://github.com/cratebasehq/cratebase/tree/main/sdk/js/client#readme) —
  the typed SDK this app talks to Cratebase with.
- [`@cratebase/react`](https://github.com/cratebasehq/cratebase/tree/main/sdk/js/react#readme) —
  the hooks used throughout `src/components/` and `src/pages/`.
