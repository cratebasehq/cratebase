//! Incoming webhooks: verifying an *external* service's signature before
//! trusting a payload it POSTs at us — the mirror image of
//! [`crate::webhooks`], which signs and sends *outgoing* notifications
//! this process originates. Nothing here reads or writes `_webhooks`;
//! that collection is exclusively about the outgoing direction.
//!
//! # The reusable part vs. the worked example
//!
//! [`verify_hmac_sha256_hex`] is the whole reusable primitive: given a
//! shared secret, the exact bytes a sender claims to have signed, and the
//! hex signature it sent, it says yes or no in constant time. Every
//! HMAC-based webhook scheme (Stripe, GitHub's `X-Hub-Signature-256`,
//! this codebase's own `X-Cratebase-Signature` in [`crate::webhooks`])
//! boils down to "compute HMAC-SHA256 over some canonical byte string,
//! compare hex digests" — the only thing that differs between providers
//! is *what* the canonical byte string is and *which header* it travels
//! in. A Rust [`crate::plugin::Plugin`] wanting to verify some other
//! provider's webhook calls this function directly; the doc comment above
//! [`stripe_webhook`] shows the exact shape (parse header → build
//! `signed_content` → call this function).
//!
//! [`verify_stripe_signature`] is that composition already done for
//! Stripe specifically: it parses `Stripe-Signature`'s
//! `t=<unix-seconds>,v1=<hex>[,v1=<hex>...]` format, builds
//! `"<timestamp>.<raw body>"` as the signed content (Stripe's own
//! `signed_payload`, see their docs), and additionally rejects a
//! timestamp older than `tolerance` even when the hex digest matches —
//! see "Replay protection" below. [`stripe_webhook`] is the one concrete
//! HTTP endpoint wired into the router, demonstrating the whole flow
//! end to end against a real provider's exact wire format.
//!
//! # Reachable from JavaScript too, with no new host API
//!
//! A `pb_hooks` file registers its own inbound-webhook endpoint with the
//! existing `routerAdd` mechanism (see `cratebase_jsvm`'s module doc and
//! `crate::jsvm_host`), and the JS runtime already exposes
//! `$security.hs256(data, secret)` — the exact HMAC-SHA256-as-hex
//! primitive [`verify_hmac_sha256_hex`] wraps on the Rust side (see
//! `cratebase_jsvm::security::hmac_hex`). So the Stripe recipe needs no
//! new binding to be reusable from JS; a hook file replicates it in a few
//! lines:
//!
//! ```js
//! routerAdd("POST", "/webhooks/stripe", (e) => {
//!   const sig = e.request.header.get("Stripe-Signature") || "";
//!   const parts = Object.fromEntries(
//!     sig.split(",").map((p) => p.split("=").map((s) => s.trim()))
//!   );
//!   const secret = $os.getenv("STRIPE_WEBHOOK_SECRET");
//!   const body = e.request.body; // raw bytes read by the runtime
//!   const signedContent = parts.t + "." + body;
//!   const expected = $security.hs256(signedContent, secret);
//!   const age = Math.floor(Date.now() / 1000) - Number(parts.t);
//!   if (expected !== parts.v1 || age > 300 || age < -300) {
//!     throw new BadRequestError("invalid webhook signature");
//!   }
//!   // signature verified — safe to act on the payload now.
//! });
//! ```
//!
//! This module's `examples/incoming-webhooks-stripe/README.md` note
//! (repo root `examples/`) writes this out in full with commentary; it
//! is deliberately *not* duplicated as a second Rust binding, because
//! there is nothing left for the host to implement that `$security.hs256`
//! doesn't already cover.
//!
//! # Replay protection
//!
//! A signature alone only proves *the secret holder produced this
//! digest at some point* — it says nothing about *when*. Without a
//! freshness check, a captured request (from a proxy log, a
//! man-in-the-middle before TLS was in place, a compromised intermediary)
//! stays forever replayable even though its signature is technically
//! valid. Stripe's own scheme defends against this by binding the
//! timestamp into the signed content (so a copied signature cannot be
//! reattached to a different timestamp) and asking receivers to reject
//! anything outside a tolerance window; [`verify_stripe_signature`]
//! enforces that window itself rather than leaving it to the caller, so
//! forgetting the check is not an option. [`DEFAULT_TOLERANCE`] (5
//! minutes) matches Stripe's own official libraries' default.
//!
//! # Trust boundary: the signature *is* the authentication
//!
//! Unlike every other route this crate mounts under `/api`, this
//! endpoint takes no `Authorization` header and no API key — an external
//! service cannot authenticate as a Cratebase superuser or record, and
//! shouldn't have to. [`verify_stripe_signature`] *is* the auth check:
//! anything that doesn't produce a valid, fresh HMAC digest with the
//! shared secret is rejected before the body is ever parsed as JSON or
//! acted upon, exactly as if it had failed a password check.
//!
//! # Why the example secret is an environment variable, not `Settings`
//!
//! `Settings` (`cratebase_core::Settings`, see `Sms`/`Llm`/`Push`) is for
//! *operator-editable* integration config: a dashboard user flips
//! `enabled` and fills in credentials for a business capability this
//! server actively uses (sending SMS, calling an LLM, pushing
//! notifications) through `PATCH /api/settings`, and the value is
//! persisted in `_params` so it survives redeploys and is editable
//! without touching the environment. `STRIPE_WEBHOOK_SECRET` is the
//! opposite shape: it authenticates *inbound* requests from one specific
//! external integration a self-hoster wires up once at deploy time
//! (generated by Stripe's dashboard/CLI when the endpoint URL is
//! registered), never rotated through this app's own admin UI, and
//! specific to a single worked example rather than a first-class
//! capability like SMS/LLM/Push that the rest of the server calls into.
//! That is exactly the shape `crate::config`'s module doc already
//! reserves for `.env`-style values (see e.g. `CB_SECRET`) rather than
//! `Settings`, so [`stripe_webhook`] reads it straight from the process
//! environment instead of adding a single-purpose `Settings` field (and
//! the secret-stripping `to_public_json` entry it would need) for a
//! demonstration endpoint with no dashboard surface of its own. A real
//! integration with more than one field (multiple event types, a
//! dashboard toggle, delivery logs) would earn its own `Settings` block
//! and likely its own system collection — explicitly out of scope here.
//!
//! # Why the header is parsed with an allocation-light hand rolled parser
//!
//! `Stripe-Signature` is a short, fixed-shape header
//! (`t=169...,v1=abc...`); pulling in a dedicated parsing crate for it
//! would be pure overhead. [`parse_stripe_signature_header`] splits on
//! `,` then `=` the same way `crate::webhooks::format_payload` reasons
//! about `url`'s host: bespoke, but proportionate to how small the format
//! is.

