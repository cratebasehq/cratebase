# First-class auth: `@cratebase/client` SDK + server session/OAuth surface

## Context

Cratebase's auth surface is wire-complete against PocketBase (13 routes,
`crates/server/src/routes/auth.rs:109-154`) but the *developer-facing* API is
still "install the `pocketbase` npm SDK and call `pb.collection(...)`". The ask:
make auth genuinely first-class — Better-Auth-grade OAuth sign-in, impersonation
including *stopping* it, session management — behind a **first-party typed
TypeScript SDK**, and update the docs, which currently tell everyone to use the
official PocketBase client.

Four decisions are binding:

1. **Client + server.** Ship the missing server pieces (opt-in httpOnly cookie
   sessions, server-driven OAuth2 redirect/callback, stop-impersonating, session
   list/revoke, ban/unban) *and* a typed client over them.
2. **A full first-party SDK**, `@cratebase/client` — records, auth, realtime,
   files, batch, admin APIs — typed off `@cratebase/schema-codegen` output. This
   overrides the earlier "don't fork the SDK, bolt onto `pocketbase`" stance at
   `docs/superpowers/specs/2026-09-04-value-add-strategy.md:85-103`; that section
   gets a superseding note and every doc repeating it is rewritten (Phase 9).
3. **Docs are a first-class requirement, not a cleanup tail** — explicitly asked
   for. The docs site, README, landing page, `llms.txt`, `openapi.yaml` and the
   AI/extras pages present the `pocketbase` SDK as *the* client today; they must
   present `@cratebase/client` as the recommended one and demote the PocketBase
   SDK to a compatibility/interop note.
4. **The dashboard dogfoods the SDK** — `web/admin` is the only in-repo consumer
   and its call sites define the surface the SDK must actually cover.

End state: `bun add @cratebase/client`, `createClient<Schema>(url)`, typed
`client.collection("posts").list({ filter })`,
`client.auth.signIn.social({ provider: "google" })` completing through a
server-side redirect that sets an httpOnly cookie, and
`client.auth.admin.impersonate(id)` / `.stopImpersonating()` — all documented,
with the `pocketbase` SDK still passing conformance.

### Hard constraints (verified; do not violate)

- **`tests/conformance/auth.test.ts:102` asserts the exact JWT claim set**:
  `Object.keys(claims).sort()` must equal
  `["collectionId", "exp", "id", "refreshable", "type"]`. **No new claim may be
  added to auth tokens.** Session identity is `sha256(token)`, never a
  `sid`/`jti` claim.
- Two tests pin the **exact set of system collections** and must be updated in
  the same commit that registers new ones:
  `tests/conformance/collections.test.ts:249-266` (sorted `name` list from
  `GET /api/collections`) and the `system_collections_match_fixture_ids` test at
  `crates/core/src/collection.rs:1240-1258` (exact id vector). System collections
  are *not* filtered out of `GET /api/collections`
  (`crates/server/src/routes/collections.rs:124-129` returns `snapshot.all`), and
  the dashboard lists them in a dedicated group
  (`web/admin/src/components/layout/app-sidebar.tsx:147-150`), so `_sessions`
  and `_bans` will be visible there — intended.
- `tests/conformance/crons.test.ts:14-22` asserts the four built-in cron ids but
  only `jobs.length >= 4`, so adding a cron id is safe.
- The `pocketbase` npm SDK (`^0.28.0`) must keep passing unchanged. Every server
  addition is additive: new routes, new boot config, new Cratebase-only system
  collections. No existing response shape, status code, or error message changes.
- Tokens stay stateless (`crates/server/src/extract.rs:12-24`: signing key is
  `app_secret + record.tokenKey + type_secret`, deliberately no session table).
  Revocation must not add a database round trip to the authenticated hot path.
- `tower_http` refuses `allow_credentials(true)` together with
  `expose_headers(Any)` — `ensure_usable_cors_rules` panics at router
  construction. `crates/server/src/middleware/cors.rs:34` sets
  `.expose_headers(tower_http::cors::Any)` today, so enabling credentials
  *requires* replacing it with an explicit list.
- No cookie crate in `crates/server/Cargo.toml`; `sha2`, `base64`,
  `parking_lot`, `http` are already there. Cookie parse/serialize is hand-rolled,
  no new dependency.
- `crates/auth` already exports everything the OAuth state token needs:
  `random_state`, `code_verifier`, `code_challenge_s256`, `decode_unverified`,
  `signing_key`, `TokenType::Custom`, and `Claims` carries arbitrary extra
  claims via `#[serde(flatten)] pub extra: BTreeMap<String, Value>` plus
  `with_extra()` (`crates/auth/src/token.rs:93-113`).
- Rotating a record's `tokenKey` has **no** helper function; the pattern, used
  verbatim at `routes/auth.rs:973-977` and `:1152-1156`, is
  `record.set("tokenKey", Value::String(crate::app::new_token_key()))` followed
  by `cratebase_db::records::update(app.db(), &app.db().collections, &mut record)`.
- Rule-free maintenance queries against system collections go through
  `app.db().query(sql, &[Sql::Text(..)])` / `app.db().execute(...)` with `$1`
  placeholders — precedent at `crates/server/src/push.rs:699-713` and
  `crates/server/src/app.rs:555-568`. `cratebase_db::records` offers only
  `find_first_by_filter` (single row, `crates/db/src/records.rs:411`), a
  rule-aware paged `list` (`:257`), `create` (`:571`), `update` (`:624`) and
  `delete` by `Record` (`:709`) — there is no rule-free find-all or
  delete-by-filter, which is why the new module uses raw SQL for bulk work and
  `records::create` only for inserts.

## Approach

Phases 1-4 are server-side and land in order; each one compiles and leaves
`cargo test` and the conformance suite green. Phases 5-6 are the SDK and depend
only on the Phase 1-4 routes existing. Phases 7-9 are consumers and docs.

### Phase 1 — Session records + O(1) revocation

Foundation for session list/revoke, real sign-out, ban and stop-impersonating.

1. **Add the `_sessions` system collection.** In `crates/core/src/collection.rs`,
   add a builder beside the `_authOrigins` one (`:716-732`) and register it in
   `default_system_collections()` (`:1098-1108`). Copy `_authOrigins`'
   construction style (`Collection::new(name, CollectionType::Base)`,
   `system = true`, `fields.splice(pos..pos, [...])`).
   Fields, in order: `collectionRef` (text), `recordRef` (text), `tokenHash`
   (text, `hidden: true`, `system: true`), `kind` (text, system), `fingerprint`
   (text, system), `ip` (text, optional, system), `userAgent` (text, optional,
   system), `expiresAt` (date, system), `lastSeenAt` (date, optional, system),
   `revoked` (bool, system).
   `kind` values, exactly: `"password"`, `"otp"`, `"oauth2"`, `"impersonation"`,
   `"refresh"`.
   Indexes:
   `CREATE UNIQUE INDEX \`idx_sessions_token\` ON \`_sessions\` (tokenHash)` and
   `CREATE INDEX \`idx_sessions_record\` ON \`_sessions\` (collectionRef, recordRef)`.
   Rules: `list_rule` and `view_rule` = the same `owner_rule` string
   `_authOrigins` uses; `create_rule`, `update_rule`, `delete_rule` = `None`
   (superuser-only), so the only owner-facing mutation path is the Phase 4
   endpoints — no second, rule-driven revocation path to keep in sync.
2. **Update the two exact-set fixtures in the same change**: add `"_sessions"`
   to the sorted list at `tests/conformance/collections.test.ts:252-266` and the
   corresponding id to the fixture vector in the
   `system_collections_match_fixture_ids` test
   (`crates/core/src/collection.rs:1240-1258`).
