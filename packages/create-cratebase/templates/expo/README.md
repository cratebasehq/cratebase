# {{PROJECT_NAME}}

An [Expo Router](https://docs.expo.dev/router/introduction/) (TypeScript)
app scaffolded by
[`create-cratebase`](https://cratebase.dev/docs/getting-started/starter-kits/),
wired up to [Cratebase](https://cratebase.dev) — a self-hostable,
PocketBase-compatible Rust backend. Kept deliberately lean: password + email
OTP sign-in and a realtime notes list. See "What's not here" below for what
to add and how.

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
2. **`expo start`** — press `w` for web, or scan the QR code with Expo Go on
   a physical device.

Sign in with the seeded demo account — `demo@example.com` / `password123`.
The Cratebase dashboard is at <http://localhost:8090/_/> (superuser
credentials are printed in the terminal on first run).

**On a physical device or simulator**, `localhost` in `.env` means the
device itself, not your computer — set `EXPO_PUBLIC_CRATEBASE_URL` to your
computer's LAN IP (e.g. `http://192.168.1.23:8090`) instead.

## What's included

- **Auth** (`/sign-in`) — password and email OTP ("email code"), via
  `cb.auth.signIn.password`/`.otp` and `cb.auth.otp.request`.
- **`AsyncAuthStore`** ([`lib/cratebase.ts`](./lib/cratebase.ts)) — React
  Native's persistence API (`AsyncStorage`) is itself async, unlike the
  browser's synchronous `localStorage`; this is `@cratebase/client`'s store
  implementation for that.
- **Realtime notes list** (`/notes`, redirected to from `/` once signed in)
  — `useRecords("posts", { realtime: true })` refetches on every matching
  create/update/delete event, plus add/delete via `useMutation`.
- **`schema.json`** — the same `posts` collection as the other
  create-cratebase templates (title/content, both searchable; an optional
  `cover` file; an `owner` relation), scoped with
  `owner = @request.auth.id` rules.
- **`pb_seed/`** — a demo user and two notes, applied once on an empty
  database.
- **`pb_hooks/welcome.pb.js`** — emails the built-in `welcome` template to
  every new signup via `$mails.send`.

## What's not here (and is web-only, or needs extra native setup)

Kept out to keep this starter lean — all of it works from Expo too, it just
needs a bit more native wiring than fits a starter:

- **Magic link and OAuth** — both need a configured deep link (`scheme` in
  `app.json`, already set to `cratebaseapp://` here) and, for OAuth,
  `expo-web-browser`'s `openAuthSessionAsync` to open the provider's
  `authURL` (from `cb.auth.methods()`) and catch the redirect. See the
  `nextjs`/`vite-react` templates' sign-in screens for the web flow to
  adapt, and Expo's [`AuthSession` guide](https://docs.expo.dev/guides/authentication/).
- **TOTP** — `cb.auth.totp.setup()`/`.confirm()`/`signIn.totp()` are the
  same calls as the web templates; rendering the QR code needs a library
  like `react-native-qrcode-svg` instead of the web templates' `<img>`/
  `qrcode.react`.
- **File upload / cover images** — `cb.files.upload()` and
  `@cratebase/react`'s `useUpload()` work unchanged from Expo; pick an
  image with `expo-image-picker` and pass its `uri` where the web templates
  pass a `File`.
- **Full-text search and the notifications bell** — `?search=` and
  `useNotifications()` both work unchanged; they're just left off this
  screen for leanness. See the `nextjs` template's dashboard for a
  reference implementation of both.

## Deploying

Cratebase ships as a single binary — no separate database server required
for a small app (SQLite by default; point `DATABASE_URL` at Postgres for
more headroom).

- **Single binary behind a reverse proxy** — run `cratebase serve` on the
  box, put [Caddy](https://caddyserver.com) or nginx in front of it for
  TLS.
- **Docker** — see the
  [`docker-compose.yml`](https://github.com/cratebasehq/cratebase/blob/main/docker-compose.yml)
  in the Cratebase repo. Point `EXPO_PUBLIC_CRATEBASE_URL` at your deployed
  server, then `expo export --platform web` for a static web build, or
  `eas build` for native app store builds.
- Push `schema.json` to the deployed instance with `cratebase schema push`
  (or `POST /api/schema/apply`) as part of your deploy — see
  [Schema as code](https://cratebase.dev/docs/extending/schema-as-code/).

## Learn more

- [Cratebase docs](https://cratebase.dev/docs/) — start with
  [Getting started](https://cratebase.dev/docs/getting-started/install/).
- [`@cratebase/client`](https://github.com/cratebasehq/cratebase/tree/main/sdk/js/client#readme) —
  the typed SDK this app talks to Cratebase with.
- [`@cratebase/react`](https://github.com/cratebasehq/cratebase/tree/main/sdk/js/react#readme) —
  the hooks used in `app/notes.tsx`.
- [Expo Router docs](https://docs.expo.dev/router/introduction/).