use std::time::Duration;

use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::Router;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use cratebase_core::AppError;

use crate::app::App;
use crate::http_error::{ApiError, ApiResult};

/// Stripe's own official libraries default to a 5 minute tolerance; kept
/// here as the default for [`stripe_webhook`] so the example endpoint and
/// its tests agree on one number.
pub const DEFAULT_TOLERANCE: Duration = Duration::from_secs(300);

/// The environment variable [`stripe_webhook`] reads its signing secret
/// from — see the module doc's "Why the example secret is an environment
/// variable" section.
pub const STRIPE_WEBHOOK_SECRET_ENV: &str = "STRIPE_WEBHOOK_SECRET";

/// Why an inbound signature failed to verify. Deliberately distinct from
/// [`AppError`]: this type is the reusable helper's own vocabulary, so a
/// caller that isn't an HTTP handler (a [`crate::plugin::Plugin`]
/// deciding whether to trust a payload before doing something
/// irreversible with it) isn't forced to think in terms of HTTP status
/// codes to handle it. [`stripe_webhook`] is the one caller that turns it
/// into an [`AppError::BadRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureError {
    /// The signature header isn't shaped the way this scheme expects
    /// (missing a required part, or a `v1`/signature value that isn't
    /// valid hex). Carries a human-readable reason.
    Malformed(String),
    /// The header parsed fine, but no signature it carried matches the
    /// HMAC this process computes over the signed content with the
    /// configured secret.
    Mismatch,
    /// The signature matches, but its timestamp is outside the allowed
    /// tolerance — see the module doc's "Replay protection" section.
    Stale,
}

impl std::fmt::Display for SignatureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignatureError::Malformed(reason) => write!(f, "malformed signature header: {reason}"),
            SignatureError::Mismatch => write!(f, "signature does not match"),
            SignatureError::Stale => write!(
                f,
                "signature timestamp is outside the allowed tolerance (stale or replayed)"
            ),
        }
    }
}

