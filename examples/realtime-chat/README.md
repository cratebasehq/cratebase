# Cratebase Realtime Chat Example

A single-page, no-build chat app that shows off Cratebase's realtime
subscriptions: open this page in two browser tabs, send a message in one,
and watch it appear instantly in the other with zero polling.

This example is intentionally **not** an auth demo — there's a plain
display-name field (stored in `localStorage`, no login) so the focus stays
on realtime record subscriptions. See `examples/todo` for a real
register/login-gated flow.

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

## 2. Create the `messages` collection

Run the setup script — it upserts an `admin@example.com` / `changeme123`
superuser (override with `ADMIN_EMAIL`/`ADMIN_PASSWORD` env vars) and
creates the `messages` collection with public list/view/create rules
(`updateRule`/`deleteRule` stay superuser-only, since this example never
edits or deletes messages) if it doesn't already exist. Safe to re-run.

```bash
bun run examples:chat:setup
# or directly: bash examples/realtime-chat/setup.sh
```

Point at a different instance with `CRATEBASE_URL=http://host:port bun
run examples:chat:setup`.

## 3. Serve the example

```bash
bun run examples:serve
```

This serves the whole repo (not just this directory) — required because
`index.html`'s import map points `"cratebase"` at
`../../sdk/js/dist/index.js`, a path that only resolves when the server
is rooted above `examples/`. Serving just this directory (e.g. `cd
examples/realtime-chat && python3 -m http.server`) 404s on that import;
the failure is silent in the UI (no JS runs, so the composer form falls
back to a native GET submit that reloads the page with your message
stuck in the URL's query string).

Then open **`http://localhost:4173/examples/realtime-chat/`** in two
browser tabs. Set a display name in each tab, send a message from one,
and it should appear in the other tab immediately via the realtime SSE
subscription — no page refresh, no polling.

## How it works

- `app.js` creates one `Cratebase` client pointed at `http://localhost:8090`.
- On load, it fetches the most recent 50 messages with
  `cb.collection("messages").getList(1, 50, { sort: "created" })` and
  renders them.
- It then calls `cb.realtime.subscribe("messages", callback)`, which opens
  an SSE connection to `/api/realtime`, waits for the `PB_CONNECT` event to
  get a `clientId`, and posts a subscription for the `messages` topic.
  Every subsequent `create`/`delete` event on that collection calls the
  callback, which appends or removes the corresponding chat bubble live.
- Sending a message is a plain `cb.collection("messages").create({ author,
  content })` — the sender doesn't render its own message from the create
  response; it relies on the realtime event to render it, exactly the same
  path every other connected tab uses, which is what proves the realtime
  path actually works end-to-end.

## Verification status

Verified live: `bun run examples:chat:setup` against a fresh `cargo run
--bin cratebase -- serve` instance, served via `bun run examples:serve`,
driven in a real browser (headless Chromium) — submitting the composer
form appended a message with no page reload and no console/network
errors. Two-tab live delivery was not re-verified after this pass; the
sender's own render comes back through the same `connectRealtime()` SSE
path a second tab would use.
