# Cratebase Todo Example

A single-page, no-build todo list that shows full CRUD (add, toggle done,
delete, list) plus realtime sync: open this page in two browser tabs, check
off or delete a todo in one, and watch it update live in the other with zero
polling — the whole point of building this on Cratebase instead of
`localStorage`.

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

Get an admin token, then create a `todos` collection with a required
`title` text field and a `done` bool field, and public list/view/create/
update/delete rules so the example works with zero auth setup.

```bash
# 1. Authenticate as an admin/superuser and capture the token.
ADMIN_TOKEN=$(curl -s -X POST http://localhost:8090/api/admins/auth-with-password \
  -H "Content-Type: application/json" \
  -d '{"email":"admin@example.com","password":"your-admin-password"}' \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["token"])')

# 2. Create the collection.
curl -s -X POST http://localhost:8090/api/collections \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -d '{
    "name": "todos",
    "type": "base",
    "schema": [
      { "id": "title", "name": "title", "type": "text", "required": true },
      { "id": "done", "name": "done", "type": "bool", "required": false }
    ],
    "listRule": "",
    "viewRule": "",
    "createRule": "",
    "updateRule": "",
    "deleteRule": ""
  }'
```

Notes:

- All five rules are set to `""` (empty string, not `null`) — that means
  "public, no auth required" in Cratebase's rule semantics. Unlike the
  chat example, this one needs a public `updateRule` (toggling `done` is a
  `PATCH`) and `deleteRule` (removing a todo), since there's no login flow
  here at all.
- Swap in your real admin email/password from whatever seeded the instance
  you're running against.
- If your `cratebase` binary/admin bootstrap flow differs, adjust step 1
  accordingly — the important part is ending up with a superuser Bearer
  token for step 2.

## 3. Serve the example

Any static file server works, from this directory:

```bash
cd examples/todo
python3 -m http.server 8080
```

Then open `http://localhost:8080` in two browser tabs. Add a todo, check it
off, or delete it in one tab — it should appear/update/disappear in the
other tab immediately via the realtime SSE subscription, no page refresh,
no polling.

> Serving over `http://` (not `file://`) matters: browsers restrict ES
> module imports and `fetch`/`EventSource` calls from `file://` origins.

## How it works

- `app.js` creates one `Cratebase` client pointed at `http://localhost:8090`.
- On load, it fetches every todo sorted newest-first with
  `cb.collection("todos").getList(1, 200, { sort: "-created" })` and
  renders each as a list item with a checkbox, title, and delete button.
- It then calls `cb.realtime.subscribe("todos", callback)`, which opens an
  SSE connection to `/api/realtime`, waits for the `PB_CONNECT` event to
  get a `clientId`, and posts a subscription for the `todos` topic. Every
  subsequent `create`/`update`/`delete` event on that collection calls the
  callback, which inserts, updates, or removes the corresponding list item
  live — this is what makes the two-tab sync work.
- Adding a todo is `cb.collection("todos").create({ title, done: false })`;
  the response is rendered immediately in the creating tab (optimistic),
  and other tabs pick it up from the realtime `create` event. Rendering is
  deduplicated by record `id`, so the creating tab doesn't get a duplicate
  row when its own realtime event arrives back.
- Toggling the checkbox does an inline `cb.collection("todos").update(id, {
  done })` (`PATCH`) — applied optimistically in the toggling tab, and
  reverted if the request fails. Other tabs update from the realtime
  `update` event.
- Deleting is `cb.collection("todos").delete(id)` — the row animates out
  (fade + collapse) in the deleting tab on success, and other tabs animate
  the same row out when the realtime `delete` event arrives.

## Verification status

This was verified with static analysis and a syntax/bundle check only:
`node --check app.js` and `bun build app.js --outdir /tmp/checkbuild` both
pass (the latter also confirms the relative import to
`sdk/js/dist/index.js` resolves correctly). **No live `cratebase serve`
instance or browser was run in producing this example** — the realtime
create/update/delete sync across tabs, the collection-creation `curl`
commands, and the rendered UI/animations have not been exercised
end-to-end. Please run through steps 1–3 above once against a live
instance to confirm before relying on this in a demo.
