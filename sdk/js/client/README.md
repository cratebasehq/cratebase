# @cratebase/client

The first-party typed TypeScript SDK for [Cratebase](https://github.com/cratebasehq/cratebase):
records, auth (password/OTP/magic-link/TOTP/OAuth2 (12 built-in presets + generic OIDC)/MFA,
linked accounts, cookie sessions, impersonation, bans), realtime, realtime channels + presence,
in-app/email/push notifications, files, batch, mail (`_emailTemplates` + `sendRule`), custom SQL
RPC, and admin APIs — plus the value-add surface (vector search, LLM chat, MCP tool schemas, the
durable job queue) built in, no separate package required.

Cratebase's wire is byte-compatible with PocketBase v0.23+, so the official
[`pocketbase`](https://www.npmjs.com/package/pocketbase) npm SDK still works unchanged against a
Cratebase server — see [`pocketbase-interop`](https://cratebase.dev/docs/sdk/pocketbase-interop/)
in the docs for that path. This package is the recommended one for new projects: a real generic
`Schema` type parameter (paired with `@cratebase/schema-codegen`), no positional-argument
ambiguity, no client-side request auto-cancellation to fight, and first-class coverage of every
Cratebase-only endpoint (sessions, bans, the server-driven OAuth2 redirect flow) that the
PocketBase-family SDKs have no surface for at all.

```
bun add @cratebase/client
```

```ts
import { createClient } from "@cratebase/client";

const cb = createClient("http://localhost:8090");

const posts = await cb.collection("posts").list({ filter: "published = true", search: "treasure" });
const unsubscribe = await cb.collection("posts").subscribe("*", (e) => console.log(e.action, e.record));

await cb.auth.signIn.password({ identity: "you@example.com", password: "..." });
```

Full documentation lives on the docs site under
[TypeScript SDK](https://cratebase.dev/docs/sdk/overview/): [`overview`](https://cratebase.dev/docs/sdk/overview/),
[`records`](https://cratebase.dev/docs/sdk/records/), [`auth`](https://cratebase.dev/docs/sdk/auth/),
[`realtime`](https://cratebase.dev/docs/sdk/realtime/), [`files`](https://cratebase.dev/docs/sdk/files/),
[`batch-and-admin`](https://cratebase.dev/docs/sdk/batch-and-admin/), and
[`ssr-and-cookies`](https://cratebase.dev/docs/sdk/ssr-and-cookies/).

## Shape

- `cb.collection(name)` — typed CRUD: `list`/`fullList`/`first`/`one`/`create`/`update`/`delete`/`subscribe`,
  object-options only. The `filter`/`raw` tagged templates build safe `filter=` expressions without
  hand-escaping user input. `list({ search })` runs a full-text query over the collection's
  `searchable` fields, ANDed onto `filter` (see [`full-text-search`](https://cratebase.dev/docs/database/full-text-search/)).
- `cb.auth` — the whole authentication surface for one auth collection (`cb.auth.as("_superusers")`
  for another): `signUp`, `signIn.password`/`.otp`/`.code`/`.social`/`.magicLink`/`.totp`, `signOut`
  (now server-revoking, not just a client-side store wipe), `refresh`, `sessions.*` (list/revoke),
  `admin.impersonate`/`.ban`/`.unban`/`.stopImpersonating`, MFA via a `401`+`mfaId` on the first
  factor, email verification, password reset, email change, `magicLink.request(...)`,
  `totp.setup`/`.confirm`/`.disable`/`.regenerateBackupCodes`, and `accounts.list()`/`.unlink(provider)`
  for a record's linked OAuth2 providers.
- `cb.mails.send(...)`/`.preview(...)` — `POST /api/mails/send`/`/preview`: send by `_emailTemplates`
  key (`{{var}}` data) or, as a superuser/API key, raw `subject`/`html`/`text`. A template with a
  `sendRule` can be sent by any caller that rule allows, restricted to `{to, template, data, locale}`.
- `cb.rpc<T>(name, params?)` — call a saved `_rpc` definition (`POST /api/rpc/{name}`); named
  parameters bind as real driver parameters, gated by that definition's own `rule`.
- `cb.realtime` / `cb.collection(x).subscribe(...)` — one shared SSE connection over `fetch` (no
  `EventSource`, no polyfill needed, works in Node/SSR too).
- `cb.channel(name)` — realtime channels not tied to any record: `.subscribe(handler)`,
  `.publish(event, data)`, `.presence.track(state)`/`.list()`/`.onChange(kind, member)`. Gated by a
  `_channels` row's `subscribeRule`/`publishRule` (no matching row disables it by default); a
  member is auto-removed when its connection drops or after ~45s with no heartbeat.
- `cb.notifications` — in-app/email/push in one call: `.send(...)` (superuser/API key),
  `.list`/`.fullList`/`.subscribe` (the ordinary records API against `_notifications`),
  `.unreadCount()`, `.markRead(id)`, `.markAllRead()`.
- `cb.files` — file URLs (`thumb`, independent `w`/`h`/`fit`/`format`/`q` image transforms,
  `download`, protected-file tokens) and `cb.files.upload(file, { collection, field })` — a
  presigned direct upload (straight to S3 or a same-origin local route, not through a multipart
  body) that returns a single-use token to pass as the field's value in the `create`/`update` that
  follows. `cb.batch()` runs several writes in one request/transaction.
- `cb.admin.*` — superuser-only management: collections, schema, settings, logs, backups, crons,
  storage, API keys, push, raw SQL.
- `cb.vector`, `cb.llm`, `cb.mcp`, `cb.queue` — the Cratebase-only value-add surface, built in (this
  is what `@cratebase/extras` bolts onto the `pocketbase` SDK for projects that stay on it).
  `cb.presence` also still works (a client-side, collection-backed presence pattern predating
  `cb.channel(name).presence` above) — prefer the channel-based one for anything new.

Pass a `Schema` generated by [`@cratebase/schema-codegen`](https://github.com/cratebasehq/cratebase/tree/main/tools/schema-codegen)
to `createClient<Schema>(url)` for `cb.collection("posts")` to type its records off your actual
collection schema instead of a bare `RecordModel`.

## Versioning

This package versions independently from the `cratebase` server binary and from
`@cratebase/extras` — it's the first-party client, not a build artifact of the Rust workspace.
Publishing is triggered by pushing a `client-v<version>` tag (see
`.github/workflows/publish-client.yml`), kept separate from the server's own `v*` tags and from
`extras-v*` so none of the three release trains block on each other.

## Development

```
bun install
bun run typecheck
bun run build   # emit dist/
```
