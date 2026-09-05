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

`crates/server/src/incoming_webhooks.rs` exports
`verify_hmac_sha256_hex(secret, signed_content, hex_signature)`: given a
shared secret, the exact bytes a sender claims to have signed, and the hex
digest it sent, it answers "trust this or not" in constant time (via
`hmac::Mac::verify_slice`, so a timing side-channel can't leak how much of
the digest matched). This is *the* primitive every HMAC-based inbound
webhook scheme reduces to — Stripe, GitHub's `X-Hub-Signature-256`, and this
codebase's own outgoing `X-Cratebase-Signature` are all "HMAC-SHA256 over
some canonical byte string, compare hex digests," differing only in what
that byte string is and which header it travels in.

A Rust `crate::plugin::Plugin` that wants to verify some *other* provider's
webhook calls `verify_hmac_sha256_hex` directly. A `pb_hooks` JS file needs
no new binding at all: the JS runtime already exposes
`$security.hs256(data, secret)` — the exact same HMAC-SHA256-as-hex
computation `verify_hmac_sha256_hex` wraps on the Rust side (both call down
to the same `hmac`/`sha2` crates; see `cratebase_jsvm::security::hmac_hex`).

## Stripe specifically

`Stripe-Signature` looks like:

```
Stripe-Signature: t=1614556800,v1=5257a869e7bfe...
```

`t` is a Unix timestamp; `v1` is `hex(HMAC-SHA256(secret, "<t>.<raw body>"))`.
Stripe sends more than one `v1` value while a webhook signing secret is
being rotated (old and new secret both sign the same event during the
overlap window) — a match against *any* one is valid.

`crates/server/src/incoming_webhooks.rs`'s `verify_stripe_signature` does
exactly this: parse the header, reject if the timestamp is more than
`DEFAULT_TOLERANCE` (5 minutes — Stripe's own libraries' default) away from
now in *either* direction, then check the raw body against every `v1`
signature via `verify_hmac_sha256_hex`. The timestamp check exists because a
signature alone only proves "the secret holder produced this digest at some
point," not *when* — without it, a captured request stays replayable
forever even though its signature is technically valid.

## The worked endpoint

`POST /api/webhooks/stripe` (`incoming_webhooks::stripe_webhook`, wired into
the router in `crates/server/src/routes/mod.rs`) is the endpoint doing this
end to end:

1. Read the signing secret from the `STRIPE_WEBHOOK_SECRET` environment
   variable (see the module doc for why this is an env var and not a
   `Settings` field — short version: it's a deploy-time credential for one
   external integration, the same shape as `CB_SECRET` in
   `crates/server/src/config.rs`, not an operator-editable dashboard toggle
   like SMTP/SMS/LLM/Push).
2. Read the raw request body and the `Stripe-Signature` header.
3. Call `verify_stripe_signature`. A missing header, malformed header,
   signature mismatch, or stale timestamp all come back as `400 Bad
   Request` before the body is ever parsed as JSON.
4. Only once verification succeeds does it parse the body and act on it
   (this example just logs the event type — a real integration would
   switch on `event["type"]`, e.g. `checkout.session.completed`).

Note this endpoint deliberately has **no** `Authorization` header and no API
key: an external service can't authenticate as a Cratebase superuser or
record, and shouldn't have to. The signature check *is* the authentication.

### Try it locally

```bash
export STRIPE_WEBHOOK_SECRET=whsec_test_secret
cratebase serve &

body='{"id":"evt_1","type":"checkout.session.completed"}'
ts=$(date +%s)
sig=$(printf '%s.%s' "$ts" "$body" \
  | openssl dgst -sha256 -hmac "$STRIPE_WEBHOOK_SECRET" \
  | sed 's/^.* //')

curl -i http://localhost:8090/api/webhooks/stripe \
  -H "Stripe-Signature: t=${ts},v1=${sig}" \
  -H 'content-type: application/json' \
  --data-raw "$body"
# 200 OK

curl -i http://localhost:8090/api/webhooks/stripe \
  -H "Stripe-Signature: t=${ts},v1=deadbeef" \
  -H 'content-type: application/json' \
  --data-raw "$body"
# 400 Bad Request — signature does not match
```

### The same recipe from a `pb_hooks` JS file

No Rust changes needed to add a second signed-inbound-webhook endpoint —
any `pb_hooks/*.pb.js` file gets there with `routerAdd` and the existing
`$security.hs256` primitive:

```js
routerAdd("POST", "/webhooks/stripe", (e) => {
  const sig = e.request.header.get("Stripe-Signature") || "";
  const parts = Object.fromEntries(
    sig.split(",").map((p) => p.split("=").map((s) => s.trim()))
  );
  const secret = $os.getenv("STRIPE_WEBHOOK_SECRET");
  const body = e.request.body; // raw bytes read by the runtime
  const signedContent = parts.t + "." + body;
  const expected = $security.hs256(signedContent, secret);
  const age = Math.floor(Date.now() / 1000) - Number(parts.t);

  if (expected !== parts.v1 || Math.abs(age) > 300) {
    throw new BadRequestError("invalid webhook signature");
  }

  const event = JSON.parse(body);
  console.log("verified Stripe webhook:", event.type);
});
```

## What's deliberately out of scope

* A `_incoming_webhooks` system collection (dashboard-configurable inbound
  endpoints, per-endpoint secret rotation, delivery logs) — this note ships
  the verification *primitive* plus one concrete, hardcoded endpoint;
  turning that into a generic no-code feature the way `_webhooks` covers
  the outgoing direction is a larger, separate piece of work.
* Idempotency / dedup on `event.id` — Stripe (like most providers) can
  retry a webhook it didn't get a 2xx for; a production integration should
  track processed event IDs before acting on one twice. That's
  application-level state (a collection storing seen `event.id`s), not
  something the signature-verification primitive itself needs to own.
