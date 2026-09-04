//! Delivery backends. Every [`crate::Mailer`] wraps exactly one
//! `Arc<dyn MailBackend>`; the server picks SMTP or Log from settings,
//! Resend from env config, and tests use [`RecordingBackend`].

use std::sync::Mutex;

use async_trait::async_trait;
use lettre::message::header::{ContentType, HeaderName, HeaderValue};
use lettre::message::{Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::transport::smtp::extension::ClientId;
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
use serde_json::json;

use crate::error::{MailerError, MailerResult};
use crate::message::{format_address, Address, Message};

/// A transport that can deliver a [`Message`].
#[async_trait]
pub trait MailBackend: Send + Sync {
    /// Delivers `msg`, returning once the backend has accepted it.
    async fn send(&self, msg: &Message) -> MailerResult<()>;
}

/// Writes every message to the `tracing` log instead of delivering it.
/// The zero-config default, so verification / password-reset flows are
/// exercisable before an operator configures SMTP.
#[derive(Debug, Default, Clone, Copy)]
pub struct LogBackend;

#[async_trait]
impl MailBackend for LogBackend {
    async fn send(&self, msg: &Message) -> MailerResult<()> {
        let to: Vec<String> = msg.to.iter().map(format_address).collect();
        tracing::info!(
            to = ?to,
            from = %format_address(&msg.from),
            subject = %msg.subject,
            "mailer: SMTP disabled — logging instead of sending; enable settings.smtp to deliver for real"
        );
        tracing::debug!(html = %msg.html, text = ?msg.text, "mailer: logged email body");
        Ok(())
    }
}

/// Keeps every sent message in memory for assertions. Also logs like
/// [`LogBackend`] so a test run's output still shows what went out.
#[derive(Debug, Default)]
pub struct RecordingBackend {
    sent: Mutex<Vec<Message>>,
}

impl RecordingBackend {
    pub fn new() -> Self {
        Self::default()
    }

    /// A snapshot of every message sent so far, oldest first.
    pub fn sent(&self) -> Vec<Message> {
        self.sent.lock().map(|v| v.clone()).unwrap_or_default()
    }

    /// The most recently sent message, if any.
    pub fn last(&self) -> Option<Message> {
        self.sent.lock().ok().and_then(|v| v.last().cloned())
    }

    /// Number of messages sent so far.
    pub fn len(&self) -> usize {
        self.sent.lock().map(|v| v.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Forgets everything recorded so far.
    pub fn clear(&self) {
        if let Ok(mut v) = self.sent.lock() {
            v.clear();
        }
    }
}

#[async_trait]
impl MailBackend for RecordingBackend {
    async fn send(&self, msg: &Message) -> MailerResult<()> {
        LogBackend.send(msg).await?;
        if let Ok(mut v) = self.sent.lock() {
            v.push(msg.clone());
        }
        Ok(())
    }
}

/// Plain SMTP through `lettre`.
pub struct SmtpBackend {
    transport: AsyncSmtpTransport<Tokio1Executor>,
}

/// How the SMTP connection is secured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpTls {
    /// Implicit TLS from the first byte (usually port 465). PocketBase's
    /// `smtp.tls: true`.
    Implicit,
    /// Connect in the clear and upgrade with `STARTTLS` if the server
    /// offers it (PocketBase's `smtp.tls: false`, ports 587/25).
    OpportunisticStartTls,
    /// Require a successful `STARTTLS` upgrade.
    RequiredStartTls,
}

/// Everything needed to build an [`SmtpBackend`].
#[derive(Debug, Clone)]
pub struct SmtpOptions {
    pub host: String,
    pub port: u16,
    /// Empty means no authentication.
    pub username: String,
    pub password: String,
    /// `"PLAIN"`, `"LOGIN"` or empty (let the server choose).
    pub auth_method: String,
    pub tls: SmtpTls,
    /// The `EHLO` hostname; empty uses `localhost`.
    pub local_name: String,
}

impl SmtpBackend {
    /// Builds the transport; no connection is opened until the first send.
    /// Must be called from within a tokio runtime (lettre's pool spawns
    /// its housekeeping task here).
    pub fn new(opts: &SmtpOptions) -> MailerResult<Self> {
        if opts.host.trim().is_empty() {
            return Err(MailerError::Config("smtp host is empty".into()));
        }
        let mut builder = match opts.tls {
            SmtpTls::Implicit => AsyncSmtpTransport::<Tokio1Executor>::relay(&opts.host)?,
            SmtpTls::RequiredStartTls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&opts.host)?
            }
            SmtpTls::OpportunisticStartTls => {
                let params = TlsParameters::new(opts.host.clone())?;
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&opts.host)
                    .tls(Tls::Opportunistic(params))
            }
        };
        builder = builder.port(opts.port);
        if !opts.username.is_empty() {
            builder = builder.credentials(Credentials::new(
                opts.username.clone(),
                opts.password.clone(),
            ));
            match opts.auth_method.to_ascii_uppercase().as_str() {
                "PLAIN" => builder = builder.authentication(vec![Mechanism::Plain]),
                "LOGIN" => builder = builder.authentication(vec![Mechanism::Login]),
                _ => {}
            }
        }
        if !opts.local_name.trim().is_empty() {
            builder = builder.hello_name(ClientId::Domain(opts.local_name.clone()));
        }
        Ok(SmtpBackend {
            transport: builder.build(),
        })
    }
}