3. **Migration.** `crates/db/src/migrations.rs`: add
   `pub const ADD_SESSIONS: &str = "11_add_sessions.rs";` plus
   `add_sessions_up`/`add_sessions_down`, modelled on `ADD_AUDIT_LOG`, and
   register it last in `Runner::core()` (`:56-150`). `up` looks the collection up
   from `Collection::default_system_collections()` by name and inserts it if
   absent; `down` deletes it if present.
4. **Put the raw token on `Auth`.** `crates/server/src/extract.rs:50-65`: add
   `pub token: String` and `pub exp: i64` to `Auth`, populated in `resolve` from
   the verified claims. Then **delete** the duplicated
   `fn bearer_token(headers: &HeaderMap)` at `routes/auth.rs:394-404` and rewrite
   `auth_refresh` (`:347-392`) to read `auth.token` instead, dropping its now
   unused `headers: HeaderMap` parameter. This is load-bearing beyond tidiness:
   that duplicate is what makes `auth-refresh` decline to renew a
   non-refreshable impersonation token, and it only looks at the
   `Authorization` header — a cookie-borne impersonation token would skip the
   guard and be re-minted refreshable by `mint` (`:564-573`), letting an
   impersonation session extend itself.
5. **New module `crates/server/src/sessions.rs`** (declare in `lib.rs` beside the
   other `mod` lines). Bulk reads/writes use `app.db().query` / `app.db().execute`
   with `$1` placeholders (precedent `push.rs:699-713`); the insert uses
   `cratebase_db::records::create(app.db(), &app.db().collections, &mut row)` so
   ids and autodates are generated normally.
   - `pub fn digest(token: &str) -> [u8; 32]` — `sha2::Sha256` of the raw token.
   - `pub fn hex(d: &[u8; 32]) -> String` — lowercase hex, stored as `tokenHash`.
   - `pub struct OriginContext { pub fingerprint: String, pub ip: String, pub user_agent: String }`.
   - `pub async fn record(app: &App, collection: &Collection, record: &Record, token: &str, kind: &str, origin: &OriginContext, expires_at: i64)` —
     best-effort insert; on error log `tracing::warn!` and continue, because a
     failed session row must never fail a login. No-op when
     `!app.config().session_tracking`.
   - `pub async fn revoke_digest(app: &App, collection_id: &str, record_id: &str, d: &[u8; 32]) -> anyhow::Result<bool>` —
     `UPDATE "_sessions" SET "revoked" = 1 WHERE "collectionRef" = $1 AND "recordRef" = $2 AND "tokenHash" = $3`,
     then inserts `d` into the in-memory set. Used by sign-out.
   - `pub async fn revoke_row(app: &App, row_id: &str) -> anyhow::Result<()>` —
     selects the row's `tokenHash`, updates it, inserts the digest. Used by
     `DELETE .../sessions/{id}`.
   - `pub async fn revoke_all_for(app: &App, collection_id: &str, record_id: &str, except: Option<&[u8; 32]>) -> anyhow::Result<usize>` —
     `SELECT "tokenHash"` for the record where `revoked = 0`, skip `except`,
     `UPDATE ... SET "revoked" = 1`, insert every hash into the set, return the
     count.
   - `pub async fn load_revoked(app: &App) -> anyhow::Result<()>` — boot-time
     `SELECT "tokenHash" FROM "_sessions" WHERE "revoked" = 1 AND "expiresAt" > $1`.
   - `pub fn is_revoked(app: &App, token: &str) -> bool` — returns `false`
     immediately when the revoked counter is `0` (the overwhelmingly common
     case), so an unrevoked deployment pays zero hashing cost.
   - `pub async fn sweep_expired(app: &App)` —
     `DELETE FROM "_sessions" WHERE "expiresAt" < $1`, dropping those digests
     from the set.
   - `pub async fn list_for(app: &App, collection_id: &str, record_id: &str) -> anyhow::Result<Vec<Row>>` —
     `SELECT "id", "kind", "fingerprint", "ip", "userAgent", "tokenHash", "created", "lastSeenAt", "expiresAt" FROM "_sessions" WHERE "collectionRef" = $1 AND "recordRef" = $2 AND "revoked" = 0 ORDER BY "created" DESC LIMIT 200`.
   No `touch()` function: updating `lastSeenAt` per request would put a write on
   the hot path. `lastSeenAt` is written only by `auth-refresh`.
6. **`crates/server/src/app.rs`**: `AppInner` gains
   `revoked_sessions: parking_lot::RwLock<std::collections::HashSet<[u8; 32]>>`
   and `revoked_len: std::sync::atomic::AtomicUsize` (raw digests, not hex
   `String`s — no allocation per check), with `pub(crate) fn revoked_sessions()`
   and `pub(crate) fn revoked_len()` accessors. Call
   `sessions::load_revoked(&app)` in the startup path that already calls
   `self.inner.cron.start()` (`app.rs:430`).
7. **Hot-path check.** `crates/server/src/extract.rs`, in `resolve` right after
   `cratebase_auth::verify` succeeds and the `claims.id` equality check
   (`:154-156`): `if crate::sessions::is_revoked(app, token) { return None; }`.
8. **Mint, record and respond in one place.** The token does not exist at the
   `record_login_origin` call sites — `mint` runs inside `respond_with_token`
   (`auth.rs:525-573`), after the `on_record_auth_request` hooks that may replace
   `event.record`. So:
   - `record_login_origin` keeps writing `_authOrigins` exactly as today and now
     also *returns* `sessions::OriginContext` built from the values
     `record_login_origin_inner` (`auth.rs:1974-2029`) already computes.
   - `respond_with_token` gains a parameter
     `session: Option<(&'static str, sessions::OriginContext)>` (kind + origin)
     and, right after `let token = mint(...)`, calls `sessions::record(...)` and
     then returns `Response` instead of `Json<Value>` so Phase 2 can attach a
     `Set-Cookie` header. Its four callers — `auth_with_password` (already
     returns `Result<Response, ApiError>`), `auth_with_otp`, `auth_with_oauth2`,
     `auth_refresh` — pass `Some(("password", origin))`, `Some(("otp", origin))`,
     `Some(("oauth2", origin))` and `Some(("refresh", origin))` respectively and
     have their return types changed to `ApiResult<Response>`.
   - `auth_refresh` additionally calls
     `sessions::revoke_digest(app, .., &digest(&auth.token))` before minting, so
     a rotated token stops being a live session, and writes `lastSeenAt` on the
     new row. The non-refreshable echo branch does neither.
   - `impersonate` (`auth.rs:2047-2109`) mints its own token rather than going
     through `respond_with_token`; add `headers: HeaderMap` and
     `peer: crate::middleware::client_ip::PeerAddr` extractors to it and build an
     `OriginContext` from the **impersonator's** UA/IP, then write a row with
     `kind: "impersonation"`. It must not write an `_authOrigins` row — that
     collection means "the record logged in from a new device", which is not what
     happened.
