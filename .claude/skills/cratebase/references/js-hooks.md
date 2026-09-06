# JS hooks (`pb_hooks/*.pb.js`)

Drop a `.pb.js` file in `pb_hooks/` next to the data directory. It runs
in an embedded QuickJS runtime (`crates/jsvm`) — no separate process, no
restart-to-reload; edit the file and it's live on the next request.
Reach for this when the logic can't be expressed as a filter-expression
API rule (see `references/filter-syntax.md`): side effects, calling
another API, computed/derived fields, custom endpoints, or scheduled
jobs. Type definitions for everything below live at
`crates/jsvm/src/types.d.ts`; the runtime globals are implemented in
`crates/jsvm/src/prelude.js` on top of a native bridge
(`crates/jsvm/src/bridge.rs`, `crates/server/src/jsvm_host.rs`).

## Lifecycle hooks

**Record hooks** — lifecycle (fire around the actual DB write):

```
onRecordEnrich onRecordValidate
onRecordCreate onRecordCreateExecute onRecordAfterCreateSuccess onRecordAfterCreateError
onRecordUpdate onRecordUpdateExecute onRecordAfterUpdateSuccess onRecordAfterUpdateError
onRecordDelete onRecordDeleteExecute onRecordAfterDeleteSuccess onRecordAfterDeleteError
```

**Record hooks** — request-side (fire around the HTTP request, before
the lifecycle ones):

```
onRecordCreateRequest onRecordUpdateRequest onRecordDeleteRequest
onRecordListRequest onRecordViewRequest
```

**Record hooks** — auth-specific:

```
onRecordAuthRequest onRecordAuthWithPasswordRequest onRecordAuthWithOAuth2Request
onRecordAuthWithOTPRequest onRecordAuthRefreshRequest onRecordRequestOTPRequest
onRecordRequestPasswordResetRequest onRecordConfirmPasswordResetRequest
onRecordRequestVerificationRequest onRecordConfirmVerificationRequest
onRecordRequestEmailChangeRequest onRecordConfirmEmailChangeRequest
```

**Collection hooks** (same create/update/delete/execute/after-success/
after-error pattern, plus request-side list/view/import):

```
onCollectionValidate
onCollectionCreate onCollectionCreateExecute onCollectionAfterCreateSuccess onCollectionAfterCreateError
onCollectionUpdate onCollectionUpdateExecute onCollectionAfterUpdateSuccess onCollectionAfterUpdateError
onCollectionDelete onCollectionDeleteExecute onCollectionAfterDeleteSuccess onCollectionAfterDeleteError
onCollectionCreateRequest onCollectionUpdateRequest onCollectionDeleteRequest
onCollectionsListRequest onCollectionViewRequest onCollectionsImportRequest
```

**Mailer hooks** (intercept/customize outgoing transactional email):

```
onMailerSend onMailerRecordVerificationSend onMailerRecordPasswordResetSend
onMailerRecordEmailChangeSend onMailerRecordOTPSend onMailerRecordAuthAlertSend
```

**Everything else:**

```
onBootstrap onServe onTerminate onBackupCreate onBackupRestore
onSettingsListRequest onSettingsUpdateRequest onSettingsReload
onRealtimeConnectRequest onRealtimeSubscribeRequest onRealtimeMessageSend
onFileDownloadRequest onFileTokenRequest
onBatchRequest
```

Pick the narrowest hook that covers the need: prefer an
`*AfterCreateSuccess`-style hook for side effects that shouldn't block or
roll back the write (logging, kicking off async work), and a
non-`Execute` lifecycle hook (`onRecordCreate`) when you need to
validate/mutate the record *before* it's persisted, inside the same
transaction.

## `routerAdd` / `cronAdd`

```
routerAdd(method, path, handler, ...middlewares)
```
Registers a custom HTTP route (`GET`/`POST`/`PUT`/`PATCH`/`DELETE`/
`HEAD`/`OPTIONS`). Use `$apis.requireAuth(...collections)` or
`$apis.requireSuperuserAuth()` as a middleware when the route needs auth
— don't reimplement token checking by hand.

```
cronAdd(id, cronExpr, handler)
```
Schedules `handler` on a croner-compatible cron expression (seconds
field ignored — minute-level granularity, per
`crates/server/src/cron.rs`). `handler` is `() => void` with access to
`$app` and the other globals, not a `RequestEvent`.

```
routerUse(...middlewares)
```
Registers global middleware across all custom routes.

## Request/response context

**`e.request`**: `method`, `headers`, `url.path`, `url.query()`,
`pathValue(name)`, `header.get(name)`, `body` (parsed JSON), `rawBody`
(exact UTF-8 bytes — use this, not `body`, when verifying a webhook
signature), `remoteAddr`.

**`e.response`**: `header(name, value?)`, `headers`.

