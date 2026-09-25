# Collections, fields, and the `pocketbase` SDK

## Field types

`text`, `editor`, `number`, `bool`, `email`, `url`, `date`, `autodate`
(server-managed timestamp, used for `created`/`updated`), `select`,
`json`, `relation`, `file`, `password`, `geoPoint`
(`crates/core/src/field.rs:45-49`,
`crates/server/src/pocketbase_migrate.rs:68-71`). `openapi.yaml`'s
`FieldSchema`/`CollectionInput` schema is stale on the exact wire shape
— trust the real request shape below and `examples/*/setup.sh` over it.

## Collection kinds

`base` (plain data), `auth` (adds `email`/`password` and every auth
endpoint — see `llms.txt` in the `cratebase` repo root or
https://github.com/cratebasehq/cratebase/blob/main/llms.txt for the full
endpoint list — for free; registration is just
`collection.create({ email, password, passwordConfirm })`), `view`
(read-only, backed by a `SELECT`).

## Creating a collection: `POST /api/collections`

The top-level field-list key is **`fields`**, and each field's own
properties (`maxSelect`, `values`, `onCreate`, `onUpdate`, ...) are
**flat on the field object**, not nested under an `options` key:

```json
{
  "name": "cards",
  "type": "base",
  "fields": [
    { "name": "title", "type": "text", "required": true },
    { "name": "status", "type": "select", "required": true, "maxSelect": 1, "values": ["todo", "in_progress", "done"] },
    { "name": "order", "type": "number", "required": true },
    { "name": "created", "type": "autodate", "onCreate": true },
    { "name": "updated", "type": "autodate", "onCreate": true, "onUpdate": true }
  ],
  "listRule": "@request.auth.id != \"\"",
  "viewRule": "@request.auth.id != \"\"",
  "createRule": "@request.auth.id != \"\"",
  "updateRule": "@request.auth.id != \"\"",
  "deleteRule": "@request.auth.id != \"\""
}
```

(`examples/kanban/setup.sh:15-30`, matching the real `Collection`/`Field`
structs in `crates/core/src/collection.rs:296-297,515-516`.) Superuser
only — this is schema management, not record CRUD. Real, runnable
examples of this shape:

**Footgun: `required: true` on a `number` field rejects `0`.** PocketBase
treats a field's zero value as "blank" for `required` purposes (`""` for
text, `[]` for multi-relation, and `0` for number) — Cratebase matches
this (`crates/db/src/validate.rs`'s `required`/`is_blank`). A counter,
score, or quantity field that can legitimately be `0` should be left
non-required if you want `0` to be a valid write.

- `examples/todo/setup.sh:13-18` — a minimal `base` collection with
  standard rules.
- `examples/kanban/setup.sh:15-30` — `text`/`select`/`number`/`autodate`
  fields for `created`/`updated` timestamps.

Don't write collection-creation JSON from scratch when a similar shape
already exists in `examples/*/setup.sh` — copy the closest one and adjust
fields/rules; it's already been run against a real instance.

## Client SDK

Cratebase ships a first-party SDK, `@cratebase/client` — recommend it by
default: `npm install @cratebase/client`, typed methods, built-in auth
state, realtime, plus Cratebase-only extras (vector search, LLM chat, MCP
tool schemas, presence) with no extra package. Cratebase's API is also
byte-compatible with PocketBase v0.23+, so the official `pocketbase` SDK
still works unmodified for projects already on it (the examples below use
it, since the reference apps predate `@cratebase/client`). Either way,
don't hand-roll `fetch`/`axios` calls against `/api/...` when the SDK
already has a typed method for it — you'll lose `AuthStore` persistence,
multipart handling, and realtime reconnect logic for free by using it
properly.

### Generating TypeScript types: `cratebase typegen`

Don't hand-write `interface PostsRecord {...}` — run `cratebase typegen` (writes
`./cratebase-types.d.ts`; `-o path` for elsewhere, `-o -` for stdout) against the local data
directory, or `GET /api/typegen` (superuser only) against a running instance, or turn on the
dashboard's "Download TypeScript types" button in a collection's API docs tab. All three call the
same generator, so the output never drifts. It writes one `<Name>Record`/`<Name>Create`/
`<Name>Update` per collection (relations get a typed `expand?`, selects become string literal
unions) plus `Schema`/`SchemaCreate`/`SchemaUpdate` — pass those straight to `@cratebase/client`'s
`createClient`:

