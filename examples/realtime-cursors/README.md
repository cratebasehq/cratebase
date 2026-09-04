# Realtime cursors

Every visitor's mouse position, live, as a colored labeled dot on everyone
else's screen — the smallest possible demo of Cratebase's realtime
(`GET /api/realtime` SSE stream + `POST /api/realtime` subscriptions) and
record CRUD together.

Plain HTML/CSS + one ES module (`app.js`). No build step, no framework —
Cratebase's API is byte-compatible with PocketBase v0.23+, so it imports
the official PocketBase JS SDK via a bare specifier:

```js
import PocketBase, { ClientResponseError } from "pocketbase";
```

resolved by an [import map](https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/script/type/importmap) in `index.html` pointing
`"pocketbase"` at the published package on esm.sh
(`https://esm.sh/pocketbase@0.28`) — no npm install and no bundler step.
One deliberate exception: the best-effort cursor cleanup on tab close
uses raw `fetch(..., {keepalive: true})` directly, since
`RecordService.delete` doesn't expose that fetch option — see the
comment above `deleteOwnCursor` in `app.js`.

## 1. Start Cratebase

From the repo root:

```bash
cargo run --bin cratebase -- serve
```

(`docker compose up -d` also works if you'd rather run it containerized —
just note its `cratebase` service uses a separate persisted volume from
the `cargo run` binary's local `data/` dir, so pick one and stick with it
per demo session, and stop the other before switching so they don't
fight over port 8090.)

This example assumes the API is reachable at `http://localhost:8090` (the
default). Serving `index.html` from a different origin is fine — CORS
defaults to `*` — but if the API isn't on `localhost:8090`, open the page
with `?api=http://your-host:port` appended.

## 2. Create the `cursors` collection

Run the setup script — it upserts an `admin@example.com` / `changeme123`
superuser (override with `ADMIN_EMAIL`/`ADMIN_PASSWORD` env vars) and
creates a `cursors` collection with fully public rules (`""` = anyone, no
auth needed) if it doesn't already exist. Safe to re-run.

```bash
bun run examples:cursors:setup
# or directly: bash examples/realtime-cursors/setup.sh
```

Every rule is `""` (public) — `createRule`/`updateRule`/`deleteRule` all
open too, since each client writes and deletes only *its own* record but
has no auth session to prove ownership with. Fine for a demo; a real
deployment would either require auth (`updateRule`/`deleteRule`:
`clientId = @request.auth.id`) or accept that this is intentionally an
open scratch collection.

## 3. Serve the example

```bash
bun run examples:serve
```

This serves the whole repo (not just this directory) — required because
`index.html`'s import map points `"cratebase"` at
`../../sdk/js/dist/index.js`, a path that only resolves when the server
is rooted above `examples/`. Serving just this directory (e.g. `cd
examples/realtime-cursors && python3 -m http.server`) 404s on that
import, which fails silently (bare specifier `"cratebase"` can't resolve
at all — the page loads but no cursor ever appears, guest or peer).

Open **`http://localhost:4173/examples/realtime-cursors/`** in two or
more browser tabs (or two different browsers/devices on the same
network, pointed at your machine's IP) and move the mouse around — each
tab renders every *other* tab's cursor live, labeled with a randomly
generated name and color. Your own real system cursor is never
duplicated with a rendered dot.

## How it works

- **Identity**: on first load, a random `clientId`, color, and label
  (e.g. "Swift Otter") are generated and persisted to `localStorage`, so
  reloading the page keeps being the same cursor instead of spawning a
  new one.
- **Throttled writes**: `mousemove` is captured on every event, but a
  `requestAnimationFrame` loop gated by a timestamp only flushes a
  position at most once per ~50ms, so a fast mouse doesn't spam the API.
- **Upsert**: the flush path tries `PATCH
  /collections/cursors/records/{id}` against the record id remembered in
  `localStorage`; if that id 404s (first visit, or a stale id from a
  cleared/expired session) it falls back to `POST
  .../records` and remembers the new id. A one-time unique-constraint
  recovery path also handles the case where a record for this `clientId`
  already exists server-side but the local id was lost.
- **Realtime**: on load the page opens `GET /api/realtime` (SSE), waits
  for the `PB_CONNECT` event to learn its `clientId`, then
  `POST /api/realtime` with `{"clientId", "subscriptions": ["cursors/*"]}`.
  Every subsequent `create`/`update`/`delete` event for any record in the
  collection arrives as an SSE `message` event and moves (or removes) the
  matching dot. Events for our own `clientId` are ignored — we already
  see our real system cursor.
- **Staleness**: a peer's dot fades after 6s without an update and is
  dropped entirely after 20s, covering tabs that vanish without a clean
  `beforeunload` (crash, killed process, network drop). A clean close
  also fires a best-effort `DELETE` (`keepalive: true`) so other tabs see
  it disappear immediately via the realtime `delete` event.

## Verification

- `bun run examples:cursors:setup` is verified live against a running
  server: it creates `cursors`, its unique `clientId` index and its rules,
  and re-running it is a no-op.
- **Not currently verified**: the browser half. It ran clean in headless
  Chromium at one point — identity, upsert, and subscribe all fired — but
  that was before `/api/realtime` was taken out of the router during the
  core rewrite. Until realtime lands again, `GET /api/realtime` 404s and
  no cursor, own or peer, will move.

**Fixed since the original version of this README**: `index.html` was
missing the import map entirely, so the bare specifier in `app.js` had
nothing to resolve to and the module failed to load at all — no cursors,
no realtime, silently. It now declares the same import map every other
`examples/*` app uses.