**`e`** (the event object itself): `auth` (authenticated `Record` or
`null`), `hasSuperuserAuth()`, `realIP()`, `requestInfo()` → `{ method,
query, headers, body, auth, context }`; response helpers `json(status,
body)`, `string(status, text)`, `html(status, html)`, `blob(status,
contentType, bytes)`, `redirect(status, url)`, `noContent(status?)`,
`error(status, message?, data?)` plus typed shortcuts
(`badRequestError`, `notFoundError`, `forbiddenError`,
`unauthorizedError`, `tooManyRequestsError`, `internalServerError`); and
per-request scratch storage `get(key)`/`set(key, value)`.

**`RecordEvent`** (passed to record lifecycle hooks): `e.record`,
`e.collection`, `e.app` (transaction-scoped for `*Execute` hooks — writes
through this `$app` handle are part of the same transaction as the
record write), `e.next()` to continue the hook chain.

## Globals available inside hooks

- **`$app`** — the main database/collection/record surface:
  `settings()`, `findCollectionByNameOrId(name)`,
  `findRecordById(coll, id)`,
  `findRecordsByFilter(coll, filter, sort?, limit?, offset?, params?)`,
  `findFirstRecordByFilter(coll, filter, params?)`,
  `findFirstRecordByData(coll, key, value)`,
  `findAllRecords(coll, ...expressions)`,
  `findAuthRecordByEmail(coll, email)`, `countRecords(coll, ...expressions)`,
  `save(record|collection)`, `saveNoValidate(record|collection)`,
  `delete(record|collection)`, `runInTransaction(fn)`, `newMailClient()`,
  `logger()`, `store()` (process-wide KV), `cron()`, `dao()`,
  `isBootstrapped()`, `isDev()`.
  The `findRecordsByFilter`/`findFirstRecordByFilter`/`countRecords`
  filter argument is the same filter-expression language as API rules —
  see `references/filter-syntax.md`.
- **`$http`** — outbound HTTP: `send({ url, method?, body?, headers?,
  timeout? })` → `{ statusCode, headers, raw, body, json, cookies }`.
- **`$os`** — `getenv(name)`, `readFile(path)`, `writeFile(path, data)`,
  `exists(path)`, `tempDir()`, `getwd()`, `args()`, `exit(code?)`.
- **`$security`** — random (`randomString`, `randomStringWithAlphabet`,
  `pseudorandomString`, `pseudorandomStringWithAlphabet`), hashing
  (`sha256`, `sha512`, `sha1`, `md5`, `hs256`, `hs512`), JWT
  (`createJWT(payload, key, secondsDuration?, alg?)`,
  `parseUnverifiedJWT(token)`, `parseJWT(token, key)`), cipher
  (`encrypt`, `decrypt`, `equal`).
- **`$tokens`** — mint record-scoped tokens: `recordAuthToken(app,
  record)`, `recordVerifyToken`, `recordResetPasswordToken`,
  `recordChangeEmailToken`, `recordFileToken`.
- **`$mails`** — trigger the built-in transactional emails:
  `sendRecordVerification(app, record)`, `sendRecordPasswordReset`,
  `sendRecordChangeEmail`, `sendRecordOTP`.
- **`$filesystem`** — build file markers to attach to a `file` field:
  `fileFromPath(path, name?)`, `fileFromBytes(bytes, name?)`,
  `fileFromURL(url, name?)`.
- **`$apis`** — middleware (`requireAuth(...collections)`,
  `requireSuperuserAuth()`, `requireSuperuserOrOwnerAuth(pathParam?)`,
  `requireGuestOnly()`, `gzip()`, `bodyLimit(bytes?)`,
  `skipSuccessActivityLog()`) and response helpers
  (`enrichRecord(e, record)`, `enrichRecords(e, records)`,
  `toApiError(err)`).
- **`$dbx`** — raw database expressions for advanced queries: `exp(sql,
  params?)`, `hashExp(pairs)`, `and(...)`, `or(...)`, `not(expr)`,
  `in(col, ...values)`, `notIn`, `like`, `notLike`, `between(col, from,
  to)`.

## Real examples

`examples/docmind/pb_hooks/docmind.pb.js` is the most complete example
in the repo (RAG/vector-search app) and worth reading end to end:

- `:82-150` — `onRecordAfterCreateSuccess`: chunks an uploaded file into
  child records inside the same transaction using `e.app.save()`,
  reading the file from local storage and logging progress.
- `:158-175` — `cronAdd` job: finds un-embedded chunks, mints a
  superuser token via `$tokens.recordAuthToken()`, then `PATCH`-updates
  records through `$http.send()` to trigger vector embedding — a good
  template for "call myself over HTTP as a superuser from a cron job".
- `:178-210` — `routerAdd` POST route: reads the body via
  `e.requestInfo().body`, calls `$app` finders, builds an LLM message
  context, drives `$http.send()` calls, and responds via `e.json()`.

Copy the closest pattern from that file rather than inventing hook
plumbing from scratch — it's already exercised against a real instance.