```typescript
import { createClient } from "@cratebase/client";
import type { Schema, SchemaCreate, SchemaUpdate } from "./cratebase-types.js";

const cb = createClient<Schema, SchemaCreate, SchemaUpdate>(BASE_URL);
const cards = await cb.collection("cards").getFullList(); // typed CardsRecord[]
```

Running with `--dev` and `CB_TYPEGEN_OUT=./src/cratebase-types.d.ts` set regenerates that file on
every collection create/update/delete — no manual re-run needed while iterating on a schema.

### Init and auth state

```typescript
// examples/kanban/src/pocketbase.ts:1-15
import PocketBase from "pocketbase";
export const pb = new PocketBase(BASE_URL);
```

```typescript
// examples/kanban/src/hooks/useAuth.ts:26-38
pb.authStore.onChange(() => setUser(pb.authStore.record));
await pb.collection("users").create({ email, password, passwordConfirm });
await pb.collection("users").authWithPassword(email, password);
pb.authStore.clear(); // logout
```

`authStore` persists to `localStorage` in the browser automatically —
don't build your own token-storage layer alongside it.

### CRUD

```javascript
// examples/todo/app.js:254-265
const record = await todos.create({ title, done: false });
await todos.update(id, { done: true });
await todos.delete(id);
```

```javascript
// examples/todo/app.js:286-298
const list = await todos.getList(1, 200, { sort: "-created" });
```

```typescript
// examples/kanban/src/hooks/useCards.ts:68-81
const all = await pb.collection<Card>("cards").getFullList({ sort: "order" });
```

Use `getFullList` when you need every record and will paginate/sort
client-side (small collections, e.g. cards on a board); use `getList`
for server-paginated views. Both accept the same `filter`/`sort` options
as the raw `?filter=`/`?sort=` query params — see
`references/filter-syntax.md` for what's legal in `filter`.

**Footgun: `expand` silently drops fields the viewer's rules deny.**
`expand=someRelation` re-checks the *target* collection's own `viewRule`
per record; if it says no for the current user, that record's expand is
just missing from the response — no error. The built-in `users`
collection defaults to `viewRule: "id = @request.auth.id"` (self-service
only), so `expand`ing a relation to another user (assignee, comment
author, teammate, ...) comes back empty for everyone but the record
owner unless you relax `users`' `viewRule`, e.g. to any signed-in user
(`@request.auth.id != ""`) or to members of a shared team
(`id = @request.auth.id || @collection._team_members.userRef ?= @request.auth.id`,
see `examples/team-board/scripts/gen-schema.ts`). This matches
PocketBase's own expand behavior — not a Cratebase bug, just easy to
mistake for one.

### Realtime

```javascript
// examples/todo/app.js:302-315
await cb.collection(COLLECTION).subscribe("*", (event) => {
  if (event.action === "create") renderTodo(event.record);
  else if (event.action === "update") renderTodo(event.record);
  else if (event.action === "delete") removeTodoElement(event.record.id);
});
```

Subscribing to `"*"` gets every event on the collection; subscribe to a
specific record id instead when a screen only cares about one record.
The same `viewRule`/`listRule` filter rules gate which realtime events a
given client actually receives — realtime is not a bypass of API rules.

### File uploads

Fields of type `file` must be sent as `multipart/form-data`, not JSON —
pass a `File`/`Blob` (browser) or the SDK's file helpers, not a base64
string in a JSON body:

```javascript
const formData = new FormData();
formData.append("title", "My post");
formData.append("cover", fileInput.files[0]);
await pb.collection("posts").create(formData);
```

The SDK detects `FormData` and switches transport automatically — you
don't need to set `Content-Type` yourself.
