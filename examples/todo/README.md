# Cratebase Todo Example

The comprehensive, single-page demo: create an account, sign in, then
manage a shared todo list (add, toggle done, delete, list) with realtime
sync — open the page in two browser tabs, check off or delete a todo in
one, and watch it update live in the other with zero polling. Everything
past "sign in" requires an authenticated session (the `todos` collection's
rules are `@request.auth.id != ""`), so this one page exercises the full
path: register → login → authenticated CRUD → realtime, end to end,
against the default `users` auth collection.

Every request the page makes to Cratebase — register, sign in, add a
todo, toggle it, delete it, the realtime subscribe call — flashes in a
small ticker pinned to the bottom of the page (`POST
/collections/todos/records · 201 · 8ms`), so none of this is a black box:
you see the literal HTTP call behind every click.

## Importing `cratebase` with zero build step

`index.html` declares an [import map](https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/script/type/importmap) mapping the bare
specifier to the already-built local package:

```html
<script type="importmap">
  { "imports": { "cratebase": "../../sdk/js/dist/index.js" } }
</script>
```

so `app.js` can just write:

```js
import { Cratebase } from "cratebase";
```

`cratebase` isn't published to npm yet, so `https://esm.sh/cratebase` (or
any other CDN-from-npm-registry URL) would 404 — the import map is what
makes the bare specifier resolve locally instead, no npm install and no
bundler step needed. Once `cratebase` is published to npm, swapping the
import map's one entry for a CDN URL (or dropping the import map
entirely and using a bundler) is a drop-in change — `app.js` doesn't
change at all.

## 1. Start Cratebase

From the repo root:

```bash
cargo run --bin cratebase -- serve
```

This serves the API at `http://localhost:8090` (the URL `app.js` is
hardcoded to point at — edit `BASE_URL` in `app.js` if you're running it
elsewhere).

## 2. Create the `todos` collection

Run the setup script — it upserts an `admin@example.com` / `changeme123`
superuser (override with `ADMIN_EMAIL`/`ADMIN_PASSWORD` env vars) and
creates (or updates) the `todos` collection, gated to signed-in users
only. Safe to re-run — it PATCHes the collection back in line if it
already exists with different rules/schema.

```bash
bun run examples:todo:setup
# or directly: bash examples/todo/setup.sh
```

Notes:

- All five rules are `"@request.auth.id != \"\""` — any signed-in user
  (not scoped to *your own* todos specifically) can list/view/create/
  update/delete, so it behaves like a shared team board once you're in.
  There's no ownership field; that's a deliberate simplification to keep
  the schema to two fields (`title`, `done`) — see "How it works" below
  for the exact rule enforcement this relies on.
- The `users` collection itself needs no setup — it ships with Cratebase
  by default (`identityField: "email"`), which is what `app.js`'s
  register/login forms authenticate against.
- Point at a different instance with `CRATEBASE_URL=http://host:port bun
  run examples:todo:setup`.

## 3. Serve the example

```bash
bun run examples:serve
```

This serves the whole repo (not just this directory) — required because
`index.html`'s import map points `"cratebase"` at
`../../sdk/js/dist/index.js`, a path that only resolves when the server
is rooted above `examples/`. Serving just this directory (e.g. `cd
examples/todo && python3 -m http.server`) 404s on that import; the
failure is silent in the UI (no JS runs, so every form falls back to a
native GET submit that reloads the page with your input stuck in the
URL's query string).

Then open **`http://localhost:4173/examples/todo/`**. Create an account
(email + password, 8 characters minimum), which registers and signs you
in immediately. Open the same URL in a second tab — sign in there too (or
register a second account) — and add/check/delete todos in one tab to
watch them sync live in the other.

## How it works

- **Register**: `users.create({ email, password, passwordConfirm })`
  then `users.authWithPassword(email, password)` — the same default
  `users` auth collection every Cratebase instance ships with.
- **Sign in**: `users.authWithPassword(identity, password)`. The SDK's
  `AuthStore` persists the resulting token/record to `localStorage` and
  fires `onChange`, which is what drives the guest-view ↔ todo-view
  swap (`cb.authStore.isValid`) — reloading the page while signed in
  skips straight to the todo list.
- **Auth-gated CRUD**: the `todos` collection's rules are
  `@request.auth.id != ""`. Since `cb.collection("todos")` shares the
  same `Cratebase` client (and therefore the same `authStore`) as
  `cb.collection("users")`, every `todos` request automatically carries
  the `Authorization: Bearer <token>` header once signed in — no manual
  header wiring in `app.js`. Signed-out requests to `todos` get rejected
  server-side by that rule.
- **List + realtime**: on sign-in, it fetches every todo sorted
  newest-first with `todos.getList(1, 200, { sort: "-created" })`, then
  calls `cb.realtime.subscribe("todos", callback)`, which opens an SSE
  connection to `/api/realtime`, waits for the `PB_CONNECT` event to get
  a `clientId`, and posts a subscription for the `todos` topic. Every
  subsequent `create`/`update`/`delete` event calls the callback, which
  inserts, updates, or removes the corresponding list item live — this
  is what makes the two-tab sync work. Signing out calls
  `cb.realtime.disconnect()` to close that connection.
- **Add/toggle/delete**: `todos.create({ title, done: false })`,
  `todos.update(id, { done })` (`PATCH`), `todos.delete(id)`. Each is
  applied optimistically in the acting tab and reconciled (or reverted,
  on failure) by the same realtime event every other tab receives —
  rendering is deduplicated by record `id`, so the acting tab never
  double-renders its own change.
- **Request ticker**: `app.js` wraps `window.fetch` once at load to log
  every call whose URL starts with `BASE_URL` — method, path, status,
  duration — into the pill at the bottom of the page. Purely
  observational; it never inspects or modifies request/response bodies.

## Verification status

Verified live end-to-end in a real browser (headless Chromium) against a
freshly seeded instance (`bun run examples:todo:setup`, `cargo run --bin
cratebase -- serve`, `bun run examples:serve`): registering an account
signs in immediately and reveals the todo view; adding a todo persists it
with no page reload and no console/network errors. Two-tab realtime
propagation and the sign-out → guest-view path were not re-verified after
the auth-gating rewrite — the realtime subscribe path itself was already
exercised by the same add flow (the list loads via the same
`connectRealtime()` call `main()` uses on sign-in).