fn mailbox((addr, name): &Address) -> MailerResult<Mailbox> {
    let email = addr
        .parse()
        .map_err(|_| MailerError::InvalidAddress(addr.clone()))?;
    let name = if name.trim().is_empty() {
        None
    } else {
        Some(name.clone())
    };
    Ok(Mailbox::new(name, email))
}

#[async_trait]
impl MailBackend for SmtpBackend {
    async fn send(&self, msg: &Message) -> MailerResult<()> {
        let mut builder = lettre::Message::builder()
            .from(mailbox(&msg.from)?)
            .subject(&msg.subject);
        for to in &msg.to {
            builder = builder.to(mailbox(to)?);
        }
        for cc in &msg.cc {
            builder = builder.cc(mailbox(cc)?);
        }
        for bcc in &msg.bcc {
            builder = builder.bcc(mailbox(bcc)?);
        }
        for (name, value) in &msg.headers {
            let header_name = HeaderName::new_from_ascii(name.clone())
                .map_err(|_| MailerError::Config(format!("invalid header name '{name}'")))?;
            builder = builder.raw_header(HeaderValue::new(header_name, value.clone()));
        }
        let message = match &msg.text {
            Some(text) => builder.multipart(
                MultiPart::alternative()
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_PLAIN)
                            .body(text.clone()),
                    )
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_HTML)
                            .body(msg.html.clone()),
                    ),
            )?,
            None => builder
                .header(ContentType::TEXT_HTML)
                .body(msg.html.clone())?,
        };
        self.transport.send(message).await?;
        Ok(())
    }
}

/// Resend's HTTPS API (`https://api.resend.com/emails`).
pub struct ResendBackend {
    api_key: String,
    client: reqwest::Client,
}

impl ResendBackend {
    pub fn new(api_key: impl Into<String>) -> Self {
        ResendBackend {
            api_key: api_key.into(),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl MailBackend for ResendBackend {
    async fn send(&self, msg: &Message) -> MailerResult<()> {
        let list =
            |addrs: &[Address]| -> Vec<String> { addrs.iter().map(format_address).collect() };
        let mut body = json!({
            "from": format_address(&msg.from),
            "to": list(&msg.to),
            "subject": msg.subject,
            "html": msg.html,
        });
        if let Some(text) = &msg.text {
            body["text"] = json!(text);
        }
        if !msg.cc.is_empty() {
            body["cc"] = json!(list(&msg.cc));
        }
        if !msg.bcc.is_empty() {
            body["bcc"] = json!(list(&msg.bcc));
        }
        if !msg.headers.is_empty() {
            let headers: serde_json::Map<String, serde_json::Value> = msg
                .headers
                .iter()
                .map(|(k, v)| (k.clone(), json!(v)))
                .collect();
            body["headers"] = serde_json::Value::Object(headers);
        }
        let res = self
            .client
            .post("https://api.resend.com/emails")
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;
        if !res.status().is_success() {
            let status = res.status().as_u16();
            let body = res.text().await.unwrap_or_default();
            return Err(MailerError::Resend { status, body });
        }
        Ok(())
    }
}