9. **Hourly sweep.** `crates/server/src/cron.rs`: add
   `pub const JOB_SESSION_SWEEP: &str = "__cbSessionSweep__";` next to
   `JOB_AUTO_BACKUP` (`:37`), with a comment noting it is Cratebase-only (the
   four existing ids are PocketBase's verbatim). Register it in `app.rs` beside
   `JOB_LOGS_CLEANUP` (`:608-627`) with expression `"0 * * * *"`, body
   `sessions::sweep_expired(&app)`.

### Phase 2 — Opt-in httpOnly cookie sessions

Cookie enablement is **boot config, not `Settings`**: the CORS layer is built
once at router construction (`lib.rs:80-138`) from `config.origins`, so a
hot-swappable setting could not turn on credentialed CORS.

1. **Config knobs** added to `pub struct Config`
   (`crates/server/src/config.rs:50-71`), read in `Config::from_env_with_dir`
   (`:126-152`) with the existing `env_or` / `env_bool` helpers. Add one row per
   variable to the module-doc env table at `config.rs:16-29`, and one entry each
   to `.env.example`.
   | field | env | default |
   | --- | --- | --- |
   | `session_cookie: bool` | `SESSION_COOKIE` | `false` |
   | `session_cookie_name: String` | `SESSION_COOKIE_NAME` | `"cb_session"` |
   | `session_cookie_domain: String` | `SESSION_COOKIE_DOMAIN` | `""` (host-only) |
   | `session_cookie_same_site: SameSite` | `SESSION_COOKIE_SAMESITE` | `Lax` |
   | `session_cookie_secure: bool` | `SESSION_COOKIE_SECURE` | `true` |
   | `session_tracking: bool` | `SESSION_TRACKING` | `true` |
   `SameSite` is a new enum in `config.rs` (`Lax`, `Strict`, `None`) parsed
   case-insensitively from `lax`/`strict`/`none`. `from_env_with_dir` is
   infallible, so an unrecognised env value falls back to `Lax` and logs
   `tracing::warn!` naming the three accepted values — it must not panic.
2. **Clap flags** in `ServeArgs` (`crates/server/src/main.rs:113-134`):
   `--session-cookie`, `--session-cookie-name`, `--session-cookie-domain`,
   `--session-cookie-samesite`, `--session-cookie-secure`, `--session-tracking`,
   every one declared `Option<...>` and applied at `main.rs:225-229` **only when
   `Some`**, following `--origins` (`:126-127`, `:225-227`). Do **not** copy
   `--dev`'s `config.dev = args.dev` form (`:129-130`, `:230`): with a
   `default_value_t` bool the flag always overwrites, which would silently make
   `SESSION_COOKIE=1` dead. `--session-cookie-samesite` uses a clap
   `value_parser` over the three literals, so a bad flag fails at argument
   parsing even though a bad env var only warns.
3. **New module `crates/server/src/cookie.rs`**:
   - `pub fn get<'a>(parts: &'a Parts, name: &str) -> Option<&'a str>` — splits
     the `Cookie` header on `;`, trims, matches `name=`.
   - `pub fn build(cfg: &Config, name: &str, value: &str, max_age: i64, path: &str) -> HeaderValue` —
     always `HttpOnly`; `Secure`, `SameSite` and `Domain` per config; `Max-Age`.
   - `pub fn attach(headers: &mut HeaderMap, value: HeaderValue)` — **appends**
     `set-cookie` so two cookies can ship in one response.
   - `pub fn session(cfg, token, max_age) -> HeaderValue` and
     `pub fn clear_session(cfg) -> HeaderValue` (empty value, `Max-Age=0`).
4. **Token extraction.** `crates/server/src/extract.rs:105-113`: replace
   `bearer_token` with
   `fn request_token<'a>(parts: &'a Parts, cfg: &Config) -> Option<(&'a str, TokenSource)>`,
   `enum TokenSource { Header, Cookie }`. The header wins; the cookie is
   consulted only when `cfg.session_cookie` is true and no `Authorization`
   header is present. `Auth` gains `pub via_cookie: bool`.
5. **CSRF origin gate.** New `crates/server/src/middleware/csrf.rs`:
   - `pub fn request_host(parts: &Parts) -> Option<&str>` — the `Host` header.
     No `X-Forwarded-Proto`/`X-Forwarded-Host` handling exists anywhere in
     `crates/` and none is added; `TrustedProxy`
     (`crates/core/src/settings.rs:272-276`) is `{headers, useLeftmostIP}` and
     concerns client IP only.
   - `pub fn origin_allowed(parts: &Parts, app: &App) -> bool` — `true` when the
     `Origin` header is absent (non-browser caller); otherwise `true` only when
     the `Origin`'s host:port equals `request_host` **or** the full `Origin`
     string is an exact member of `app.config().origins`. A `"*"` entry does
     **not** satisfy it: the default config is `origins: vec!["*"]`
     (`config.rs:96`), and treating that as permission would make the gate a
     no-op for every default deployment. Scheme is deliberately ignored in the
     same-host comparison; a same-host cross-scheme attacker needs an active
     MITM, which the `Secure` cookie attribute already addresses.
   - `pub async fn csrf(State(app), req, next)` — when `cfg.session_cookie`, the
     request carries the session cookie, has no `Authorization` header, and the
     method is not `GET`/`HEAD`/`OPTIONS`, reject with `403` and message
     `"Cross-site request rejected."` unless `origin_allowed`.
   Layer position matters: in `lib.rs:80-138` each later `.layer()` wraps the
   earlier ones, and the comment there explains rate-limit `429`s are logged
   because `rate_limit` sits *inside* `log_requests`. Register `csrf` between
   them — `.layer(rate_limit)` then `.layer(csrf)` then `.layer(log_requests)`
   then `.layer(record_metrics)` — so a rejected write is logged and metered but
   never consumes a rate-limit slot. Apply the identical layer to `js_routes`
   (`lib.rs:113-121`), which otherwise would let a hook route be driven
   cross-site with the caller's cookie.
   Also call `origin_allowed` inside `extract::resolve` and refuse to resolve a
   cookie token on an unsafe method when it fails; the middleware produces the
   clear error, this makes bypass impossible for a future route that skips the
   layer.
6. **CORS credentials.** `crates/server/src/middleware/cors.rs:23-42`: signature
   becomes `pub fn layer(origins: &[String], allow_credentials: bool) -> CorsLayer`.
   When `allow_credentials` is true **and** the origin list is explicit
   (non-empty, no `"*"`): add `.allow_credentials(true)` and replace
   `.expose_headers(tower_http::cors::Any)` with the explicit list
   `["content-type", "content-length", "content-disposition"]`
   (`content-disposition` is set by the file download paths,
   `routes/files.rs:178,195`). `tower_http`'s `ensure_usable_cors_rules` panics
   on credentials + wildcard expose-headers, so this substitution is mandatory,
   not cosmetic. In every other case the layer is byte-identical to today.
   The call site in `lib.rs` passes `app.config().session_cookie`. When cookies
   are on but the origin list is wildcard/empty, log one startup warning:
   `"SESSION_COOKIE is on with a wildcard CORS origin; browsers refuse credentialed cross-origin requests and the CSRF origin check will reject cross-site writes. Set CORS_ALLOW_ORIGINS to an explicit list."`
   Same-origin cookie auth still works in that case, so it is a warning, not a
   startup failure.
7. **Attach the cookie where the token is minted.** In `respond_with_token`
   (restructured in Phase 1 step 8) and in `impersonate`, when
   `cfg.session_cookie`, attach `cookie::session(cfg, &token, ttl)` with
   `ttl = collection.auth.auth_token.duration`. The JSON body is unchanged —
   `{token, record}` still carries the bearer token, so the PocketBase SDK is
   unaffected.

### Phase 3 — Server-driven OAuth2 redirect flow

Removes the popup + client-side code-exchange dance. Two new routes; all
security-critical record resolution is shared, not duplicated.

1. **Extract the shared completion path.** In `crates/server/src/routes/auth.rs`,
   split `auth_with_oauth2` (`:1462-1610`) so everything after body parsing
   becomes
   `pub(crate) async fn complete_oauth2(app: &App, collection: &Arc<Collection>, provider: &str, code: &str, code_verifier: &str, redirect_url: &str, create_data: Map<String, Value>, caller: Option<&Auth>, info: &RequestInfo) -> Result<Oauth2Outcome, ApiError>`
   with `pub(crate) struct Oauth2Outcome { record: Record, meta: Value, mfa: MfaGate }`.
   `auth_with_oauth2`'s own response shape (`{token, record, meta}`) stays
   byte-identical. `resolve_oauth2_record` (`:1619-1716`) and
   `create_oauth2_record` (`:1770-1861`), including the pre-hijacking
   protections, are untouched and reachable only through `complete_oauth2`.
2. **Extract the provider authorize-URL builder.** `oauth2_provider_info`
   (`auth.rs:459-521`) builds the per-provider `authURL`, `state`, PKCE pair and
   `codeChallengeMethod`. Lift the URL construction into
   `pub(crate) fn provider_auth_url(config: &cratebase_core::OAuth2Provider, state: &str, code_challenge: &str, redirect_uri: &str) -> Option<String>`
   and call it from both `oauth2_provider_info` and the new `start` route, so
   there is one implementation. It honours `config.pkce.unwrap_or(true)`
   (`auth.rs:481`) and `config.extra.scope` (`:467-473`) exactly as today; note
   that `oauth2_provider_info` ends its URL with a trailing `&redirect_uri=` for
   the SDK to fill in (`:507-511`) while `start` passes a real `redirect_uri`,
   so the trailing-empty-parameter behaviour stays in `oauth2_provider_info`.
   The new routes accept **no** `scopes` parameter: scope comes from the
   provider config and preset default, the same source `auth-methods` uses, so
   there is no precedence question to answer.
3. **New module `crates/server/src/routes/oauth2_flow.rs`** with
   `pub fn router() -> Router<App>`, added to the `.merge(...)` chain in
   `routes::api_router` (`crates/server/src/routes/mod.rs:41-70`, beside
   `.merge(auth::router())`):
   - `GET /collections/{collection}/oauth2/{provider}/start` — query `redirect`
     (required), `createData` (optional, base64url JSON).
   - `GET /collections/{collection}/oauth2/{provider}/callback` — query `code`,
     `state`, `error`.
4. **Public base URL is `settings.meta.app_url`** (serialized `appURL`,
   `crates/core/src/settings.rs:11-35`, default `"http://localhost:8090"`), and
   it is **required** for this flow — no scheme guessing from `Host`. When it is
   empty, `start` redirects with `cb_error=app_url_not_configured`. The
   `redirect_uri` handed to the provider and recomputed by `callback` is
   `{appURL}/api/collections/{collection}/oauth2/{provider}/callback`; providers
   demand an exact match, so both sites must build it with the same helper.
5. **Stateless flow state.** `start` mints `state` with
   `cratebase_auth::random_state()` and, when the provider has PKCE enabled, a
   pair with `code_verifier()` / `code_challenge_s256()`; it then signs a
   short-lived JWT — `Claims` with `token_type = TokenType::Custom("oauth2State")`,
   `id = state`, `collection_id = collection.id`, `exp = now + 600`, and
   `with_extra` carrying `{provider, codeVerifier, redirect, createData}` —
   using `cratebase_auth::signing_key(&cfg.secret, "", "oauth2State")` (no record
   is involved). It travels in cookie `cb_oauth_state` (`HttpOnly`,
   `SameSite=Lax`, `Path=/api`, `Max-Age=600`, `Secure` per config). No database
   row, nothing to sweep.
6. **Redirect target validation.** `redirect` is accepted when its origin equals
   the `appURL` origin or the full origin is an exact member of
   `config.origins` (a `"*"` entry does not qualify). Otherwise `400` with
   message `"Untrusted redirect target."` — this single case must answer JSON
   rather than redirect, or it becomes the open redirect it guards against.
7. **`start` otherwise always answers `303`.** Misconfiguration redirects to
   `redirect` with a `cb_error=<code>` query parameter, using exactly these
   codes: `cookie_sessions_disabled` (`!cfg.session_cookie`),
   `app_url_not_configured`, `oauth2_disabled`, `provider_not_enabled`. This is
   what lets `signIn.social()` be a single `location.assign()` with no probe
   request.
8. **`callback`** verifies the `cb_oauth_state` cookie JWT and requires
   `claims.id == state` and `claims.collection_id == collection.id`; a provider
   error (`?error=`) redirects to the state's `redirect` with
   `cb_error=<provider error>`. On success it calls `complete_oauth2`, mints the
   token exactly as `auth_with_oauth2` does, writes the session row
   (`kind: "oauth2"`), attaches the session cookie, clears `cb_oauth_state`, and
   `303`s to `redirect`. An MFA-pending outcome `303`s with `cb_mfa=<mfaId>` —
   an mfaId is safe in a URL, unlike a token, which is why no token ever appears
   in a redirect.
9. **Rate limiting.** `tags_for` (`crates/server/src/middleware/rate_limit.rs:314-381`):
   add an arm so the `oauth2` path segment pushes the existing `auth` and
   `authWithOauth2` tags, reusing the login buckets rather than inventing new
   ones.

### Phase 4 — Session, impersonation and ban routes

New module `crates/server/src/routes/session.rs`, `pub fn router() -> Router<App>`,
added to the `.merge(...)` chain in `routes::api_router`
(`crates/server/src/routes/mod.rs:41-70`). All paths are collection-scoped,
matching the existing auth routes. Every handler that needs the caller's raw
token or its expiry reads `auth.token` / `auth.exp` from the `Auth` extractor
extended in Phase 1 step 4.

1. `GET /collections/{collection}/sessions` — `Auth`; lists the caller's own
   sessions via `sessions::list_for`. A superuser may pass `?recordId=<id>` for
   another record. Response
   `{"items": [{"id", "kind", "fingerprint", "ip", "userAgent", "current", "created", "lastSeenAt", "expiresAt"}]}`;
   `current` is `hex(digest(&auth.token)) == row.tokenHash`. `tokenHash` is
   never serialized — the handler projects the fields explicitly.
2. `DELETE /collections/{collection}/sessions/{id}` — own row or superuser →
   `204` via `sessions::revoke_row`. Revoking the row that matches the caller's
   own digest also clears the session cookie.
3. `POST /collections/{collection}/sessions/revoke-others` → `{"revoked": n}`,
   `sessions::revoke_all_for(.., except: Some(&digest(&auth.token)))`.
4. `POST /collections/{collection}/sessions/revoke-all` → `{"revoked": n}`,
   `except: None`, and clears the session cookie.
5. `POST /collections/{collection}/auth-signout` — `Auth` → `204`;
   `sessions::revoke_digest(.., &digest(&auth.token))` and clear the session
   cookie. PocketBase has no such endpoint; it is additive, and it is what makes
   `client.auth.signOut()` more than a client-side store wipe.
6. `POST /collections/{collection}/stop-impersonating` — reads the
   `cb_session_prev` cookie, promotes it back into the session cookie, clears
   `cb_session_prev`, and answers `{"record": <restored record, serialized>}`.
   `400 "No impersonation session to stop."` when the cookie is absent.
   Deliberately returns no token: the restored credential lives only in the
   httpOnly cookie.
   Correspondingly `impersonate` (`auth.rs:2047-2109`) — when
   `cfg.session_cookie` and the caller resolved `via_cookie` — attaches
   `cb_session_prev` = the caller's `auth.token` with `Max-Age` = the smaller of
   `auth.exp - now` and the impersonation duration, before overwriting
   `cb_session`. In bearer mode there is nothing to do server-side: the SDK keeps
   the previous store in memory (Phase 6). Re-minting from the impersonation
   token is deliberately not offered — it would let a leaked impersonation token
   escalate back to the superuser.
7. **`_bans` system collection** (Cratebase-only), added exactly like
   `_sessions` (Phase 1 steps 1-3) with migration
   `pub const ADD_BANS: &str = "12_add_bans.rs";` and the same two fixture
   updates (`"_bans"` sorts before `"_cron_jobs"`). Fields: `collectionRef`
   (text), `recordRef` (text), `reason` (text, optional), `expiresAt` (date,
   optional), `bannedBy` (text, system). Unique index
   `CREATE UNIQUE INDEX \`idx_bans_unique_pairs\` ON \`_bans\` (collectionRef, recordRef)`.
   All rules `None` (superuser-only). Bans live in their own collection so no app
   schema is touched.
8. `POST /collections/{collection}/ban/{id}` — superuser, with the same
   owner-only escalation guard `impersonate` applies to `_superusers` targets
   (`auth.rs:2054-2074`), message `"Only an owner can ban a superuser account."`.
   Body `{"reason"?: string, "expiresAt"?: string}`. Upserts the `_bans` row,
   then invalidates every outstanding token for that record with the standard
   pattern — `record.set("tokenKey", Value::String(crate::app::new_token_key()))`
   followed by `cratebase_db::records::update(app.db(), &app.db().collections, &mut record)`
   (`auth.rs:973-977`, `:1152-1156`; there is no wrapper helper) — and calls
   `sessions::revoke_all_for(.., except: None)`. Response
   `{"recordRef", "reason", "expiresAt"}`.
9. `DELETE /collections/{collection}/ban/{id}` — superuser → `204`, deletes the
   row. Previously issued tokens stay dead (`tokenKey` already rotated); the user
   logs in again.
10. **Ban enforcement**:
    `pub(crate) async fn active_ban(app: &App, collection: &Collection, record_id: &str) -> Option<Record>`
    (row with `expiresAt` empty or in the future, via
    `records::find_first_by_filter`), called at the top of `auth_with_password`,
    `auth_with_otp`, `complete_oauth2` and `auth_refresh` →
    `403 "This account is banned."`. Enforcement lives in the login paths, not
    `extract::resolve`, so no per-request lookup is added; immediate lockout of
    live sessions is what the `tokenKey` rotation in step 8 buys.
11. **Rate-limit tags**: extend `tags_for`
    (`crates/server/src/middleware/rate_limit.rs:314-381`) with
    `"sessions" | "auth-signout" | "stop-impersonating" | "ban"` pushed through
    the existing `lower_camel(action)` call.

### Phase 5 — `@cratebase/client`: package, transport, records, realtime, files, batch, admin

New package at `sdk/js/client/`, mirroring `sdk/js/extras/`'s layout and
`tsconfig.json` (es2022 / esnext / bundler / strict / declaration /
declarationMap). No runtime dependencies.

1. **Packaging, and why it is *not* a workspace member.** `package.json`:
   `"name": "@cratebase/client"`, `"version": "0.1.0"`, `"type": "module"`,
   single root `exports` entry (`types` + `default` → `./dist/index.js`),
   `"files": ["dist", "README.md"]`, scripts `build` (`tsc -p tsconfig.json`) and
   `typecheck` (`tsc --noEmit`), plus its own `bun.lock` — exactly like
   `sdk/js/extras` and `tools/schema-codegen`. Publish workflow
   `.github/workflows/publish-client.yml` copied from `publish-extras.yml`,
   trigger tag `client-v*`.
   The package is **not** added to the root `package.json` `workspaces` array.
   Doing so would break `Dockerfile:10-18`, which copies only `web/admin`,
   `web/email` and `tests/conformance/package.json` before `bun install`
   precisely because Bun refuses to install with a declared member missing, and
   would make `web/admin`'s build depend on `sdk/js/client/dist` existing, which
   nothing in `ci.yml:55-58` or `release.yml:37-38` builds.
   Instead **the dashboard and the conformance suite consume the SDK from
   source**: add a Vite `resolve.alias` entry and a `tsconfig` `paths` entry
   mapping `"@cratebase/client"` →
   `<repo>/sdk/js/client/src/index.ts` in `web/admin` and
   `tests/conformance`. One line is added to the Dockerfile —
   `COPY sdk/js/client sdk/js/client` beside the existing `COPY web/admin` — and
   its stage comment at `Dockerfile:3-7`, which currently states the dashboard
   uses "the official `pocketbase` npm client — no in-house SDK to build here
   anymore", is corrected.
2. **`src/transport.ts`**
   - `export class CratebaseError extends Error` with
     `readonly status: number`, `readonly url: string`,
     `readonly response: { status: number; message: string; data: Record<string, { code: string; message: string }> }`,
     `readonly isAbort: boolean`, `readonly mfaId?: string`. `mfaId` comes from
     the server's bare `401 {"mfaId": "..."}` body
     (`auth.rs:1863-1875`), which is not wrapped in the standard envelope. This
     type replaces every dashboard use of `ClientResponseError`.
   - `export interface SendOptions { method?, query?, body?, headers?, signal?, fetch? }`
     — `undefined` query values are dropped, arrays joined with `,`; `body` is
     JSON-encoded unless it is `FormData`.
   - `send<T>(path, options): Promise<T>`; `204` resolves to `undefined as T`.
   - `buildURL(path): string`, correct for a `baseUrl` of `"/"` (the dashboard's
     configuration) — naive prefixing yields `//api/...`, a protocol-relative URL
     resolved against a host literally named `api`, the bug documented at
     `web/admin/src/components/settings/file-manager-page.tsx:68-77`.
   - **No auto-cancellation.** PocketBase auto-cancels same-key in-flight
     requests, which is why the dashboard sprinkles `requestKey: null`. Callers
     pass an `AbortSignal` instead.
3. **`src/types.ts`** — `RecordModel`
   (`{ id: string; collectionId: string; collectionName: string; [k: string]: unknown }`),
   `CollectionModel` and `CollectionField` (the shapes `GET /api/collections`
   returns; the dashboard imports `CollectionModel` from `pocketbase` in ten
   files, e.g. `web/admin/src/components/layout/app-sidebar.tsx:2`,
   `.../dashboard/dashboard-home.tsx:5`, so the SDK must export it),
   `RecordSubscription<T>`, `ListResult<T>`
   (`{ page: number; perPage: number; totalItems: number; totalPages: number; items: T[] }`),
   `SortSpec<T>`, `ListOptions<T>`, `ViewOptions<T>`, `SubscribeOptions<T>`.
   `totalItems`/`totalPages` are **required `number`s carrying `-1`** when
   `skipTotal` is set: `Page` (`crates/server/src/routes/records.rs:108-121`) has
   non-optional `i64` fields and the query layer returns `-1`
   (`crates/db/src/records.rs:283-285`, `:317-325`) — it never omits the keys.
   `ListOptions` keys map 1:1 onto `ListQuery`
   (`crates/server/src/routes/records.rs:71-87`): `page`, `perPage`, `sort`,
   `filter`, `expand`, `fields`, `skipTotal`.
4. **`src/filter.ts`** — `export function filter(strings, ...values): string`, a
   tagged template that serializes interpolations as **values** (strings quoted
   and escaped, numbers/bools raw, `Date` → ISO-8601, `null`/`undefined` →
   `null`, arrays → parenthesised list), plus
   `export function raw(identifier: string): Raw` for interpolating a **field
   name**. Both are needed: `web/admin/src/components/records/relation-picker.tsx:96`
   interpolates a field name (`${display} ~ "${term}"`) while `:113`
   interpolates values.
5. **`src/records.ts`** — `CollectionService<T>`, exactly:
   `list(options?): Promise<ListResult<T & RecordModel>>`,
   `fullList(options?): Promise<Array<T & RecordModel>>` (pages internally at
   `perPage: 500`), `first(options?): Promise<(T & RecordModel) | null>`,
   `one(id, options?): Promise<T & RecordModel>`,
   `create(data: Partial<T> | FormData, options?): Promise<T & RecordModel>`,
   `update(id, data: Partial<T> | FormData, options?): Promise<T & RecordModel>`,
   `delete(id): Promise<void>`,
   `subscribe(topic, handler, options?): Promise<() => void>`.
   Object-options only — no positional `getList(page, perPage, opts)` alias, so
   the codebase has exactly one calling convention. Reads return
   `T & RecordModel` so a codegen'd interface (which carries only schema fields)
   still yields typed `collectionId`/`collectionName`.
6. **`src/realtime.ts`** — one shared connection per client.
   `GET /api/realtime` is consumed with **`fetch` plus a `ReadableStream` SSE
   parser**, not `EventSource`: `EventSource` cannot send an `Authorization`
   header and needs a polyfill outside browsers. That makes the stream itself
   authenticated and removes the polyfill requirement `@cratebase/extras`
   documents. Parse `event:`/`data:`/`id:` frames; the first is
   `event: PB_CONNECT` with `data: {"clientId": "<40 alphanumerics>"}`
   (`crates/server/src/realtime.rs:380-400`). Subscriptions are declared with
   `POST /api/realtime` `{clientId, subscriptions}`; a topic is `collection`,
   `collection/recordId`, `collection/*`, or
   `collection?options=<url-encoded {"query":{"filter","fields","expand"}}>`
   (`realtime.rs:137-189`). Re-POST on every subscribe/unsubscribe and after any
   auth change; reconnect with capped exponential backoff. Handler payload:
   `{ action: "create" | "update" | "delete", record: T & RecordModel }`.
7. **`src/files.ts`** — `url(record, filename, options?: { thumb?, download?, token? })`
   building `/api/files/{collectionIdOrName}/{recordId}/{filename}`, and
   `token(): Promise<string>` for `POST /api/files/token`. `thumb` accepts the
   server grammar (`WxH`, `WxHt|b|f`, `0xH`, `Wx0`).
8. **`src/batch.ts`** — `client.batch()` returns a builder with
   `create(collection, data)`, `update(collection, id, data)`,
   `delete(collection, id)`, `upsert(collection, data)` and
   `send(): Promise<Array<{ status: number; body: unknown }>>`. JSON body when no
   value is a `File`/`Blob`, otherwise multipart with `@jsonPayload` plus
   `requests.<index>.<field>` parts
   (`crates/server/src/routes/batch.rs:134-183`). A `400` surfaces as a
   `CratebaseError` whose `.response.data.requests` carries the failing index's
   nested `response` (`batch.rs:929-943`).
9. **`src/admin.ts`** — the surfaces the dashboard needs, each a thin typed
   wrapper over `send`: `collections` (`list`, `one`, `create`, `update`,
   `delete`, `truncate`, `import`, `scaffolds`), `schema.apply`, `settings`
   (`get`, `update`, `testS3`, `testEmail`, `generateAppleClientSecret`), `logs`
   (`list`, `one`, `stats`), `backups` (`list`, `create`, `upload`, `delete`,
   `restore`, `storageInfo`, `downloadURL`), `crons` (`list`, `run`), `storage`
   (`list`, `upload`, `delete`, `downloadURL`), `sql`, `functions`,
   `apiKeys.create`, `push.send`, `health`, `setup` (`status`, `create`).
10. **`src/cratebase-only.ts`** — `vector.nearestTo`, `llm.chat`,
    `mcp.toolSchema(s)`, `queue.enqueue`, `presence.track`. `llm.chat` streams
    through the client's own realtime connection (`llm_chunk` / `llm_done` /
    `llm_error` frames).
    `@cratebase/extras` keeps its own implementations and is **not** rebased onto
    this module: it peer-depends on `pocketbase`, has an independent lockfile,
    and `publish-extras.yml:26-48` runs `bun install --frozen-lockfile` inside
    `sdk/js/extras` before `npm publish`, so a `workspace:`/source dependency
    could neither resolve nor publish. The overlap is five thin wrappers over
    `send`, and the two packages target different client objects.
