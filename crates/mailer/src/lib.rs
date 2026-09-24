//! Transactional email for Cratebase: a [`Mailer`] wrapping a pluggable
//! [`MailBackend`] (SMTP from `settings.smtp`, Resend from env config, a
//! `tracing` log fallback, and an in-memory recorder for tests), the
//! transport-agnostic [`Message`] envelope, and [`render_template`] for
//! the per-collection PocketBase-style email templates.

mod backend;
mod config;
mod dev_mailbox;
mod error;
mod message;
mod template;

pub use backend::{
    LogBackend, MailBackend, RecordingBackend, ResendBackend, SmtpBackend, SmtpOptions, SmtpTls,
};
pub use config::MailerConfig;
pub use dev_mailbox::{CapturedMail, DevMailbox, DEFAULT_CAPACITY};
pub use error::{MailerError, MailerResult};
pub use message::{format_address, Address, Message};
pub use template::{escape_html, render_template, DEFAULT_LAYOUT};

use std::sync::Arc;

use cratebase_core::settings::{Meta, Smtp};

/// Sends [`Message`]s through one backend, filling in the configured
/// sender for callers that don't set `from` themselves. Cheap to clone.
#[derive(Clone)]
pub struct Mailer {
    from: Address,
    backend: Arc<dyn MailBackend>,
    /// `Some` exactly when `backend` is a [`LogBackend`] — see
    /// [`Mailer::dev_mailbox`]. Kept here rather than downcast from
    /// `backend` on demand so callers don't need `MailBackend: Any`.
    dev_mailbox: Option<Arc<DevMailbox>>,
}

impl Mailer {
    /// Wraps an arbitrary backend with no default sender; chain
    /// [`Mailer::with_sender`] to set one. No dev mailbox — use
    /// [`Mailer::from_settings`]/[`Mailer::from_settings_with_inbox`] for
    /// a `Mailer` whose `Log` backend is exposed via
    /// [`Mailer::dev_mailbox`].
    pub fn with_backend(backend: Arc<dyn MailBackend>) -> Self {
        Mailer {
            from: (String::new(), String::new()),
            backend,
            dev_mailbox: None,
        }
    }

    /// Sets the default `from` used by [`Mailer::send_html`] and by
    /// [`Mailer::send`] when the message's own `from` address is empty.
    pub fn with_sender(mut self, address: impl Into<String>, name: impl Into<String>) -> Self {
        self.from = (address.into(), name.into());
        self
    }

    /// Builds from the app settings, PocketBase style: SMTP when
    /// `smtp.enabled`, otherwise the log backend with its own private,
    /// short-lived dev mailbox. `meta.senderAddress` / `meta.senderName`
    /// become the default sender.
    ///
    /// Prefer [`Mailer::from_settings_with_inbox`] wherever the mailer
    /// gets rebuilt more than once (e.g. on every `PATCH /api/settings`)
    /// and the dev inbox should survive that — this constructor always
    /// starts a fresh, empty one.
    ///
    /// Must be called from within a tokio runtime when SMTP is enabled:
    /// lettre's connection pool spawns its housekeeping task on build.
    pub fn from_settings(smtp: &Smtp, meta: &Meta) -> MailerResult<Self> {
        Self::from_settings_with_inbox(smtp, meta, Arc::new(DevMailbox::default()))
    }

    /// As [`Mailer::from_settings`], but the `Log` backend (when chosen)
    /// captures into the given `inbox` instead of a fresh one — pass the
    /// same `Arc` across every rebuild so a dev inbox survives an
    /// unrelated settings save.
    pub fn from_settings_with_inbox(
        smtp: &Smtp,
        meta: &Meta,
        inbox: Arc<DevMailbox>,
    ) -> MailerResult<Self> {
        let (backend, dev_mailbox): (Arc<dyn MailBackend>, Option<Arc<DevMailbox>>) =
            if smtp.enabled {
                (
                    Arc::new(SmtpBackend::new(&SmtpOptions {
                        host: smtp.host.clone(),
                        port: smtp.port,
                        username: smtp.username.clone(),
                        password: smtp.password.clone(),
                        auth_method: smtp.auth_method.clone(),
                        tls: if smtp.tls {
                            SmtpTls::Implicit
                        } else {
                            SmtpTls::OpportunisticStartTls
                        },
                        local_name: smtp.local_name.clone(),
                    })?),
                    None,
                )
            } else {
                (Arc::new(LogBackend::with_mailbox(inbox.clone())), Some(inbox))
            };
        Ok(Mailer {
            from: (meta.sender_address.clone(), meta.sender_name.clone()),
            backend,
            dev_mailbox,
        })
    }

