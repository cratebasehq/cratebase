//! `sslmode`-aware TLS for [`crate::postgres::PostgresEngine`].
//!
//! `tokio_postgres::Config`'s own `sslmode` parsing only recognizes
//! `disable`/`prefer`/`require` — `verify-ca` and `verify-full` are both
//! ordinary libpq values but make `Config::from_str` return a parse
//! error (see [`extract_sslmode`]'s doc), which used to make a
//! Postgres-managed provider that *requires* one of them (Neon,
//! Supabase, RDS) unreachable no matter what `DATABASE_URL` said, since
//! `PostgresEngine::connect` also hard-coded `NoTls` regardless of what
//! `sslmode` parsed to. This module owns the full six-value vocabulary
//! and turns it into a `rustls::ClientConfig` (or `None` for `disable`).

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, Error as TlsError, RootCertStore,
    SignatureScheme,
};

use crate::error::DbError;

/// The six values libpq's `sslmode` can take.
/// `tokio_postgres::config::SslMode` only has three of these (`Disable`/
/// `Prefer`/`Require`) — see [`SslMode::driver_mode`] for how the other
/// three map onto it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SslMode {
    Disable,
    Allow,
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

impl SslMode {
    fn parse(raw: &str) -> Option<SslMode> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "disable" => Some(SslMode::Disable),
            "allow" => Some(SslMode::Allow),
            "prefer" => Some(SslMode::Prefer),
            "require" => Some(SslMode::Require),
            "verify-ca" => Some(SslMode::VerifyCa),
            "verify-full" => Some(SslMode::VerifyFull),
            _ => None,
        }
    }

    /// The nearest value `tokio_postgres::Config` understands: TLS is
    /// mandatory (`Require`) from `require` up, opportunistic (`Prefer`)
    /// for `allow`/`prefer`, and off for `disable`. Only *whether* TLS
    /// runs at the protocol-negotiation level is the driver's concern
    /// here — *how carefully the certificate gets checked* is entirely
    /// the `rustls::ClientConfig` built by [`client_config`], so
    /// collapsing the three "verify" levels onto `Require` loses
    /// nothing.
    pub(crate) fn driver_mode(self) -> tokio_postgres::config::SslMode {
        use tokio_postgres::config::SslMode as Driver;
        match self {
            SslMode::Disable => Driver::Disable,
            SslMode::Allow | SslMode::Prefer => Driver::Prefer,
            SslMode::Require | SslMode::VerifyCa | SslMode::VerifyFull => Driver::Require,
        }
    }
}

/// Pull `sslmode=<value>` out of a Postgres connection string — either a
/// URI (`postgres://host/db?sslmode=verify-full`) or a libpq
/// keyword/value DSN (`host=... sslmode=verify-full`) — and return the
/// string with that one parameter removed alongside the parsed mode, so
/// the remainder can still go through `tokio_postgres::Config::from_str`:
/// its own parser only recognizes `disable`/`prefer`/`require` for this
/// key and errors out on `allow`/`verify-ca`/`verify-full`, which are
/// otherwise perfectly ordinary libpq values (see the module doc).
///
/// Defaults to `prefer` — libpq's own default — when `sslmode` is
/// absent. An unrecognized value is left in the returned string
/// untouched rather than silently dropped, so `Config::from_str` still
/// produces its own clear parse error instead of this function guessing
/// at what the caller meant.
pub(crate) fn extract_sslmode(url: &str) -> (String, SslMode) {
    if let Some(qpos) = url.find('?') {
        let (base, query) = url.split_at(qpos);
        let mut mode = None;
        let mut found = false;
        let mut kept = Vec::new();
        for pair in query[1..].split('&') {
            if pair.is_empty() {
                continue;
            }
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            if key.eq_ignore_ascii_case("sslmode") {
                found = true;
                mode = SslMode::parse(value);
            } else {
                kept.push(pair);
            }
        }
        return match (found, mode) {
            (true, Some(mode)) if kept.is_empty() => (base.to_string(), mode),
            (true, Some(mode)) => (format!("{base}?{}", kept.join("&")), mode),
            // Present but unrecognized: leave the URL untouched so the
            // driver reports the bad value itself.
            (true, None) => (url.to_string(), SslMode::Prefer),
            (false, _) => (url.to_string(), SslMode::Prefer),
        };
    }

    // libpq keyword/value DSN form: space-separated `key=value` tokens,
    // never a `postgres://` URI.
    if !url.contains("://") && url.contains('=') {
        let mut mode = None;
        let mut found = false;
        let mut kept = Vec::new();
        for token in url.split_whitespace() {
            let (key, value) = token.split_once('=').unwrap_or((token, ""));
            if key.eq_ignore_ascii_case("sslmode") {
                found = true;
                mode = SslMode::parse(value.trim_matches(['\'', '"']));
            } else {
                kept.push(token);
            }
        }
        if found {
            return match mode {
                Some(mode) => (kept.join(" "), mode),
                None => (url.to_string(), SslMode::Prefer),
            };
        }
    }

    (url.to_string(), SslMode::Prefer)
}