11. **`src/index.ts`** —
    `createClient<S extends Record<string, Record<string, unknown>> = Record<string, RecordModel>>(baseUrl, options?)`
    returning `CratebaseClient<S>` with `collection<K extends keyof S & string>(name: K)`,
    `auth`, `realtime`, `files`, `batch`, `admin`, `vector`, `llm`, `mcp`,
    `queue`, `presence`, `send`, `buildURL`. The constraint is
    `Record<string, Record<string, unknown>>`, **not** `Record<string, RecordModel>`,
    because codegen'd interfaces carry only schema fields and would not satisfy
    the stricter bound. `options`:
    `{ authStore?, authCollection? (default "users"), credentials? (default "same-origin"), cookie?, fetch?, headers?, lang? }`.

### Phase 6 — `@cratebase/client`: the auth namespace

`src/auth-store.ts` and `src/auth.ts`.

1. **Stores.**
   `interface AuthStore { token: string; record: RecordModel | null; save(token, record): void; clear(): void; onChange(cb, fireImmediately?): () => void }`
   with `MemoryAuthStore`, `LocalAuthStore` (browser `localStorage`, key
   `"cratebase_auth"`) and `AsyncAuthStore` (`{ save, clear?, initial? }`, for
   React Native / Expo). Default: `LocalAuthStore` in browsers,
   `MemoryAuthStore` elsewhere. `isValid` decodes `exp` locally; `isSuperuser` is
   `record?.collectionName === "_superusers"`.
