//! Transactional SMS for Cratebase: an [`SmsSender`] wrapping a pluggable
//! [`SmsProvider`] (Twilio's REST API from `settings.sms`, or a
//! `tracing`-log fallback for dev), mirroring [`crate::Mailer`]'s own
//! `MailBackend`/`from_settings` shape exactly — see that module's doc
//! comment for the reasoning this repeats: `enabled` picks between the
//! configured Twilio backend and the zero-config log fallback, the same
//! way `smtp.enabled` picks between `SmtpBackend` and `LogBackend`.

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;

use cratebase_core::settings::Sms;

use crate::error::{MailerError, MailerResult};

/// A transport that can deliver a single SMS message.
#[async_trait]
pub trait SmsProvider: Send + Sync {
    /// Sends `body` to `to` (an E.164 phone number), returning once the
    /// backend has accepted it for delivery.
    async fn send_sms(&self, to: &str, body: &str) -> MailerResult<()>;
}

/// Writes every message to the `tracing` log instead of delivering it.
/// The zero-config default, matching [`crate::LogBackend`]'s role for
/// email: SMS-triggering flows stay exercisable before an operator
/// configures a real provider.
#[derive(Debug, Default, Clone, Copy)]
pub struct LogSmsBackend;

#[async_trait]
impl SmsProvider for LogSmsBackend {
    async fn send_sms(&self, to: &str, body: &str) -> MailerResult<()> {
        tracing::info!(to, body, "sms (log backend, not delivered)");
        Ok(())
    }
}

/// Twilio's Programmable Messaging REST API
/// (`https://api.twilio.com/2010-04-01/Accounts/{AccountSid}/Messages.json`),
/// authenticated with HTTP Basic auth — Account SID as username, Auth
/// Token as password — exactly as Twilio's API docs specify, and posted
/// as `application/x-www-form-urlencoded` (`To`/`From`/`Body`), the only
/// body encoding that endpoint accepts.
pub struct TwilioBackend {
    account_sid: String,
    auth_token: String,
    from_number: String,
    client: reqwest::Client,
}

impl TwilioBackend {
    pub fn new(
        account_sid: impl Into<String>,
        auth_token: impl Into<String>,
        from_number: impl Into<String>,
    ) -> Self {
        TwilioBackend {
            account_sid: account_sid.into(),
            auth_token: auth_token.into(),
            from_number: from_number.into(),
            client: reqwest::Client::new(),
        }
    }

    /// The account-scoped Messages endpoint this backend posts to.
    fn endpoint(&self) -> String {
        format!(
            "https://api.twilio.com/2010-04-01/Accounts/{}/Messages.json",
            self.account_sid
        )
    }

    /// The `Authorization: Basic ...` header value for `account_sid`/
    /// `auth_token`, split out from [`SmsProvider::send_sms`] so the
    /// exact bytes sent over the wire are unit-testable without a mock
    /// HTTP server.
    fn basic_auth_header(&self) -> String {
        let credentials = format!("{}:{}", self.account_sid, self.auth_token);
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(credentials)
        )
    }

    /// The `To`/`From`/`Body` form fields Twilio's API requires, split
    /// out from [`SmsProvider::send_sms`] for the same testability
    /// reason as [`Self::basic_auth_header`].
    fn form_fields<'a>(&'a self, to: &'a str, body: &'a str) -> [(&'static str, &'a str); 3] {
        [
            ("To", to),
            ("From", self.from_number.as_str()),
            ("Body", body),
        ]
    }
}

#[async_trait]
impl SmsProvider for TwilioBackend {
    async fn send_sms(&self, to: &str, body: &str) -> MailerResult<()> {
        let res = self
            .client
            .post(self.endpoint())
            .header("Authorization", self.basic_auth_header())
            .form(&self.form_fields(to, body))
            .send()
            .await?;
        if !res.status().is_success() {
            let status = res.status().as_u16();
            let body = res.text().await.unwrap_or_default();
            return Err(MailerError::Twilio { status, body });
        }
        Ok(())
    }
}

