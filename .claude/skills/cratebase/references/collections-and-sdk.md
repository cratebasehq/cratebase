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

### Realtime channels & presence (not tied to a record)

A channel is a topic that exists only because you configured it — no
collection, no `listRule`. Create a `_channels` row first (superuser
only, in the dashboard under Automation → Realtime channels, or via the
records API), or every publish/subscribe/presence call on that name is
refused (secure default: no matching row = disabled):

```javascript
await cb.collection("_channels").create({
  name: "room:*", // exact name, or a prefix pattern ending in "*"
  subscribeRule: "@request.auth.id != ''", // null = superusers only, "" = anyone
  publishRule: "@request.auth.id != ''",
});

const room = cb.channel("room:" + roomId);
const unsubscribe = await room.subscribe((msg) => console.log(msg.event, msg.data));
await room.publish("chat", { text: "hi", from: userId });

// Presence: who's here right now, auto-removed when their connection drops.
await room.presence.track({ name: userName, cursor: [x, y] });
const members = await room.presence.list();
const stop = await room.presence.onChange((kind, member) => {
  // kind: "join" | "update" | "leave"
});
```

React: `useChannel("room:" + roomId, { onMessage })` and
`usePresence("room:" + roomId, state)` from `@cratebase/react` wrap the
above with subscribe-on-mount/unsubscribe-on-unmount and a heartbeat
interval. A prefix pattern's matched suffix is available to rules as
`@request.data.suffix` (and the full name as `@request.data.channel`) —
e.g. `subscribeRule: "@request.data.suffix = @request.auth.id"` for a
private per-user channel `"user:*"`. From a JS hook, `$realtime.publish(name, event, data)`
runs at the trusted tier (no `publishRule` check, same as `$app.save`).

### Notifications (in-app + email + push, one call)

```javascript
// pb_hooks/*.pb.js — after creating a comment, say
$notify.send({
  to: post.get("authorId"),
  type: "comment",
  title: "New comment",
  body: `${author.name} replied to your post`,
  link: `/posts/${post.id}`,
  // channels defaults to ["inapp", "email", "push"]; narrow it to skip some
});
```

```javascript
const { items } = await cb.notifications.list({ sort: "-created" });
const unread = await cb.notifications.unreadCount();
await cb.notifications.markRead(notificationId);
await cb.notifications.markAllRead();
```

`_notifications` is an ordinary system collection (`owner_rule`-scoped —
`list`/`view`/`delete` are the caller's own rows, `update` only ever
accepts `readAt`), so `cb.notifications.subscribe(handler)` or
`useNotifications()` (React) just works. Sending
(`$notify.send`/`POST /api/notifications/send`) is superuser/API-key
only — email renders the `notification` `_emailTemplates` row, push
goes to the recipient's `_push_subscriptions` rows, both best-effort
(a recipient missing an email/subscription is silently skipped for that
channel only).

### Full-text search

Mark a `text`/`editor`/`email`/`url` field `"searchable": true` and the
collection's records list accepts `?search=` (or the equivalent
`search("query")` filter/rule predicate), ANDed onto `filter`, ranked by
relevance (SQLite FTS5 `bm25`, Postgres GIN `ts_rank`) unless an explicit
`sort` is given:

```javascript
await cb.collection("posts").getList(1, 30, { search: "treasure map" });
```

A collection with no searchable field rejects `search`/`?search=` with a
`400` — it is not silently ignored. Postgres additionally reads the
collection's `searchLanguage` (e.g. `"english"`) for stemming; SQLite
always does plain token/prefix matching (`term*` prefix, `"exact
phrase"` quoting; no boolean `OR`/`NOT` operators — see
`site/src/content/docs/docs/database/full-text-search.mdx` for the full
query grammar and the SQLite-vs-Postgres performance numbers). Both
index types stay in sync automatically on any schema change that
touches the searchable field set, the collection name, or
`searchLanguage` — nothing to run by hand after editing the schema.

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