2. **`client.auth`** is bound to `options.authCollection`; `client.auth.as(name)`
   returns the same interface bound to another collection (the dashboard uses
   `client.auth.as("_superusers")`). Members, exactly:
   - `token`, `record`, `isValid`, `isSuperuser`, `onChange(cb, fireImmediately?)`
   - `signUp(data: { email: string; password: string; passwordConfirm: string } & Record<string, unknown>): Promise<RecordModel>`
     — `POST .../records`, then `signIn.password` unless
     `options.autoSignIn === false`.
   - `signIn.password({ identity, password, identityField?, mfaId? })`
   - `signIn.otp({ otpId, code, mfaId? })` — sends
     `{otpId, password: code, mfaId}`; the wire field really is `password`
     (`auth.rs:1265-1276`).
   - `signIn.code({ provider, code, codeVerifier, redirectUrl, createData? })` —
     the existing `auth-with-oauth2` exchange.
   - `signIn.social({ provider, redirect?, createData?, mode? })`
   - `signOut()` — `POST .../auth-signout`, then clears the store.
   - `refresh()`, `methods()`
   - `otp.request({ email }): Promise<{ otpId: string }>`
   - `verifyEmail.request(email)` / `.confirm(token)`
   - `resetPassword.request(email)` / `.confirm({ token, password, passwordConfirm })`
   - `changeEmail.request(newEmail)` / `.confirm({ token, password })`
   - `sessions.list(options?)` / `.revoke(id)` / `.revokeOthers()` / `.revokeAll()`
   - `externalAuths.list(recordId)` / `.unlink(recordId, provider)`
   - `admin.impersonate(recordId, { duration? }): Promise<CratebaseClient<S>>` —
     returns a *new* client backed by a `MemoryAuthStore`, never mutating the
     caller's store, and remembers the caller's token so
     `admin.stopImpersonating()` can restore it in bearer mode; in cookie mode
     that method calls `POST .../stop-impersonating` and then `refresh()`.
   - `admin.ban(recordId, { reason?, expiresAt? })` / `admin.unban(recordId)`
   - `completeSocial(search?: string): Promise<AuthResult | { mfaId: string }>` —
     reads `cb_error` / `cb_mfa` from `location.search` (or the passed string),
     throws a `CratebaseError` for `cb_error`, returns `{mfaId}` for `cb_mfa`,
     otherwise calls `refresh()` to hydrate the store from the cookie.
