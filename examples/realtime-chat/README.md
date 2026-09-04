# Cratebase Realtime Chat Example

A single-page, no-build chat app that shows off Cratebase's realtime
subscriptions: open this page in two browser tabs, send a message in one,
and watch it appear instantly in the other with zero polling.

This example is intentionally **not** an auth demo — there's a plain
display-name field (stored in `localStorage`, no login) so the focus stays
on realtime record subscriptions. See `examples/auth-demo` for the auth
flows.

## Importing `pocketbase` with zero build step

Cratebase's API is byte-compatible with PocketBase v0.23+, so this example
uses the official PocketBase JS SDK straight off a CDN instead of a
bespoke client. `index.html` declares an [import map](https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/script/type/importmap)
mapping the bare specifier to the published package on esm.sh:

```html
<script type="importmap">
  { "imports": { "pocketbase": "https://esm.sh/pocketbase@0.28" } }
</script>
```

so `app.js` can just write:

```js
import PocketBase from "pocketbase";
```

No npm install and no bundler step needed. Swapping the import map's one
entry for a local `node_modules/pocketbase` resolution (or dropping the
import map entirely and using a bundler) is a drop-in change if you'd
rather not depend on a CDN — `app.js` doesn't change at all.

## 1. Start Cratebase

From the repo root:

```bash
cargo run --bin cratebase -- serve
```

This serves the API at `http://localhost:8090` (the URL `app.js` is
hardcoded to point at — edit `BASE_URL` in `app.js` if you're running it
elsewhere).

## 2. Create the `messages` collection

Get an admin token, then create a `messages` collection with `author` and
`content` text fields, and public list/view/create rules so the example
works with zero auth setup.

```bash
# 1. Authenticate as a superuser and capture the token. PocketBase v0.23+
#    dropped /api/admins/* in favour of the _superusers auth collection.
ADMIN_TOKEN=$(curl -s -X POST http://localhost:8090/api/collections/_superusers/auth-with-password \
  -H "Content-Type: application/json" \
  -d '{"identity":"admin@example.com","password":"your-admin-password"}' \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["token"])')

# 2. Create the collection. PocketBase v0.23+ uses "fields" (not "schema"),
#    and every collection needs its own id/created/updated fields spelled
#    out explicitly — the server only fills in auth columns for you.
curl -s -X POST http://localhost:8090/api/collections \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -d '{
    "name": "messages",
    "type": "base",
    "fields": [
      { "name": "author", "type": "text", "required": true },
      { "name": "content", "type": "text", "required": true },
      { "name": "created", "type": "autodate", "onCreate": true },
      { "name": "updated", "type": "autodate", "onCreate": true, "onUpdate": true }
    ],
    "listRule": "",
    "viewRule": "",
    "createRule": "",
    "updateRule": null,
    "deleteRule": null
  }'
```

Notes:

- `listRule`/`viewRule`/`createRule` set to `""` (empty string, not `null`)
  means "public, no auth required" in Cratebase's rule semantics — anyone
  can list, view, and create messages. `updateRule`/`deleteRule` are left
  `null` (superuser-only) since this example never edits or deletes
  messages.
- Swap in your real admin email/password from whatever seeded the instance
  you're running against.
- If your `cratebase` binary/admin bootstrap flow differs, adjust step 1
  accordingly — the important part is ending up with a superuser Bearer
  token for step 2.

## 3. Serve the example

Any static file server works, from this directory:

```bash
cd examples/realtime-chat
python3 -m http.server 8080
```

Then open `http://localhost:8080` in two browser tabs. Set a display name
in each tab, send a message from one, and it should appear in the other
tab immediately via the realtime SSE subscription — no page refresh, no
polling.

> Serving over `http://` (not `file://`) matters: browsers restrict ES
> module imports and `fetch`/`EventSource` calls from `file://` origins.

## How it works

- `app.js` creates one `PocketBase` client pointed at `http://localhost:8090`.
- On load, it fetches the most recent 50 messages with
  `cb.collection("messages").getList(1, 50, { sort: "created" })` and
  renders them.
- It then calls `cb.collection("messages").subscribe("*", callback)`, which
  opens an SSE connection to `/api/realtime`, waits for the `PB_CONNECT`
  event to get a `clientId`, and posts a subscription for the
  `messages/*` topic. Every subsequent `create`/`delete` event on that
  collection calls the callback, which appends or removes the
  corresponding chat bubble live.
- Sending a message is a plain `cb.collection("messages").create({ author,
  content })` — the sender doesn't render its own message from the create
  response; it relies on the realtime event to render it, exactly the same
  path every other connected tab uses, which is what proves the realtime
  path actually works end-to-end.

## Verification status

This was verified with static analysis only: `node --check app.js` passes.
**No live `cratebase serve` instance or browser was run in producing this
example** — the realtime subscribe/publish flow, the collection-creation
`curl` commands, and the rendered UI have not been exercised end-to-end.
Please run through steps 1–3 above once against a live instance to
confirm before relying on this in a demo.
