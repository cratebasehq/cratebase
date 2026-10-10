# {{PROJECT_NAME}}

A [Vite](https://vite.dev) + React + TypeScript + Tailwind single-page app on [Cratebase](https://cratebase.dev).

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
2. **`vite`** — the app at <http://localhost:5173>.

Sign in with the seeded demo account, `demo@example.com` / `password123`, or create
your own. The Cratebase admin dashboard is at <http://localhost:8090/_/>.

## What's included

- **Auth** (`/sign-in`, `/sign-up`): password, email code (OTP), magic link, OAuth buttons for the
  providers enabled on the server, and a TOTP challenge when two-factor is on.
- **Dashboard** (`/dashboard`, behind [`RequireAuth`](./src/components/RequireAuth.tsx)): create, edit and
  delete notes; a realtime list (`useRecords(..., { realtime: true })`) that updates across tabs;
  full-text search; cover-image upload; a notifications bell. Loading, empty and error states included.
- **`schema.json`**: a `posts` collection (searchable `title`/`content`, optional `cover` file, `owner`
  relation) with `owner = @request.auth.id` rules, so people only see their own notes.
- **`cb_seed/`**: a demo user and two notes, applied once to an empty database.
- **`cb_hooks/welcome.cb.js`**: sends the built-in `welcome` email to every new user.
- Light and dark themes (follows the OS), labelled forms, keyboard-visible focus.

## Configuration

Copy `.env.example` to `.env` to change anything. `VITE_CRATEBASE_URL` (default `http://localhost:8090`) is the
Cratebase URL; it is exposed to the browser because the SDK talks to Cratebase directly.

## Deploying

Cratebase is a single binary (SQLite by default, Postgres via `DATABASE_URL`).

- **Run it:** `cratebase serve` behind Caddy or nginx for TLS, or use the
  [`docker-compose.yml`](https://github.com/cratebasehq/cratebase/blob/main/docker-compose.yml)
  in the Cratebase repo.
- **Frontend:** `bun run build` produces static files in `dist/` — host them anywhere (Netlify, Cloudflare Pages, next
  to Cratebase). Set `VITE_CRATEBASE_URL` to your public Cratebase URL at build time.

- Push `schema.json` to a deployed instance with `cratebase schema push` (or
  `POST /api/schema/apply`) — see [Schema as code](https://cratebase.dev/docs/extending/schema-as-code/).

## Project layout

```
src/pages/        routes: Home, SignIn, SignUp, MagicLinkCallback, Dashboard
src/components/   Field, PostForm, PostList, SearchBox, NotificationsBell, OAuthButtons, TotpChallenge, RequireAuth
src/lib/cratebase.ts   the client + typed hooks
schema.json       collections and rules (source of truth)
cb_seed/ cb_hooks/   demo data and server-side JS hooks
```

## Learn more

- [Cratebase docs](https://cratebase.dev/docs/)
- [`@cratebase/client`](https://github.com/cratebasehq/cratebase/tree/main/sdk/js/client#readme) — the typed SDK
- [`@cratebase/react`](https://github.com/cratebasehq/cratebase/tree/main/sdk/js/react#readme) — the hooks used throughout `src/`
- [Filter syntax](https://cratebase.dev/docs/concepts/api-rules/) — for editing rules in `schema.json`