impl std::error::Error for SignatureError {}

/// The reusable primitive: does `hex_signature` equal
/// `hex(HMAC-SHA256(secret, signed_content))`, checked in constant time?
///
/// This is the one function every HMAC-based inbound webhook scheme
/// reduces to — see the module doc's "The reusable part vs. the worked
/// example" section. `signed_content` is whatever byte string the
/// provider's docs say it signs (for Stripe, that is
/// `"<timestamp>.<raw body>"`, built by [`verify_stripe_signature`]; for
/// a scheme that signs the raw body alone, `signed_content` is just the
/// body).
///
/// Comparison goes through [`Mac::verify_slice`] rather than `==` on two
/// byte slices: `hmac::Mac`'s implementation compares in constant time,
/// so how much of the digest matched before the first differing byte
/// never leaks through response-timing differences — the same property
/// `subtle`-style constant-time comparisons exist for, without needing
/// that crate as a dependency.
pub fn verify_hmac_sha256_hex(
    secret: &[u8],
    signed_content: &[u8],
    hex_signature: &str,
) -> Result<(), SignatureError> {
    let expected = decode_hex(hex_signature)
        .ok_or_else(|| SignatureError::Malformed("signature is not valid hex".into()))?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts a key of any length");
    mac.update(signed_content);
    mac.verify_slice(&expected)
        .map_err(|_| SignatureError::Mismatch)
}

/// `"abc123"` → `[0xab, 0xc1, 0x23]`. No `hex` crate dependency for the
/// same reason [`crate::webhooks::deliver`] hand-encodes its own
/// signature with `format!("{b:02x}")` instead of pulling one in for the
/// other direction: it is three lines either way.
fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// `Stripe-Signature`'s own shape: `t=<unix-seconds>,v1=<hex>[,v1=<hex>]`.
/// Stripe sends more than one `v1` value while a webhook endpoint's
/// signing secret is being rotated (the old and new secret both sign the
/// same event during the overlap window), so a match against *any* of
/// them is accepted — matching Stripe's own SDKs.
struct StripeSignatureHeader {
    timestamp: i64,
    v1_signatures: Vec<String>,
}

fn parse_stripe_signature_header(header: &str) -> Result<StripeSignatureHeader, SignatureError> {
    let mut timestamp: Option<i64> = None;
    let mut v1_signatures = Vec::new();
    for part in header.split(',') {
        let part = part.trim();
        let Some((key, value)) = part.split_once('=') else {
            return Err(SignatureError::Malformed(format!(
                "expected \"key=value\", got {part:?}"
            )));
        };
        match key.trim() {
            "t" => {
                timestamp = Some(value.trim().parse::<i64>().map_err(|_| {
                    SignatureError::Malformed(format!("\"t\" is not an integer: {value:?}"))
                })?);
            }
            "v1" => v1_signatures.push(value.trim().to_string()),
            // Stripe reserves other prefixes (e.g. `v0` for an older,
            // deprecated scheme) for future/legacy use; this verifier
            // only ever checks `v1`, so anything else is ignored rather
            // than rejected, the same tolerance Stripe's own libraries
            // extend to unknown scheme versions.
            _ => {}
        }
    }
    let timestamp =
        timestamp.ok_or_else(|| SignatureError::Malformed("missing \"t\" field".into()))?;
    if v1_signatures.is_empty() {
        return Err(SignatureError::Malformed("missing \"v1\" field".into()));
    }
    Ok(StripeSignatureHeader {
        timestamp,
        v1_signatures,
    })
}

/// The full Stripe verification flow: parse `header`, reject a timestamp
/// older (or, defensively, newer — a clock-skewed forgery is just as
/// invalid) than `tolerance` away from `now_unix`, then check `raw_body`
/// against every `v1` signature the header carried via
/// [`verify_hmac_sha256_hex`] over Stripe's own signed-content shape,
/// `"<timestamp>.<raw body>"`.
///
/// `now_unix` is a parameter rather than read from the clock internally
/// so tests can exercise the tolerance boundary deterministically instead
/// of racing a real clock; [`stripe_webhook`] is the only caller that
/// passes the real time.
pub fn verify_stripe_signature(
    header: &str,
    raw_body: &[u8],
    secret: &[u8],
    tolerance: Duration,
    now_unix: i64,
) -> Result<(), SignatureError> {
    let parsed = parse_stripe_signature_header(header)?;

    let age = now_unix - parsed.timestamp;
    let tolerance_secs = tolerance.as_secs() as i64;
    if age.unsigned_abs() > tolerance_secs as u64 {
        return Err(SignatureError::Stale);
    }

    let mut signed_content = parsed.timestamp.to_string().into_bytes();
    signed_content.push(b'.');
    signed_content.extend_from_slice(raw_body);

    let mut last_err = SignatureError::Mismatch;
    for candidate in &parsed.v1_signatures {
        match verify_hmac_sha256_hex(secret, &signed_content, candidate) {
            Ok(()) => return Ok(()),
            Err(e) => last_err = e,
        }
    }
    Err(last_err)
}