For a large file, prefer a **direct upload** instead — the bytes go
straight to storage rather than through this multipart body:

```javascript
const upload = await cb.files.upload(file, { collection: "posts", field: "cover" });
await cb.collection("posts").create({ id: upload.recordId, title: "My post", cover: upload.token });
```

`upload.recordId` **must** be reused as the new record's own `id` — the
token is reserved against it. `@cratebase/react`'s `useUpload()` hook
wraps this with `progress`/`pending`/`error` state. Optionally metered by
`settings.storage.userQuotaBytes` (per auth record, only for collections
with `ownerField` set) — exceeding it fails the upload with a `400`
before any bytes move.

An image file also accepts on-the-fly transforms on download, beyond the
existing `?thumb=WxH`: `?w=&h=&fit=cover|contain|inside&format=jpeg|png|webp&q=`,
cached the same way a thumbnail is. Gated by
`settings.storage.imageTransformsEnabled` (default on); dimensions are
clamped to `settings.storage.maxTransformDimension` for non-superuser
requests rather than rejected.

### Custom SQL RPC and Postgres extensions

For a query a `filter`/`sort` genuinely can't express (a `GROUP BY`
report, a PostGIS nearest-store lookup, a `pg_trgm` fuzzy search), save a
named SQL definition to the `_rpc` collection — superuser only, same
tier as a collection schema edit — and call it from the SDK:

```javascript
const { items } = await cb.rpc("nearest_stores", { lon: -122.42, lat: 37.77, radiusKm: 5 });
```

