# Cratebase auth demo

Static ESM app (no build step) exercising every auth flow Cratebase exposes
on the default `users` auth collection: register, password login, OAuth2
(Google/GitHub), password reset, email verification, and email change.

> **OAuth2 login doesn't complete yet.** Cratebase's API is byte-compatible
> with PocketBase v0.23+, but the server's OAuth2/OTP/MFA routes
> (`auth-with-oauth2`, `request-otp`, `auth-with-otp`, `impersonate`) are
> still being implemented — `listAuthMethods()` works, so the OAuth2
> buttons render, but clicking one through to a real provider and back
> will 404 on the token exchange. Register, password login, password
> reset, and email verification/change all work today.

`index.html` and `oauth-callback.html` both declare an [import map](https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/script/type/importmap)
mapping the bare specifier `"pocketbase"` to the official PocketBase JS
SDK on esm.sh, so both `app.js` and `oauth-callback.js` can write
`import PocketBase, { ClientResponseError } from "pocketbase";` — the
same zero-build-step approach every other `examples/*` app uses.

Files:

- `index.html` / `app.js` — the main app: guest view (register, login,
  OAuth2 buttons, forgot-password) and authenticated view (profile,
  logout, verification, email change).
- `oauth-callback.html` / `oauth-callback.js` — the OAuth2 redirect
  target. Reads `?code=` off the URL, exchanges it via
  `authWithOAuth2Code`, and bounces back to `index.html`.
- `style.css` — shared styling for both pages.

## 1. Configure the `users` collection

Cratebase ships a default `users` auth collection with `identityField:
"email"` already. To confirm/adjust it (or recreate it), authenticate as a
superuser and `PATCH` the collection:

```bash
# Superuser login — replace with your own superuser credentials.
# PocketBase v0.23+ dropped /api/admins/* in favour of the _superusers
# auth collection.
ADMIN_TOKEN=$(curl -s http://localhost:8090/api/collections/_superusers/auth-with-password \
  -H 'content-type: application/json' \
  -d '{"identity":"admin@example.com","password":"adminpassword"}' | jq -r .token)

# Inspect the current collection.
curl -s http://localhost:8090/api/collections/users \
  -H "authorization: Bearer $ADMIN_TOKEN" | jq .

# Make sure identityFields includes "email" (the default — only needed if
# you changed it). This demo never blocks login on `verified`, so there's
# no requireEmailVerification toggle here — the "confirm verification"
# step below just flips `record.verified` to `true` and this page
# displays it.
curl -s -X PATCH http://localhost:8090/api/collections/users \
  -H "authorization: Bearer $ADMIN_TOKEN" \
  -H 'content-type: application/json' \
  -d '{
        "passwordAuth": {
          "identityFields": ["email"]
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
specifier `"pocketbase"` → `https://esm.sh/pocketbase@0.28`), so it must
be served over `http://`, not opened as a `file://` URL (browsers block
ES module imports from `file://`).

```bash
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
| OAuth2 button | `listAuthMethods()` → render a button per provider; click navigates to `provider.authURL + encodeURIComponent(redirectUri)` (PKCE `codeVerifier` stashed alongside for the callback) |
| OAuth2 callback | `authWithOAuth2Code(provider, code, codeVerifier, redirectUri)` |
| Forgot password | `requestPasswordReset(email)` then `confirmPasswordReset(token, password, passwordConfirm)` |
| Resend verification | `requestVerification(email)` |
| Confirm verification | `confirmVerification(token)` then `authRefresh()` to pick up `verified: true` |
| Change email | `requestEmailChange(newEmail)` (authenticated) then `confirmEmailChange(token, currentPassword)` then `authRefresh()` |
| Logout | `authStore.clear()` |

`authStore` persists the token/record to `localStorage` and its
`onChange` listener drives which of the two views (`#view-guest` /
`#view-auth`) is shown — the same reactive pattern the SDK's `AuthStore`
is designed for.

## Verification performed

- `node --check app.js` and `node --check oauth-callback.js` both pass
  (syntax only — see below).
- Manually traced every code path against the official `pocketbase` npm
  package's own type declarations and README to match method names,
  argument order, and response shapes exactly (`RecordAuthResponse<T>`,
  `AuthMethodsList`, `ClientResponseError.data` field errors, etc.).

**Not verified**: no browser was used, and no real Google/GitHub OAuth2
app or SMTP/Resend mailer was configured, so the live register → login →
password-reset/email-verification round trips were **not exercised end
to end**, and the OAuth2 redirect → callback exchange can't be exercised
at all yet — the server doesn't implement those routes (see the note at
the top of this file). Correctness rests on code review against the
`pocketbase` SDK and Cratebase's own route table, not on an observed run.