/// A `rustls::ClientConfig` for `mode`, or `None` when TLS shouldn't run
/// at all (`disable`).
///
/// The crypto backend is pinned explicitly (`aws_lc_rs::default_provider`)
/// rather than relying on `rustls::crypto::CryptoProvider::install_default`
/// having already run somewhere else in the process: `cratebase-db` is
/// the only `rustls` consumer that builds a `ClientConfig` from scratch
/// here, and it must not depend on some other crate (or none) having
/// installed a process-wide default first.
pub(crate) fn client_config(mode: SslMode) -> Result<Option<ClientConfig>, DbError> {
    if mode == SslMode::Disable {
        return Ok(None);
    }
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let versions = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| DbError::Other(format!("postgres TLS setup failed: {e}")))?;

    let config = match mode {
        SslMode::Disable => unreachable!("handled above"),
        // `allow`/`prefer`/`require` never check the certificate at all
        // — same as libpq itself, which only validates the chain (and,
        // for `verify-full`, the hostname) starting at `verify-ca`. TLS
        // here still defeats passive eavesdropping; it just isn't proof
        // against an active MITM, exactly as these modes promise.
        SslMode::Allow | SslMode::Prefer | SslMode::Require => versions
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyCert(provider)))
            .with_no_client_auth(),
        SslMode::VerifyCa => {
            let inner = WebPkiServerVerifier::builder_with_provider(mozilla_roots(), provider)
                .build()
                .map_err(|e| DbError::Other(format!("postgres TLS setup failed: {e}")))?;
            versions
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(ChainOnly(inner)))
                .with_no_client_auth()
        }
        SslMode::VerifyFull => versions
            .with_root_certificates(mozilla_roots())
            .with_no_client_auth(),
    };
    Ok(Some(config))
}

/// Mozilla's root CA bundle, compiled in via `webpki-roots` rather than
/// read from the host's certificate store: deterministic across
/// containers that may or may not ship a system CA bundle, which is
/// exactly the class of environment a managed-Postgres deployment (Neon,
/// Supabase, RDS) tends to run in.
fn mozilla_roots() -> Arc<RootCertStore> {
    Arc::new(RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    })
}