3. **`signIn.social` modes**, all four fixed here so nothing is invented:
   - `"redirect"` (default in a browser) — `location.assign` of
     `/api/collections/{c}/oauth2/{provider}/start?redirect=<current URL>`;
     resolves `{ url }` immediately before navigating.
   - `"manual"` (default when `typeof window === "undefined"`) — returns
     `{ url }` and navigates nothing, for SSR `redirect()` handlers.
   - `"popup"` — opens the start URL in a popup, polls `popup.closed`, then
     `refresh()` (the cookie is already set) and resolves the `AuthResult`.
   - `"pkce"` — the bearer-only path for deployments without cookie sessions:
     `methods()` → open the provider `authURL` → collect `code` →
     `signIn.code`.
4. **MFA.** A first factor needing a second throws a `CratebaseError` with
   `status === 401` and `mfaId` set; the caller retries the *other* method with
   `{ mfaId }`. This is the one place a 401 is not a failure.
5. **SSR/cookies.** `createClient(url, { cookie: req.headers.get("cookie") })`
   forwards that header on every request; `client.auth.exportCookie(): string`
   returns a `Set-Cookie` value for the bearer token, for frameworks managing
   their own cookie when server-side cookie sessions are off. Cross-origin
   cookie mode needs `credentials: "include"`.

