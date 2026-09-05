# Incoming webhooks: verifying Stripe's signature

Cratebase ships an *outgoing* webhook feature out of the box (`_webhooks`,
see `crates/server/src/webhooks.rs`): configure a URL from the dashboard and
this server POSTs to it whenever a record event fires, optionally signing
the body with `X-Cratebase-Signature`. That covers Cratebase notifying
someone else. It says nothing about the other direction — an external
service (Stripe, GitHub, a payment processor, another Cratebase instance's
own outgoing webhook) POSTing *at* Cratebase and this server needing to
trust the payload before acting on it.

This note is the worked pattern for that direction, using Stripe as the
concrete example because its `Stripe-Signature` scheme is the de facto
reference design most other HMAC-webhook providers copy.

## The reusable primitive

The embedded JS runtime (`pb_hooks/*.pb.js`, backed by `crates/jsvm`)
already exposes `$security.hs256(data, secret)`: given a shared secret and
the exact bytes a sender claims to have signed, it computes
`hex(HMAC-SHA256(secret, data))`. This is *the* primitive every HMAC-based
inbound webhook scheme reduces to — Stripe, GitHub's
`X-Hub-Signature-256`, and this codebase's own outgoing
`X-Cratebase-Signature` are all "HMAC-SHA256 over some canonical byte
string, compare hex digests," differing only in what that byte string is
and which header it travels in. No native Rust binding is needed for this:
a `pb_hooks` JS file gets there with the existing `$security.hs256` alone.

A Rust `crate::plugin::Plugin` that wants to verify some *other*
provider's webhook from native code can reach for the same primitive via
`hmac`/`sha2` directly (see `cratebase_jsvm::security::hmac_hex` for the
implementation `$security.hs256` wraps) — but for the common case of "one
more inbound webhook endpoint," no Rust changes are needed at all.

## Stripe specifically

`Stripe-Signature` looks like:

```
Stripe-Signature: t=1614556800,v1=5257a869e7bfe...
```

`t` is a Unix timestamp; `v1` is `hex(HMAC-SHA256(secret, "<t>.<raw body>"))`.
Stripe sends more than one `v1` value while a webhook signing secret is
being rotated (old and new secret both sign the same event during the
overlap window) — a match against *any* one is valid.

The recipe: parse the header, reject if the timestamp is more than 300
seconds (Stripe's own libraries' default tolerance) away from now in
*either* direction, then check the raw body against every `v1` signature
via `$security.hs256`. The timestamp check exists because a signature
alone only proves "the secret holder produced this digest at some point,"
not *when* — without it, a captured request stays replayable forever even
though its signature is technically valid.

## The worked endpoint, as a `pb_hooks` JS file

No Rust changes needed as of the `rawBody`/`header.get()` fixes landed
alongside this example: any `pb_hooks/*.pb.js` file gets a signed-inbound-
webhook endpoint with `routerAdd`, `$security.hs256`, and
`e.request.rawBody`.

```js
routerAdd("POST", "/webhooks/stripe", (e) => {
  const sig = e.request.header.get("Stripe-Signature") || "";
  const parts = Object.fromEntries(
    sig.split(",").map((p) => p.split("=").map((s) => s.trim()))
  );
  const secret = $os.getenv("STRIPE_WEBHOOK_SECRET");
  const body = e.request.rawBody; // exact bytes, before JSON parsing -- `e.request.body` is
                                   // already parsed and will never match a real signature
  const signedContent = parts.t + "." + body;
  const expected = $security.hs256(signedContent, secret);
  const age = Math.floor(Date.now() / 1000) - Number(parts.t);

  if (expected !== parts.v1 || Math.abs(age) > 300) {
    throw new BadRequestError("invalid webhook signature");
  }

  const event = JSON.parse(body);
  console.log("verified Stripe webhook:", event.type);
  // signature verified — safe to act on the payload now, e.g. switch on
  // event.type (checkout.session.completed, ...).
});
```

Note this endpoint deliberately has **no** `Authorization` header and no API
key: an external service can't authenticate as a Cratebase superuser or
record, and shouldn't have to. The signature check *is* the authentication.

### Try it locally

Drop the hook above into `pb_hooks/stripe.pb.js` next to your data
directory, then:

```bash
export STRIPE_WEBHOOK_SECRET=whsec_test_secret
cratebase serve &

body='{"id":"evt_1","type":"checkout.session.completed"}'
ts=$(date +%s)
sig=$(printf '%s.%s' "$ts" "$body" \
  | openssl dgst -sha256 -hmac "$STRIPE_WEBHOOK_SECRET" \
  | sed 's/^.* //')

curl -i http://localhost:8090/webhooks/stripe \
  -H "Stripe-Signature: t=${ts},v1=${sig}" \
  -H 'content-type: application/json' \
  --data-raw "$body"
# 200 OK

curl -i http://localhost:8090/webhooks/stripe \
  -H "Stripe-Signature: t=${ts},v1=deadbeef" \
  -H 'content-type: application/json' \
  --data-raw "$body"
# 400 Bad Request — signature does not match
```

(`pb_hooks` `routerAdd` routes mount at the root, not under `/api` — same
as PocketBase.)

## What's deliberately out of scope

* A `_incoming_webhooks` system collection (dashboard-configurable inbound
  endpoints, per-endpoint secret rotation, delivery logs) — this note ships
  the verification *pattern* for one concrete, hardcoded endpoint; turning
  that into a generic no-code feature the way `_webhooks` covers the
  outgoing direction is a larger, separate piece of work.
* Idempotency / dedup on `event.id` — Stripe (like most providers) can
  retry a webhook it didn't get a 2xx for; a production integration should
  track processed event IDs before acting on one twice. That's
  application-level state (a collection storing seen `event.id`s), not
  something the signature-verification primitive itself needs to own.