    /// Builds from the env-derived [`MailerConfig`] (Resend / SMTP / Log).
    pub fn from_config(
        config: MailerConfig,
        from_address: &str,
        from_name: &str,
    ) -> MailerResult<Self> {
        let mut dev_mailbox = None;
        let backend: Arc<dyn MailBackend> = match config {
            MailerConfig::Resend { api_key } => Arc::new(ResendBackend::new(api_key)),
            MailerConfig::Smtp {
                host,
                port,
                username,
                password,
                implicit_tls,
            } => Arc::new(SmtpBackend::new(&SmtpOptions {
                host,
                port,
                username,
                password,
                auth_method: String::new(),
                tls: if implicit_tls {
                    SmtpTls::Implicit
                } else {
                    SmtpTls::RequiredStartTls
                },
                local_name: String::new(),
            })?),
            MailerConfig::Log => {
                let inbox = Arc::new(DevMailbox::default());
                dev_mailbox = Some(inbox.clone());
                Arc::new(LogBackend::with_mailbox(inbox))
            }
        };
        Ok(Mailer {
            from: (from_address.to_string(), from_name.to_string()),
            backend,
            dev_mailbox,
        })
    }

    /// Alias of [`Mailer::from_config`] kept for existing callers.
    pub fn connect(
        config: &MailerConfig,
        from_address: &str,
        from_name: &str,
    ) -> MailerResult<Self> {
        Mailer::from_config(config.clone(), from_address, from_name)
    }

    /// A mailer backed by a [`RecordingBackend`], returned alongside it so
    /// tests can inspect what was sent.
    pub fn recording() -> (Self, Arc<RecordingBackend>) {
        let recorder = Arc::new(RecordingBackend::new());
        let mailer = Mailer::with_backend(recorder.clone())
            .with_sender("no-reply@test.local", "Cratebase Test");
        (mailer, recorder)
    }

    /// The configured default sender.
    pub fn sender(&self) -> &Address {
        &self.from
    }

    /// The wrapped backend.
    pub fn backend(&self) -> &Arc<dyn MailBackend> {
        &self.backend
    }

    /// The dev mail inbox this mailer's `Log` backend captures into, or
    /// `None` when a real transport (SMTP/Resend) is configured. The
    /// server's `/api/dev/mails` routes 404 whenever this is `None`, so
    /// real outgoing mail is never exposed there.
    pub fn dev_mailbox(&self) -> Option<Arc<DevMailbox>> {
        self.dev_mailbox.clone()
    }

    /// Delivers `msg`, substituting the default sender when `msg.from`
    /// has an empty address.
    pub async fn send(&self, msg: &Message) -> MailerResult<()> {
        if msg.from.0.is_empty() {
            let mut msg = msg.clone();
            msg.from = self.from.clone();
            return self.backend.send(&msg).await;
        }
        self.backend.send(msg).await
    }