/// `POST /api/webhooks/stripe` — the one concrete worked example: verify
/// a Stripe event's signature end to end, then act on it. See the module
/// doc for the full design rationale.
pub fn router() -> Router<App> {
    Router::new().route("/webhooks/stripe", post(stripe_webhook))
}

async fn stripe_webhook(headers: HeaderMap, body: Bytes) -> ApiResult<StatusCode> {
    let secret = std::env::var(STRIPE_WEBHOOK_SECRET_ENV).map_err(|_| {
        ApiError(AppError::internal(format!(
            "{STRIPE_WEBHOOK_SECRET_ENV} is not configured"
        )))
    })?;
    let header = headers
        .get("Stripe-Signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError(AppError::bad_request("missing Stripe-Signature header")))?;

    verify_stripe_signature(
        header,
        &body,
        secret.as_bytes(),
        DEFAULT_TOLERANCE,
        chrono::Utc::now().timestamp(),
    )
    .map_err(|e| ApiError(AppError::bad_request(e.to_string())))?;

    // The signature is verified — only now is it safe to parse the body
    // as JSON and act on it. A real integration would branch on
    // `event["type"]` here (`checkout.session.completed`,
    // `invoice.paid`, ...); this worked example just proves the payload
    // was trustworthy enough to log.
    let event: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|_| ApiError(AppError::bad_request("invalid JSON body")))?;
    tracing::info!(
        event_type = event.get("type").and_then(serde_json::Value::as_str),
        "verified incoming Stripe webhook"
    );

    Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(secret: &str, signed_content: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(signed_content);
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn stripe_header(secret: &str, timestamp: i64, body: &[u8]) -> String {
        let mut signed_content = timestamp.to_string().into_bytes();
        signed_content.push(b'.');
        signed_content.extend_from_slice(body);
        format!("t={timestamp},v1={}", sign(secret, &signed_content))
    }

    #[test]
    fn valid_signature_accepted() {
        let secret = "whsec_test";
        let body = br#"{"id":"evt_1","type":"checkout.session.completed"}"#;
        let now = 1_700_000_000i64;
        let header = stripe_header(secret, now, body);

        assert!(
            verify_stripe_signature(&header, body, secret.as_bytes(), DEFAULT_TOLERANCE, now,)
                .is_ok()
        );
    }

    #[test]
    fn tampered_body_rejected() {
        let secret = "whsec_test";
        let now = 1_700_000_000i64;
        let header = stripe_header(secret, now, b"original body");

        let err = verify_stripe_signature(
            &header,
            b"tampered body",
            secret.as_bytes(),
            DEFAULT_TOLERANCE,
            now,
        )
        .unwrap_err();
        assert_eq!(err, SignatureError::Mismatch);
    }

    #[test]
    fn wrong_secret_rejected() {
        let now = 1_700_000_000i64;
        let body = b"payload";
        let header = stripe_header("whsec_correct", now, body);

        let err = verify_stripe_signature(&header, body, b"whsec_wrong", DEFAULT_TOLERANCE, now)
            .unwrap_err();
        assert_eq!(err, SignatureError::Mismatch);
    }

    #[test]
    fn old_timestamp_rejected_as_replay_even_with_valid_signature() {
        let secret = "whsec_test";
        let body = b"payload";
        let signed_at = 1_700_000_000i64;
        // A technically-valid signature for a request signed 10 minutes
        // ago — well outside the 5 minute default tolerance.
        let header = stripe_header(secret, signed_at, body);
        let now = signed_at + 600;

        let err = verify_stripe_signature(&header, body, secret.as_bytes(), DEFAULT_TOLERANCE, now)
            .unwrap_err();
        assert_eq!(
            err,
            SignatureError::Stale,
            "must reject as a replay, not accept"
        );
    }

    #[test]
    fn timestamp_from_the_future_beyond_tolerance_rejected() {
        // Defensive symmetry: a forged/clock-skewed future timestamp is
        // just as invalid as a stale one.
        let secret = "whsec_test";
        let body = b"payload";
        let signed_at = 1_700_000_000i64;
        let header = stripe_header(secret, signed_at, body);
        let now = signed_at - 600;

        let err = verify_stripe_signature(&header, body, secret.as_bytes(), DEFAULT_TOLERANCE, now)
            .unwrap_err();
        assert_eq!(err, SignatureError::Stale);
    }

    #[test]
    fn timestamp_within_tolerance_accepted() {
        let secret = "whsec_test";
        let body = b"payload";
        let signed_at = 1_700_000_000i64;
        let header = stripe_header(secret, signed_at, body);
        let now = signed_at + 299; // just inside the 300s default

        assert!(
            verify_stripe_signature(&header, body, secret.as_bytes(), DEFAULT_TOLERANCE, now,)
                .is_ok()
        );
    }

    #[test]
    fn malformed_header_missing_timestamp_rejected() {
        let err = verify_stripe_signature(
            "v1=abcdef",
            b"payload",
            b"secret",
            DEFAULT_TOLERANCE,
            1_700_000_000,
        )
        .unwrap_err();
        assert!(matches!(err, SignatureError::Malformed(_)), "{err:?}");
    }

    #[test]
    fn malformed_header_missing_v1_rejected() {
        let err = verify_stripe_signature(
            "t=1700000000",
            b"payload",
            b"secret",
            DEFAULT_TOLERANCE,
            1_700_000_000,
        )
        .unwrap_err();
        assert!(matches!(err, SignatureError::Malformed(_)), "{err:?}");
    }

    #[test]
    fn malformed_header_garbage_rejected() {
        let err = verify_stripe_signature(
            "not even close to the right shape",
            b"payload",
            b"secret",
            DEFAULT_TOLERANCE,
            1_700_000_000,
        )
        .unwrap_err();
        assert!(matches!(err, SignatureError::Malformed(_)), "{err:?}");
    }

    #[test]
    fn malformed_header_non_hex_signature_rejected() {
        let err = verify_stripe_signature(
            "t=1700000000,v1=not-hex-at-all",
            b"payload",
            b"secret",
            DEFAULT_TOLERANCE,
            1_700_000_000,
        )
        .unwrap_err();
        assert!(matches!(err, SignatureError::Malformed(_)), "{err:?}");
    }

    #[test]
    fn non_integer_timestamp_rejected() {
        let err = verify_stripe_signature(
            "t=not-a-number,v1=abcdef",
            b"payload",
            b"secret",
            DEFAULT_TOLERANCE,
            1_700_000_000,
        )
        .unwrap_err();
        assert!(matches!(err, SignatureError::Malformed(_)), "{err:?}");
    }

    #[test]
    fn rotation_accepts_a_match_on_any_v1_value() {
        // During secret rotation Stripe sends two `v1` values, one per
        // secret. Only the second matches this verifier's configured
        // (new) secret.
        let old_secret = "whsec_old";
        let new_secret = "whsec_new";
        let now = 1_700_000_000i64;
        let body = b"payload";

        let mut signed_content = now.to_string().into_bytes();
        signed_content.push(b'.');
        signed_content.extend_from_slice(body);
        let header = format!(
            "t={now},v1={},v1={}",
            sign(old_secret, &signed_content),
            sign(new_secret, &signed_content),
        );

        assert!(verify_stripe_signature(
            &header,
            body,
            new_secret.as_bytes(),
            DEFAULT_TOLERANCE,
            now,
        )
        .is_ok());
    }

    #[test]
    fn generic_helper_rejects_invalid_hex_signature() {
        let err = verify_hmac_sha256_hex(b"secret", b"content", "zz").unwrap_err();
        assert!(matches!(err, SignatureError::Malformed(_)), "{err:?}");
    }

    #[test]
    fn generic_helper_accepts_matching_digest() {
        let signed_content = b"hello world";
        let hex_sig = sign("secret", signed_content);
        assert!(verify_hmac_sha256_hex(b"secret", signed_content, &hex_sig).is_ok());
    }
}