/// `sslmode=allow|prefer|require`: encrypt, but accept whatever
/// certificate the server presents unchecked. Signatures are still
/// verified (so a corrupted or truncated handshake still fails) —
/// only the certificate's *trust chain and identity* go unchecked, which
/// is exactly what these three `sslmode` values mean in libpq itself.
#[derive(Debug)]
struct AcceptAnyCert(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// `sslmode=verify-ca`: validate the certificate's chain against
/// [`mozilla_roots`] like `verify-full` does, but — unlike `verify-full`
/// — don't require the certificate's hostname/SAN to match the address
/// dialed. Built on top of the standard [`WebPkiServerVerifier`] rather
/// than reimplementing chain validation: it already does exactly this
/// check, so this wrapper only has to recognize *that specific* failure
/// and downgrade it to success, leaving every other rejection (expired,
/// untrusted, revoked, ...) alone.
#[derive(Debug)]
struct ChainOnly(Arc<WebPkiServerVerifier>);

impl ServerCertVerifier for ChainOnly {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        match self
            .0
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
        {
            Err(TlsError::InvalidCertificate(
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
            )) => Ok(ServerCertVerified::assertion()),
            other => other,
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        self.0.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        self.0.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_prefer_when_sslmode_is_absent() {
        let (url, mode) = extract_sslmode("postgres://user:pass@host/db");
        assert_eq!(url, "postgres://user:pass@host/db");
        assert_eq!(mode, SslMode::Prefer);
    }

    #[test]
    fn parses_every_recognized_value_from_a_uri_query_string() {
        for (raw, expected) in [
            ("disable", SslMode::Disable),
            ("allow", SslMode::Allow),
            ("prefer", SslMode::Prefer),
            ("require", SslMode::Require),
            ("verify-ca", SslMode::VerifyCa),
            ("verify-full", SslMode::VerifyFull),
        ] {
            let url = format!("postgres://host/db?sslmode={raw}");
            let (stripped, mode) = extract_sslmode(&url);
            assert_eq!(mode, expected, "sslmode={raw}");
            assert_eq!(stripped, "postgres://host/db", "sslmode={raw}");
        }
    }

    #[test]
    fn strips_sslmode_case_insensitively_and_keeps_the_other_params() {
        let (url, mode) = extract_sslmode("postgres://host/db?a=1&SSLMODE=verify-full&b=2");
        assert_eq!(url, "postgres://host/db?a=1&b=2");
        assert_eq!(mode, SslMode::VerifyFull);
    }

    #[test]
    fn sslmode_as_the_only_query_param_leaves_no_dangling_question_mark() {
        let (url, mode) = extract_sslmode("postgres://host/db?sslmode=require");
        assert_eq!(url, "postgres://host/db");
        assert_eq!(mode, SslMode::Require);
    }

    #[test]
    fn unrecognized_value_is_left_in_place_for_the_driver_to_reject() {
        let (url, mode) = extract_sslmode("postgres://host/db?sslmode=bogus");
        assert_eq!(url, "postgres://host/db?sslmode=bogus");
        assert_eq!(mode, SslMode::Prefer);
    }

    #[test]
    fn parses_a_libpq_keyword_value_dsn() {
        let (dsn, mode) = extract_sslmode("host=localhost dbname=cb sslmode=verify-ca user=cb");
        assert_eq!(dsn, "host=localhost dbname=cb user=cb");
        assert_eq!(mode, SslMode::VerifyCa);
    }

    #[test]
    fn a_uri_with_no_query_string_is_untouched() {
        let (url, mode) = extract_sslmode("postgres://host/db");
        assert_eq!(url, "postgres://host/db");
        assert_eq!(mode, SslMode::Prefer);
    }

    #[test]
    fn disable_produces_no_tls_config() {
        assert!(client_config(SslMode::Disable).unwrap().is_none());
    }

    #[test]
    fn every_other_mode_produces_a_tls_config() {
        for mode in [
            SslMode::Allow,
            SslMode::Prefer,
            SslMode::Require,
            SslMode::VerifyCa,
            SslMode::VerifyFull,
        ] {
            assert!(client_config(mode).unwrap().is_some(), "{mode:?}");
        }
    }

    #[test]
    fn driver_mode_only_ever_requires_or_prefers_or_disables() {
        assert_eq!(
            SslMode::Disable.driver_mode(),
            tokio_postgres::config::SslMode::Disable
        );
        assert_eq!(
            SslMode::Allow.driver_mode(),
            tokio_postgres::config::SslMode::Prefer
        );
        assert_eq!(
            SslMode::Prefer.driver_mode(),
            tokio_postgres::config::SslMode::Prefer
        );
        assert_eq!(
            SslMode::Require.driver_mode(),
            tokio_postgres::config::SslMode::Require
        );
        assert_eq!(
            SslMode::VerifyCa.driver_mode(),
            tokio_postgres::config::SslMode::Require
        );
        assert_eq!(
            SslMode::VerifyFull.driver_mode(),
            tokio_postgres::config::SslMode::Require
        );
    }
}