    /// Sends a single-recipient HTML message from the default sender.
    pub async fn send_html(&self, to: &str, subject: &str, html: &str) -> MailerResult<()> {
        let msg = Message::new(
            self.from.clone(),
            (to.to_string(), String::new()),
            subject,
            html,
        );
        self.backend.send(&msg).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn log_backend_never_fails() {
        let mailer = Mailer::from_config(MailerConfig::Log, "no-reply@test.local", "Test").unwrap();
        mailer
            .send_html("someone@example.com", "Hello", "<p>Hi</p>")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn dev_mailbox_is_only_some_for_the_log_backend() {
        let log_mailer = Mailer::from_settings(&Smtp::default(), &Meta::default()).unwrap();
        assert!(log_mailer.dev_mailbox().is_some());

        let smtp = Smtp {
            enabled: true,
            host: "smtp.example.com".into(),
            ..Smtp::default()
        };
        let smtp_mailer = Mailer::from_settings(&smtp, &Meta::default()).unwrap();
        assert!(smtp_mailer.dev_mailbox().is_none());

        let (recording_mailer, _recorder) = Mailer::recording();
        assert!(recording_mailer.dev_mailbox().is_none());
    }

    #[tokio::test]
    async fn from_settings_with_inbox_shares_the_same_mailbox_across_rebuilds() {
        let inbox = Arc::new(DevMailbox::default());
        let first =
            Mailer::from_settings_with_inbox(&Smtp::default(), &Meta::default(), inbox.clone())
                .unwrap();
        first
            .send_html("a@example.com", "First", "<p>1</p>")
            .await
            .unwrap();
        assert_eq!(inbox.len(), 1);

        // Simulates `App::apply_settings` rebuilding the `Mailer` for an
        // unrelated settings change: the same `inbox` is passed again, so
        // the earlier capture is still there.
        let second =
            Mailer::from_settings_with_inbox(&Smtp::default(), &Meta::default(), inbox.clone())
                .unwrap();
        assert_eq!(second.dev_mailbox().unwrap().len(), 1);
        second
            .send_html("b@example.com", "Second", "<p>2</p>")
            .await
            .unwrap();
        assert_eq!(inbox.len(), 2);
    }

    #[tokio::test]
    async fn recording_backend_captures_messages_and_fills_sender() {
        let (mailer, recorder) = Mailer::recording();
        assert!(recorder.is_empty());

        mailer
            .send_html("a@example.com", "First", "<p>1</p>")
            .await
            .unwrap();
        let msg = Message {
            to: vec![("b@example.com".into(), "Bee".into())],
            subject: "Second".into(),
            html: "<p>2</p>".into(),
            text: Some("2".into()),
            headers: vec![("X-Test".into(), "yes".into())],
            ..Default::default()
        };
        mailer.send(&msg).await.unwrap();

        let sent = recorder.sent();
        assert_eq!(sent.len(), 2);
        assert_eq!(
            sent[0].to,
            vec![("a@example.com".to_string(), String::new())]
        );
        assert_eq!(
            sent[0].from,
            (
                "no-reply@test.local".to_string(),
                "Cratebase Test".to_string()
            )
        );
        assert_eq!(sent[1].subject, "Second");
        assert_eq!(
            sent[1].from.0, "no-reply@test.local",
            "empty from is filled in"
        );
        assert_eq!(sent[1].text.as_deref(), Some("2"));
        assert_eq!(recorder.last().unwrap().headers[0].0, "X-Test");

        recorder.clear();
        assert_eq!(recorder.len(), 0);
    }

    #[tokio::test]
    async fn explicit_from_is_preserved() {
        let (mailer, recorder) = Mailer::recording();
        let msg = Message::new(
            ("custom@example.com".into(), "Custom".into()),
            ("x@example.com".into(), String::new()),
            "S",
            "<p/>",
        );
        mailer.send(&msg).await.unwrap();
        assert_eq!(recorder.last().unwrap().from.0, "custom@example.com");
    }

    #[test]
    fn from_settings_picks_log_when_smtp_disabled() {
        let meta = Meta::default();
        let mailer = Mailer::from_settings(&Smtp::default(), &meta).unwrap();
        assert_eq!(mailer.sender().0, meta.sender_address);
        assert_eq!(mailer.sender().1, meta.sender_name);
    }

    #[tokio::test]
    async fn from_settings_builds_smtp_when_enabled() {
        let smtp = Smtp {
            enabled: true,
            host: "smtp.example.com".into(),
            port: 587,
            username: "u".into(),
            password: "p".into(),
            auth_method: "PLAIN".into(),
            tls: false,
            local_name: "mail.example.com".into(),
        };
        assert!(Mailer::from_settings(&smtp, &Meta::default()).is_ok());
        let implicit = Smtp {
            tls: true,
            port: 465,
            ..smtp.clone()
        };
        assert!(Mailer::from_settings(&implicit, &Meta::default()).is_ok());
        let no_host = Smtp {
            host: String::new(),
            ..smtp
        };
        assert!(matches!(
            Mailer::from_settings(&no_host, &Meta::default()),
            Err(MailerError::Config(_))
        ));
    }

    #[tokio::test]
    async fn smtp_backend_rejects_invalid_addresses_before_connecting() {
        let smtp = Smtp {
            enabled: true,
            host: "smtp.invalid".into(),
            ..Smtp::default()
        };
        let mailer = Mailer::from_settings(&smtp, &Meta::default()).unwrap();
        let err = mailer
            .send_html("not an address", "S", "<p/>")
            .await
            .unwrap_err();
        assert!(matches!(err, MailerError::InvalidAddress(_)), "{err}");
    }

    #[test]
    fn format_address_variants() {
        assert_eq!(format_address(&("a@b.c".into(), String::new())), "a@b.c");
        assert_eq!(
            format_address(&("a@b.c".into(), "Name".into())),
            "Name <a@b.c>"
        );
    }
}
