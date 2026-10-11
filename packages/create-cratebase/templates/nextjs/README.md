# {{PROJECT_NAME}}

A [Next.js](https://nextjs.org) (App Router) + TypeScript + Tailwind app on [Cratebase](https://cratebase.dev).

## Quickstart

```sh
bun install      # or: npm install / pnpm install / yarn
bun run dev      # or: npm run dev / pnpm dev / yarn dev
```

`dev` starts two processes side by side:

1. **`cratebase dev`** — a local Cratebase server on <http://localhost:8090>. It
   applies [`schema.json`](./schema.json), seeds [`cb_seed/`](./cb_seed) on the first
   run, regenerates [`cratebase-types.d.ts`](./cratebase-types.d.ts) when the schema
   changes, and prints a superuser login. If the `cratebase` binary is missing, the
   script prints the install command:
   `curl -fsSL https://cratebase.dev/install.sh | sh`.
2. **`next dev`** — the app at <http://localhost:3000>.

Sign in with the seeded demo account, `demo@example.com` / `password123`, or create
your own. The Cratebase admin dashboard is at <http://localhost:8090/_/>.

## What's included

- **Auth** (`/sign-in`, `/sign-up`): password, email code (OTP), magic link, OAuth buttons for the
  providers enabled on the server, and a TOTP challenge when two-factor is on.
- **Dashboard** (`/dashboard`, guarded by [`app/dashboard/layout.tsx`](./app/dashboard/layout.tsx)): create, edit
  and delete notes; a realtime list that updates across tabs; full-text search; cover-image upload; a
  notifications bell. Loading, empty and error states included.
- **SSR-safe client setup:** `lib/cratebase.ts` creates the client at module scope (it never touches
  `window` until a hook runs) and `app/providers.tsx` is the single `"use client"` boundary for
  `<CratebaseProvider>`. Auth state lives in `localStorage`, so the guard is client-side; collection rules
  are the real security boundary. For server-rendered auth, use the
  [`tanstack-start`](https://cratebase.dev/docs/getting-started/starter-kits/) template or the
  [cookie guide](https://cratebase.dev/docs/sdk/ssr-and-cookies/).
- **`schema.json`**: a `posts` collection with `owner = @request.auth.id` rules (people only see their own notes).
- **`cb_seed/`**, **`cb_hooks/welcome.cb.js`**: a demo user and notes, and a welcome email on signup.
- Light and dark themes (follows the OS), labelled forms, keyboard-visible focus.

## Configuration

Copy `.env.example` to `.env.local` to change anything. `NEXT_PUBLIC_CRATEBASE_URL` (default
`http://localhost:8090`) is the Cratebase URL; it is exposed to the browser.

## Deploying

Cratebase is a single binary (SQLite by default, Postgres via `DATABASE_URL`).

- **Run it:** `cratebase serve` behind Caddy or nginx for TLS, or use the
  [`docker-compose.yml`](https://github.com/cratebasehq/cratebase/blob/main/docker-compose.yml)
  in the Cratebase repo.
- **Frontend:** `bun run build && bun run start`, or deploy to Vercel, Fly.io or a container. Set
  `NEXT_PUBLIC_CRATEBASE_URL` to your public Cratebase URL at build time.

- Push `schema.json` to a deployed instance with `cratebase schema push` (or
  `POST /api/schema/apply`) — see [Schema as code](https://cratebase.dev/docs/extending/schema-as-code/).

## Project layout

```
app/               routes: (auth)/sign-in, (auth)/sign-up, auth/magic-link, dashboard
components/        Field, PostForm, PostList, SearchBox, NotificationsBell, OAuthButtons, TotpChallenge
lib/cratebase.ts   the client + typed hooks
schema.json        collections and rules (source of truth)
cb_seed/ cb_hooks/ demo data and server-side JS hooks
```

## Learn more

- [Cratebase docs](https://cratebase.dev/docs/)
- [`@cratebase/client`](https://github.com/cratebasehq/cratebase/tree/main/sdk/js/client#readme) — the typed SDK
- [`@cratebase/react`](https://github.com/cratebasehq/cratebase/tree/main/sdk/js/react#readme) — the hooks used throughout `components/` and `app/`
- [Filter syntax](https://cratebase.dev/docs/concepts/api-rules/) — for editing rules in `schema.json`
