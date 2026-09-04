# Realtime cursors

Every visitor's mouse position, live, as a colored labeled dot on everyone
else's screen — the smallest possible demo of Cratebase's realtime
(`GET /api/realtime` SSE stream + `POST /api/realtime` subscriptions) and
record CRUD together.

Plain HTML/CSS + one ES module (`app.js`). No build step, no framework —
it imports the real `cratebase` JS SDK via a bare specifier:

```js
import { ClientResponseError, Cratebase } from "cratebase";
```

resolved by an [import map](https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/script/type/importmap) in `index.html` pointing
`"cratebase"` at the already-built local package
(`../../sdk/js/dist/index.js`) — `cratebase` isn't published to npm yet,
so a CDN import (`https://esm.sh/cratebase`) would 404, and the import
map is what makes the bare specifier resolve locally instead, with no
npm install and no bundler step. One deliberate exception: the
best-effort cursor cleanup on tab close uses raw `fetch(..., {keepalive:
true})` directly, since `RecordService.delete` doesn't expose that fetch
option — see the comment above `deleteOwnCursor` in `app.js`.

## 1. Start Cratebase

```bash
docker compose up -d
docker compose exec cratebase cratebase superuser create you@example.com yourpassword
```

(or, from source: `cargo build --release -p cratebase-server && ./target/release/cratebase superuser create you@example.com yourpassword && ./target/release/cratebase serve`)

This example assumes the API is reachable at `http://localhost:8090` (the
default). Serving `index.html` from a different origin is fine — CORS
defaults to `*` — but if the API isn't on `localhost:8090`, open the page
with `?api=http://your-host:port` appended.

## 2. Create the `cursors` collection

Get a superuser token, then create a `base` collection with fully public
rules (`""` = anyone, no auth needed) so the demo works with zero client
setup:

```bash
ADMIN_TOKEN=$(curl -s -X POST localhost:8090/api/admins/auth-with-password \
  -H 'content-type: application/json' \
  -d '{"email":"you@example.com","password":"yourpassword"}' | jq -r .token)

curl -X POST localhost:8090/api/collections \
  -H "authorization: Bearer $ADMIN_TOKEN" -H 'content-type: application/json' \
  -d '{
    "name": "cursors",
    "type": "base",
    "schema": [
      {"id": "f1", "name": "clientId", "type": "text", "required": true, "unique": true},
      {"id": "f2", "name": "x", "type": "number", "required": true},
      {"id": "f3", "name": "y", "type": "number", "required": true},
      {"id": "f4", "name": "color", "type": "text", "required": true},
      {"id": "f5", "name": "label", "type": "text"}
    ],
    "listRule": "",
    "viewRule": "",
    "createRule": "",
    "updateRule": "",
    "deleteRule": ""
  }'
```

Every rule is `""` (public) — `createRule`/`updateRule`/`deleteRule` all
open too, since each client writes and deletes only *its own* record but
has no auth session to prove ownership with. Fine for a demo; a real
deployment would either require auth (`updateRule`/`deleteRule`:
`clientId = @request.auth.id`) or accept that this is intentionally an
open scratch collection.

## 3. Serve the example

Any static file server works, e.g.:

```bash
cd examples/realtime-cursors
python3 -m http.server 5500
```

Open `http://localhost:5500` in two or more browser tabs (or two
different browsers/devices on the same network, pointed at your
machine's IP) and move the mouse around — each tab renders every *other*
tab's cursor live, labeled with a randomly generated name and color. Your
own real system cursor is never duplicated with a rendered dot.

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
  `POST /api/realtime` with `{"clientId", "subscriptions": ["cursors"]}`.
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

- `node --check app.js` passes (plain ES module syntax, no bundler
  needed).
- **Not verified**: actual multi-tab live behavior in a browser — this
  sandbox has no browser available. The realtime/CRUD flow was checked
  against `openapi.yaml`, `sdk/js/src/record-service.ts`, and
  `sdk/js/src/realtime.ts` line-by-line instead (same request shapes,
  same SSE event names, same subscribe-after-`PB_CONNECT` sequencing).
  Please smoke-test with two real browser tabs before relying on this
  as a finished demo.