`_rpc.sql` uses `:name`-style placeholders declared in `_rpc.params`
(`{name, type, required, default}`); every value is bound as a real
driver parameter, never string-interpolated. `_rpc.rule` (evaluated
against `@request.auth`/`@request.body`, i.e. the call's own params)
decides who may call it: `null` = superuser only (the default),
`""` = anyone, otherwise a normal filter expression. `readOnly` defaults
`true` and is enforced by the database itself, not just a text check.
Prefer a **view collection** instead when the result is naturally a list
of records a client will further `filter`/`sort`/paginate — RPC is for
when the parameters shape the query itself, or the result isn't
record-shaped at all.

A Postgres extension (`postgis`, `pgvector`, `pg_trgm`, ...) is enabled
via `POST /api/db/extensions/{name}` (superuser only, Postgres only) or
the dashboard's Settings → Database extensions page; version it from a
migration with `$app.db().exec("CREATE EXTENSION IF NOT EXISTS postgis")`.
See `site/src/content/docs/docs/database/` for the full guides.

#### RPC recipe: nearest-location query

`geoDistance(lonField, latField, lon, lat)` is a first-class filter/sort
function — for a collection with a `geoPoint` field (stored as
`{lon, lat}`), the simplest "find nearby" query needs no RPC at all:

```javascript
const nearby = await cb.collection("stores").getList(1, 20, {
  filter: `geoDistance(location.lon, location.lat, ${lon}, ${lat}) < ${radiusKm}`,
  sort: `geoDistance(location.lon, location.lat, ${lon}, ${lat})`, // nearest first
});
```

On Postgres, once `postgis` is enabled this automatically compiles to a
GiST-indexed `ST_DWithin`/KNN `<->` query instead of the portable
haversine calculation — same syntax, no client change. Reach for an
`_rpc` definition instead when you want a **fixed, rule-gated shape**
(the caller shouldn't be able to pass arbitrary `filter`/`sort` strings)
or the query needs a `JOIN`/aggregate a plain list can't express:

```sql
-- _rpc row: name = "nearest_stores"
SELECT id, name, geoDistance(location.lon, location.lat, :lon, :lat) AS "distanceKm"
FROM stores
WHERE geoDistance(location.lon, location.lat, :lon, :lat) < :radiusKm
ORDER BY "distanceKm"
LIMIT 20
```

```javascript
const { items } = await cb.rpc("nearest_stores", { lon: -122.42, lat: 37.77, radiusKm: 5 });
```

`_rpc.params` would declare `lon`/`lat`/`radiusKm` as `number`, and
`_rpc.rule` as `""` if any signed-out visitor may call it, or a filter
expression otherwise.

### Email: sending from a frontend, magic-link + TOTP login, triggers

**Recipe: sending an email straight from a frontend (`sendRule`).**
`POST /api/mails/send` is superuser/API-key only by default, but an
`_emailTemplates` row with a non-`null` `sendRule` opens itself to a
non-superuser caller — an authenticated app user, or even an anonymous
one — with no backend route of your own:

```javascript
// _emailTemplates row: key = "invite", sendRule = "@request.auth.id != ''"
await cb.mails.send({
  template: "invite",
  to: { address: "friend@example.com" },
  data: { inviterName: user.name, teamName: team.name },
});
```

A non-superuser send is restricted to `{to, template, data, locale}` —
no raw `subject`/`html`/`text`, no `from`/`cc`/`bcc`/`replyTo` override,
capped at 5 recipients — and `sendRule` is evaluated once per `to`
address against `@request.auth.*`/`@request.body.{to,data,locale}` (no
bare field reference; there's no "record" a send is about). `null`
(the default) keeps a template superuser-only; `""` opens it to anyone.
Prefer this over a custom `pb_hooks` route whenever the check is
expressible as a rule — it's strictly less code to maintain.

**Recipe: email triggers (no code, fires on a record write).** For
"email the assignee when a card is assigned" or "send a receipt on
order create," a `_emailTriggers` row (superuser-managed, same shape as
`_webhooks`) needs no hook file at all:

```json
{
  "collection": "orders",
  "event": "create",
  "template": "order-receipt",
  "toField": "customerEmail",
  "condition": "status = 'paid'",
  "dataMap": { "supportUrl": "https://example.com/support" }
}
```

`toField` is a dotted path into the written record (or a literal address
if that path doesn't resolve); `condition` is a filter expression
evaluated against the record's bare fields (no `@request.*`); `dataMap`
merges onto the default `{ record }` template data. It fires from an
after-success hook and can never fail the triggering request — a
`toField` that resolves to nothing usable is logged to `_mailLog` as a
failure instead of raising an error. Reach for `$mails.send(...)` in a
`pb_hooks/*.pb.js` hook instead when the trigger needs logic `condition`/
`dataMap` can't express (calling another API first, computed content).

**Recipe: magic-link + TOTP login.** Enable magic links on an auth
collection (`authOptions.magicLink.enabled`, default `false`), then:

```javascript
await cb.auth.magicLink.request({ email, redirectUrl: "https://app.example.com/auth/magic-link" });
// user clicks the emailed link → your app's /auth/magic-link route:
const token = getMagicLinkTokenFromUrl(); // reads ?token= (or useMagicLinkCallback in React)
await cb.auth.signIn.magicLink({ token });
```

If the signed-in record also has TOTP confirmed (independently of
`authOptions.mfa`), any successful first factor — password, OTP, *or*
magic link — comes back `401 { mfaId }` instead of a session, and a
second call finishes it:

```javascript
const setup = await cb.auth.totp.setup(); // { qrUri, secret } — render qrUri with qrcode.react
await cb.auth.totp.confirm({ code }); // enables TOTP, returns 10 one-time backup codes shown once
// ...next login:
try {
  await cb.auth.signIn.password({ identity, password });
} catch (err) {
  if (err.mfaId) await cb.auth.signIn.totp({ mfaId: err.mfaId, code: totpCodeFromUser });
}
```

`useMagicLinkCallback` (`@cratebase/react`) wraps the whole
magic-link-callback dance (read the token, sign in, strip the param from
the URL) as one hook — see `sdk/js/react/README.md` in the `cratebase`
repo.