/// Sends SMS messages through one backend. Cheap to clone, mirroring
/// [`crate::Mailer`].
#[derive(Clone)]
pub struct SmsSender {
    backend: Arc<dyn SmsProvider>,
}

impl SmsSender {
    /// Wraps an arbitrary backend.
    pub fn with_backend(backend: Arc<dyn SmsProvider>) -> Self {
        SmsSender { backend }
    }

    /// Builds from the app settings, mirroring [`crate::Mailer::from_settings`]:
    /// Twilio when `sms.enabled`, otherwise the log backend.
    pub fn from_settings(sms: &Sms) -> Self {
        let backend: Arc<dyn SmsProvider> = if sms.enabled {
            Arc::new(TwilioBackend::new(
                sms.account_sid.clone(),
                sms.auth_token.clone(),
                sms.from_number.clone(),
            ))
        } else {
            Arc::new(LogSmsBackend)
        };
        SmsSender::with_backend(backend)
    }

    /// The wrapped backend.
    pub fn backend(&self) -> &Arc<dyn SmsProvider> {
        &self.backend
    }

    /// Delivers `body` to `to` through the configured backend.
    pub async fn send_sms(&self, to: &str, body: &str) -> MailerResult<()> {
        self.backend.send_sms(to, body).await
    }
}

#[cfg(test)]
mod tests {
    use parking_lot::Mutex;

    use super::*;

    /// Records every message sent to it, for assertions — the SMS
    /// analogue of `crate::RecordingBackend`.
    #[derive(Default)]
    struct RecordingSmsBackend {
        sent: Mutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl SmsProvider for RecordingSmsBackend {
        async fn send_sms(&self, to: &str, body: &str) -> MailerResult<()> {
            self.sent.lock().push((to.to_string(), body.to_string()));
            Ok(())
        }
    }

    #[tokio::test]
    async fn log_backend_accepts_messages() {
        let sender = SmsSender::with_backend(Arc::new(LogSmsBackend));
        sender.send_sms("+15551234567", "hello").await.unwrap();
    }

    #[tokio::test]
    async fn from_settings_disabled_uses_log_backend() {
        let sms = Sms::default();
        assert!(!sms.enabled);
        // Not directly introspectable (the backend is type-erased behind
        // `Arc<dyn SmsProvider>`), but this must succeed without ever
        // attempting a network call — only the log backend can do that.
        let sender = SmsSender::from_settings(&sms);
        sender.send_sms("+15551234567", "hello").await.unwrap();
    }

    #[tokio::test]
    async fn recording_backend_captures_sent_messages() {
        let backend = Arc::new(RecordingSmsBackend::default());
        let sender = SmsSender::with_backend(backend.clone());
        sender
            .send_sms("+15551234567", "your code is 123456")
            .await
            .unwrap();
        let sent = backend.sent.lock();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "+15551234567");
        assert_eq!(sent[0].1, "your code is 123456");
    }

    #[test]
    fn twilio_backend_builds_expected_endpoint() {
        let backend = TwilioBackend::new("ACxxxxxxxx", "authtoken", "+15550000000");
        assert_eq!(
            backend.endpoint(),
            "https://api.twilio.com/2010-04-01/Accounts/ACxxxxxxxx/Messages.json"
        );
    }

    #[test]
    fn twilio_backend_basic_auth_header_encodes_sid_and_token() {
        let backend = TwilioBackend::new("ACxxxxxxxx", "s3cr3t", "+15550000000");
        // `base64("ACxxxxxxxx:s3cr3t")` computed independently of the
        // production encoder, so this test would catch a wrong
        // separator or field order, not just round-trip the same call.
        let expected = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("ACxxxxxxxx:s3cr3t")
        );
        assert_eq!(backend.basic_auth_header(), expected);
    }

    #[test]
    fn twilio_backend_form_fields_carry_to_from_body() {
        let backend = TwilioBackend::new("ACxxxxxxxx", "authtoken", "+15550000000");
        let fields = backend.form_fields("+15559998888", "your code is 123456");
        assert_eq!(fields[0], ("To", "+15559998888"));
        assert_eq!(fields[1], ("From", "+15550000000"));
        assert_eq!(fields[2], ("Body", "your code is 123456"));
    }
}
