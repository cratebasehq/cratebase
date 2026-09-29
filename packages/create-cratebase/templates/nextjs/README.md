# {{PROJECT_NAME}}

A [Next.js](https://nextjs.org) (App Router) app scaffolded by
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
2. **`next dev`** — the frontend, at <http://localhost:3000>.

Sign in with the seeded demo account — `demo@example.com` / `password123` —
or create your own from `/sign-up`. The Cratebase dashboard is at
<http://localhost:8090/_/> (superuser credentials are printed in the
terminal on first run).

## What's included

- **Auth** (`/sign-in`, `/sign-up`) — password, email OTP ("email code"),
  magic link, OAuth buttons (rendered only for providers actually
  configured on the server — see [`OAuthButtons`](./components/OAuthButtons.tsx)),
  and TOTP/MFA challenge handling if the signed-in account has two-factor
  enabled.
- **Protected dashboard** (`/dashboard`) — a realtime list of your own notes
  (`useRecords(..., { realtime: true })`), create/edit/delete via
  `useMutation`, a full-text search box, cover-image upload via `useUpload`,
  and a notifications bell (`useNotifications`).
- **SSR-safe client setup** — `lib/cratebase.ts` creates the client at
  module scope (safe on the server: it never touches `window`/`document`
  until a hook actually runs), and `app/providers.tsx` is the one
  `"use client"` boundary `<CratebaseProvider>` needs — see the Next.js
  note in [`@cratebase/react`'s README](https://github.com/cratebasehq/cratebase/tree/main/sdk/js/react#readme).
  For SSR-rendered auth state (not just client-side redirects), see the
  [cookie auth store guide](https://cratebase.dev/docs/sdk/ssr-and-cookies/).
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
more headroom). A typical deploy:

- **Single binary behind a reverse proxy** — run `cratebase serve` on the
  box, put [Caddy](https://caddyserver.com) or nginx in front of it for
  TLS. Caddy: a two-line `Caddyfile` (`example.com { reverse_proxy
  localhost:8090 }`) gets you automatic HTTPS.
- **Docker** — see the
  [`docker-compose.yml`](https://github.com/cratebasehq/cratebase/blob/main/docker-compose.yml)
  in the Cratebase repo for a ready-made Cratebase + Caddy setup; point this
  app's `NEXT_PUBLIC_CRATEBASE_URL` at that server's public URL and deploy
  the Next.js app anywhere that runs Node (Vercel, Fly.io, a container next
  to Cratebase itself, etc).
- Push `schema.json` to the deployed instance with `cratebase schema push`
  (or `POST /api/schema/apply`) as part of your deploy — see
  [Schema as code](https://cratebase.dev/docs/extending/schema-as-code/).

## Learn more

- [Cratebase docs](https://cratebase.dev/docs/) — start with
  [Getting started](https://cratebase.dev/docs/getting-started/install/).
- [`@cratebase/client`](https://github.com/cratebasehq/cratebase/tree/main/sdk/js/client#readme) —
  the typed SDK this app talks to Cratebase with.
- [`@cratebase/react`](https://github.com/cratebasehq/cratebase/tree/main/sdk/js/react#readme) —
  the hooks used throughout `components/` and `app/`.
- [Filter syntax](https://cratebase.dev/docs/concepts/api-rules/) — for
  editing `schema.json`'s rules or adding new collections.