### Phase 7 — Codegen emits a schema map

`tools/schema-codegen/src/codegen.ts`: `generate()` (`:140-149`) emits, after the
interfaces:

```ts
/** Every non-system collection, keyed by name — pass to `createClient<Schema>()`. */
export interface Schema {
  posts: PostsRecord;
}
export type Collections = keyof Schema;
```

built from the same filtered `nonSystem` list, so `client.collection("posts")` is
typed and an unknown name is a compile error. The emitted interfaces
(`generateInterface`, `:89-103`) stay exactly as they are — no base-record
`extends` is added, because `createClient`'s constraint is deliberately
`Record<string, Record<string, unknown>>` and reads widen to `T & RecordModel`.
Bump the package version and document the new export in
`tools/schema-codegen/README.md`.

### Phase 8 — Dashboard migrates to the SDK (dogfooding)

`web/admin` is the proof the surface is complete.

1. `web/admin/package.json`: drop `pocketbase`. Add the `resolve.alias` entry to
   `web/admin/vite.config.ts` and the matching `paths` entry to
   `web/admin/tsconfig.json`, both pointing `"@cratebase/client"` at
   `../../sdk/js/client/src/index.ts`.
2. `web/admin/src/lib/api.ts` —
   `export const cb = createClient(import.meta.env.VITE_API_URL ?? "/")`;
   `isLoggedIn()` → `cb.auth.isValid && cb.auth.record !== null`;
   `currentSuperuser()` reads `cb.auth.record`;
   `authWithPassword` → `cb.auth.as("_superusers").signIn.password({ identity, password })`;
   `signOut()` → `await cb.auth.as("_superusers").signOut()`;
   `describeFailure`/`isSessionExpired` switch from `ClientResponseError`
   (imported at `api.ts:1`) to `CratebaseError`.
3. Mechanical call-site migration, one convention throughout:
   `cb.collection(x).getList(p, pp, o)` → `.list({ page, perPage, ...o })`;
   `getFullList(o)` → `fullList(o)`; `getOne` → `one`;
   `cb.getFileUrl` / `cb.files.getURL` → `cb.files.url`;
   `cb.authStore.token` → `cb.auth.token`;
   `CollectionModel` imports repoint from `pocketbase` to `@cratebase/client`;
   `cb.send(...)` becomes the typed `cb.admin.*` method where one exists
   (`/api/backups*`, `/api/crons`, `/api/logs*`, `/api/storage/objects*`,
   `/api/sql`, `/api/functions`, `/api/api-keys`, `/api/mcp`), otherwise stays
   `cb.send`. Drop every `requestKey: null` — there is no auto-cancellation to
   opt out of. `relation-picker.tsx:96` becomes
   `filter\`${raw(display)} ~ ${term}\`` and `:113` a `filter`-tag join.
4. Add a superuser-facing **Sessions** panel under
   `web/admin/src/components/settings/` listing `cb.auth.sessions.list()` with
   per-row revoke, and wire **Impersonate** / **Ban** actions into
   `superusers-page.tsx` and the record view. This is the only new dashboard
   feature; everything else is a rename.

### Phase 9 — Docs, OpenAPI, and examples

Every new/edited docs page follows the existing convention exactly: frontmatter
`title` / `description` / `sidebar.order`, terse technical prose, inline code, no
custom Starlight components.

1. **New sidebar group.** `site/astro.config.mjs:66-126`: insert
   `{ label: "TypeScript SDK", items: [{ autogenerate: { directory: "docs/sdk" } }] }`
   directly after the "Getting started" group. New pages under
   `site/src/content/docs/docs/sdk/` then auto-register: `overview.mdx` (1,
   install + `createClient` + typed `Schema`), `records.mdx` (2, the
   object-options CRUD API, the `filter`/`raw` tags, the `-1` totals under
   `skipTotal`), `auth.mdx` (3, the whole `client.auth` namespace including the
   four `signIn.social` modes and the MFA 401), `realtime.mdx` (4),
   `files.mdx` (5), `batch-and-admin.mdx` (6), `ssr-and-cookies.mdx` (7),
   `pocketbase-interop.mdx` (8, `@cratebase/extras` and moving off the
   `pocketbase` SDK).
2. **New auth concept pages** under
   `site/src/content/docs/docs/concepts/authentication/` (autogenerated sidebar,
   ordered by frontmatter): `sessions.mdx` (12), `cookie-sessions.mdx` (13),
   `bans.mdx` (14).
3. **Extend existing auth concept pages**: `overview.mdx` (1) — the two token
   transports and what revocation now means; `oauth2.mdx` (6) — the
   server-driven redirect flow, its `cb_error`/`cb_mfa` query parameters and the
   `{appURL}/api/collections/{c}/oauth2/{provider}/callback` URL to register with
   the provider; `impersonation.mdx` (7) — `stop-impersonating` and the
   `cb_session_prev` cookie.
4. **Reference pages**:
   `site/src/content/docs/docs/reference/rest-api/auth.mdx` gains table rows and
   `###` sections for `auth-signout`, `sessions`, `sessions/{id}`,
   `sessions/revoke-others`, `sessions/revoke-all`, `stop-impersonating`,
   `ban/{id}`, `oauth2/{provider}/start`, `oauth2/{provider}/callback`.
   `reference/system-collections.mdx` gains `_sessions` and `_bans`.
   `deploy/configuration-reference.mdx` gains the six `SESSION_*` variables.
   `extending/schema-codegen.mdx` documents the emitted `Schema` map.
   **`openapi.yaml`** (linked from `README.md:56`) gains the same nine paths with
   request/response schemas.
5. **Reposition the existing SDK pages**:
   `getting-started/using-js-sdk.mdx` becomes the `@cratebase/client`
   quickstart (title "Using the TypeScript SDK") closing with a pointer to
   `pocketbase-interop.mdx`; `getting-started/other-sdks.mdx` distinguishes the
   first-party TypeScript SDK from the PocketBase-family SDKs that still work;
   `ai/extras-package.mdx` reframes `@cratebase/extras` as the companion for
   projects staying on the `pocketbase` SDK and points at `client.vector` /
   `client.llm` / `client.mcp` / `client.queue` / `client.presence`.
6. **Root and marketing copy**: `README.md:44-47` and `:59-63`,
   `site/src/pages/index.astro:105-109`,
   `site/src/content/docs/docs/index.mdx:9` switch their samples to
   `@cratebase/client` while keeping the wire-compatibility claim, which stays
   true and is still what `tests/conformance` proves. `llms.txt` gains the new
   endpoints and the package name. `ARCHITECTURE.md:104-134` gains the cookie
   transport and the revocation set. `Dockerfile:3-7`'s stage comment is
   corrected (Phase 5 step 1).
7. **Correct the stale stance**: add a `> **Superseded 2026-09-08:**` block under
   §1 of `docs/superpowers/specs/2026-09-04-value-add-strategy.md:85-103` (these
   dated spec files are a record, so they are annotated, not rewritten); rewrite
   `sdk/js/extras/README.md:4-20`; update `.claude/skills/cratebase/SKILL.md:36-38`
   and `.claude/skills/cratebase/references/collections-and-sdk.md:63-66` to
   recommend `@cratebase/client`; note in `CONTRIBUTING.md:84` that
   `tests/conformance` deliberately keeps its `pocketbase`-SDK suites while the
   new `client*.test.ts` files exercise the first-party one;
   `docs/migrating-from-pocketbase.md` and
   `site/src/content/docs/docs/migrating/compatibility.mdx:8` keep the wire
   promise and add that the `pocketbase` SDK works unchanged.
