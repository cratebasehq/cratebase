# Cratebase auth demo

Static ESM app (no build step) exercising every auth flow Cratebase exposes
on the default `users` auth collection: register, password login, OAuth2
(Google/GitHub), password reset, email verification, and email change.

`index.html` and `oauth-callback.html` both declare an [import map](https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/script/type/importmap)
mapping the bare specifier `"cratebase"` to the already-built local
package (`../../sdk/js/dist/index.js`, run `npm run build` inside
`sdk/js/` if `dist/` doesn't exist yet), so both `app.js` and
`oauth-callback.js` can write `import { Cratebase, ClientResponseError }
from "cratebase";` — the same zero-build-step approach every other
`examples/*` app uses.

Files:

- `index.html` / `app.js` — the main app: guest view (register, login,
  OAuth2 buttons, forgot-password) and authenticated view (profile,
  logout, verification, email change).
- `oauth-callback.html` / `oauth-callback.js` — the OAuth2 redirect
  target. Reads `?code=` off the URL, exchanges it via `authWithOAuth2`,
  and bounces back to `index.html`.
- `style.css` — shared styling for both pages.

## 1. Configure the `users` collection

Cratebase ships a default `users` auth collection with `identityField:
"email"` already. To confirm/adjust it (or recreate it), authenticate as a
superuser and `PATCH` the collection:

```bash
# Superuser/admin login — replace with your own admin credentials.
ADMIN_TOKEN=$(curl -s http://localhost:8090/api/admins/auth-with-password \
  -H 'content-type: application/json' \
  -d '{"email":"admin@example.com","password":"adminpassword"}' | jq -r .token)

# Inspect the current schema.
curl -s http://localhost:8090/api/collections/users \
  -H "authorization: Bearer $ADMIN_TOKEN" | jq .

# Make sure identityField is "email" (the default — only needed if you
# changed it) and, optionally, flip requireEmailVerification on to see
# unverified users blocked from features that check `record.verified`
# (this demo never blocks login on it — it just displays the flag — but
# flipping this toggle is a good way to observe `verified` flipping to
# `true` after the confirm-verification step below).
curl -s -X PATCH http://localhost:8090/api/collections/users \
  -H "authorization: Bearer $ADMIN_TOKEN" \
  -H 'content-type: application/json' \
  -d '{
        "authOptions": {
          "identityField": "email",
          "requireEmailVerification": false
        }
      }'
```

## 2. Register OAuth2 apps

The demo builds its OAuth2 redirect URI from wherever `oauth-callback.html`
is actually served, e.g. `http://localhost:4173/oauth-callback.html` (see
"Serve it locally" below for the exact URL your setup will use — print it
by opening the browser console on `index.html`, or just read it off the
address bar once you visit `oauth-callback.html`). **Both providers need
their "authorized redirect URI" set to that exact URL** — scheme, host,
port, and path must match byte-for-byte, including the absence/presence of
a trailing slash.

### Google

1. https://console.cloud.google.com/apis/credentials → **Create
   credentials → OAuth client ID** → Application type **Web application**.
2. Under **Authorized redirect URIs**, add your `oauth-callback.html` URL,
   e.g. `http://localhost:4173/oauth-callback.html`.
3. Copy the generated **Client ID** and **Client secret**.

### GitHub

1. https://github.com/settings/developers → **OAuth Apps → New OAuth App**.
2. **Homepage URL**: `http://localhost:4173` (or wherever `index.html` is
   served). **Authorization callback URL**: your `oauth-callback.html` URL,
   e.g. `http://localhost:4173/oauth-callback.html`.
3. Copy the generated **Client ID**, then **Generate a new client secret**
   and copy that too.

## 3. Configure the Cratebase server

Set the four OAuth2 env vars from the repo root's `.env.example` — a
provider only shows up in `listAuthMethods()` once both of its vars are
set:

```bash
# .env (next to the cratebase binary / crates/server)
OAUTH_GOOGLE_CLIENT_ID=your-google-client-id
OAUTH_GOOGLE_CLIENT_SECRET=your-google-client-secret
OAUTH_GITHUB_CLIENT_ID=your-github-client-id
OAUTH_GITHUB_CLIENT_SECRET=your-github-client-secret
```

Restart `cratebase serve` after editing `.env`. Password-reset and
verification emails work out of the box in dev without any mailer
configured — unset `MAIL_DRIVER` just logs the email (including the
token/link) to the server's stdout instead of sending it, which is enough
to copy the token into this demo's "confirm" forms.

## 4. Serve it locally

This is a static site with ESM imports resolved via the import map (bare
specifier `"cratebase"` → `../../sdk/js/dist/index.js`), so it must be
served over `http://`, not opened as a `file://` URL (browsers block ES
module imports from `file://`).
From the repo root:

```bash
# build the SDK once so dist/index.js exists
cd sdk/js && npm install && npm run build && cd ../..

# serve the example (any static file server works)
npx serve examples/auth-demo -l 4173
# or: python3 -m http.server 4173 --directory examples/auth-demo
```

Then open `http://localhost:4173/`. The "Cratebase server URL" field at
the top defaults to `http://localhost:8090` and is saved to
`localStorage`, independent of whatever port the demo's own static files
are served on — start `cratebase serve` on 8090 (or point the field at
wherever it actually runs) with `CORS_ALLOW_ORIGINS` permitting the
demo's origin (default `*` already does).

## What each flow does, and how it maps to the SDK

| UI action | SDK call |
| --- | --- |
| Register | `collection('users').create({ email, password, passwordConfirm })` then `authWithPassword` |
| Log in | `collection('users').authWithPassword(identity, password)` |
| OAuth2 button | `listAuthMethods(redirectUri)` → render a button per `providers[].authUrl`; click navigates to it |
| OAuth2 callback | `authWithOAuth2(provider, code, redirectUri)` |
| Forgot password | `requestPasswordReset(email)` then `confirmPasswordReset(token, password, passwordConfirm)` |
| Resend verification | `requestVerification(email)` |
| Confirm verification | `confirmVerification(token)` then `authRefresh()` to pick up `verified: true` |
| Change email | `requestEmailChange(newEmail)` (authenticated) then `confirmEmailChange(token)` then `authRefresh()` |
| Logout | `authStore.clear()` |

`authStore` persists the token/record to `localStorage` and its
`onChange` listener drives which of the two views (`#view-guest` /
`#view-auth`) is shown — the same reactive pattern the SDK's `AuthStore`
is designed for.

## Verification performed

- `node --check app.js` and `node --check oauth-callback.js` both pass
  (syntax only — see below).
- Manually traced every code path against `sdk/js/src/record-service.ts`,
  `client.ts`, `auth-store.ts`, and `error.ts` to match method names,
  argument order, and response shapes exactly (`AuthResponse<T>`,
  `AuthMethodsResponse`, `ClientResponseError.data` field errors, etc.).
- Confirmed `sdk/js/dist/index.js` exports `Cratebase` and
  `ClientResponseError` (the two imports this demo uses) via
  `sdk/js/src/index.ts`.

**Not verified**: no browser was used, and no real Google/GitHub OAuth2
app or SMTP/Resend mailer was configured, so the live register → login →
OAuth2-redirect → callback → email-verification/reset round trips were
**not exercised end to end**. Correctness there rests on code review
against the SDK source and the API contract in `openapi.yaml`, not on an
observed run.