8. **Examples.** `examples/kanban` is the only example with a real npm
   dependency (`"pocketbase": "^0.28.0"`,
   `examples/kanban/src/pocketbase.ts:6-13`). Migrate it to `@cratebase/client`
   as the flagship typed example: rename the module to `src/cratebase.ts`,
   export a `createClient<Schema>` singleton, and update
   `src/hooks/useAuth.ts` (`signIn.password`, `signUp`, `auth.onChange`),
   `useCards.ts` and `usePresence.ts`. The four zero-build examples (`todo`,
   `realtime-chat`, `realtime-cursors`, `docmind`) resolve `"pocketbase"`
   through an `esm.sh` import map, which cannot point at an unpublished package —
   they stay on the `pocketbase` SDK, with one README line each noting
   `@cratebase/client` is the first-party option for build-based apps. Migrate
   them once `client-v0.1.0` is on npm.

## Critical files & anchors

- `crates/server/src/extract.rs:49-160` — `Auth` (gains `token`, `exp`,
  `via_cookie`), `bearer_token` → `request_token`, and `resolve`, where the
  revocation check and the fail-safe origin check land. Keep the module doc at
  `:1-34` accurate.
- `crates/server/src/routes/auth.rs` — `:109-154` router; `:347-404`
  `auth_refresh` plus the duplicate `bearer_token` to delete; `:459-521`
  `oauth2_provider_info` (URL builder to extract); `:525-573`
  `respond_with_token`/`mint`, the single token-minting point; `:1462-1610`
  `auth_with_oauth2` to split; `:1974-2029` `record_login_origin_inner`;
  `:2047-2109` `impersonate` and its owner-only guard at `:2054-2074`.
- `crates/core/src/collection.rs:716-732` (`_authOrigins`, the template for
  `_sessions`/`_bans`), `:1098-1108` (`default_system_collections()`),
  `:1240-1258` (the exact-id fixture test to update).
- `crates/server/src/lib.rs:80-138` — router assembly; later `.layer()` wraps
  earlier, which is why the CSRF layer goes between `rate_limit` and
  `log_requests`, and `js_routes` at `:113-121` needs it too.
- `web/admin/src/lib/api.ts` — the dashboard's only client seam; migrating it
  first makes the remaining files mechanical.

## Verification

Prerequisites: repo root, `cargo` toolchain, `bun`.

1. **Build + Rust tests** —
   `cargo test -p cratebase-server -p cratebase-core -p cratebase-db`.
   `system_collections_match_fixture_ids`
   (`crates/core/src/collection.rs:1240-1258`) is the canary that a new system
   collection was registered without updating the fixture.
2. **PocketBase-SDK conformance must stay green** —
   `cargo build --release && SERVER_BIN=target/release/cratebase bun run conformance`.
   `tests/conformance/auth.test.ts:102` is the canary for an accidental extra JWT
   claim; `collections.test.ts:249-266` for the system-collection set.
3. **Cookie mode in the harness.** `tests/conformance/harness.ts` spawns one
   server per run from `process.env` (`:87-92`, `:122-124`, `:155-158`) and has
   no per-file spawn API, so cookie mode is enabled **run-wide** for the
   Cratebase flavour only: add `SESSION_COOKIE: "1"`,
   `SESSION_COOKIE_SECURE: "0"` and
   `CORS_ALLOW_ORIGINS: "http://127.0.0.1:<spawned port>"` to that branch's env
   block. Cookie auth is purely additive, so the existing bearer-token suites are
   unaffected, and the `PB_BIN` flavour ignores the variables.
4. **New first-party SDK suite.** Two files in `tests/conformance/` reusing the
   existing harness (server spawn, SMTP sink, `uniq`, `expectError`), with
   `"@cratebase/client"` resolved through the `tsconfig` `paths` entry from
   Phase 5 step 1:
   - `client.test.ts` — typed CRUD; the `filter`/`raw` tags; a realtime
     subscription receiving a `create` event; a file URL with `thumb`; a batch
     whose second sub-request fails, asserting
     `err.response.data.requests["1"].response.status === 400`; and
     `list({ skipTotal: true })` returning `totalItems === -1`.
   - `client-auth.test.ts` — input → expected observable, one per behaviour:
     - `signIn.password` then `sessions.list()` → exactly one item, `current: true`,
       and `Object.keys(item)` contains no `tokenHash`.
     - a second `signIn.password` with a different `User-Agent` → two items;
       `sessions.revokeOthers()` → `{revoked: 1}`; the other client's next
       `one()` → `401`.
     - `signOut()`, then a raw `send` with the old token → `401` (before this
       work the token stayed valid, because sign-out was client-side only).
     - `admin.ban(id)` → the banned user's `signIn.password` fails `403` with
       message `"This account is banned."`; `admin.unban(id)` → login succeeds.
     - `admin.impersonate(id)` → a client acting as the target whose `refresh()`
       returns the identical token, while the impersonating client's own
       `auth.token` is unchanged.
     - cookie mode: the `signIn.password` response carries a `set-cookie` for
       `cb_session`; a client built with `{ cookie: "<that cookie>" }` and no
       bearer token can `one()` its own record; that same client issuing a
       `PATCH` with header `Origin: https://evil.example` gets `403` and message
       `"Cross-site request rejected."`
     - `GET /api/collections/users/oauth2/google/start?redirect=https://evil.example`
       → `400 "Untrusted redirect target."`; with a trusted `redirect` and
       `oauth2` disabled → `303` whose `Location` contains
       `cb_error=provider_not_enabled`. (A full provider round trip needs a live
       Google/GitHub app; the exchange itself is covered by the shared
       `complete_oauth2` that `auth-with-oauth2` already exercises.)
   Run: `SERVER_BIN=target/release/cratebase bun run conformance`.
5. **Typecheck the packages** —
   `bun install && bun run --cwd sdk/js/client typecheck && bun run --cwd tools/schema-codegen typecheck`.
6. **Dashboard** — `bun run admin:build` succeeds and
   `grep -rn "from \"pocketbase\"" web/admin/src` returns nothing. Then serve the
   built dashboard against a local server and confirm by hand: log in, list
   records, upload a file, open the Sessions panel and revoke a session,
   impersonate a user and stop impersonating.
7. **Docker image still builds** — `docker build .` (proves the added
   `COPY sdk/js/client` line and the untouched workspace list are consistent).
8. **Docs build** — `bun run --cwd site build`; the "TypeScript SDK" group
   renders with its eight pages, and
   `grep -rn "bun add pocketbase" site/src README.md` shows it only as an interop
   note.

## Assumptions & contingencies

- **Cookie enablement is boot config, not `Settings`.** If a dashboard toggle is
  wanted later it must also rebuild the CORS layer; do not add it as a plain
  `Settings` field.
- **Session rows are written even in bearer mode** (`SESSION_TRACKING=1` by
  default) so `sessions.list()`/revoke and real sign-out work without cookies.
  With it off, `sessions.list()` returns an empty list and revoke is a no-op —
  the docs must say so.
- **`settings.meta.appURL` is required for the server-driven OAuth flow.** If a
  deployment cannot set it, `signIn.social({ mode: "pkce" })` remains a complete
  bearer-mode path and needs no server configuration.
- **`@cratebase/client` is consumed from source in-repo** (Vite alias +
  tsconfig `paths`) rather than as a workspace package. If a future consumer
  needs the built package instead, publish `client-v0.1.0` and switch that
  consumer to the npm version — do not add the directory to the root
  `workspaces` array without also adding a `COPY` line to `Dockerfile:10-18` and
  a `sdk/js/client` build step to `ci.yml:55-58` and `release.yml:37-38`.
- **The four `esm.sh` examples stay on the `pocketbase` SDK** until
  `client-v0.1.0` is published. If publishing happens during execution, migrate
  them in the same pass by pointing the import map at
  `https://esm.sh/@cratebase/client@0.1`.
