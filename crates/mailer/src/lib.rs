//! Sends transactional email (verification links, password resets, email
//! change confirmations) through whichever backend is configured —
//! Resend's HTTP API, plain SMTP, or a `Log` fallback that writes the
//! email to `tracing` instead of delivering it, so every email-dependent
//! flow is exercisable with zero external setup.
//!
//! A closed three-variant enum, not a trait object: there's no dynamic
//! plugin story here (unlike `cratebase_server::plugin::Plugin`), just a
//! fixed choice of transport made once at startup from `Config`.

mod config;
mod error;

pub use config::MailerConfig;
pub use error::{MailerError, MailerResult};

use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde_json::json;

enum Backend {
    Resend {
        api_key: String,
        client: reqwest::Client,
    },
    Smtp(AsyncSmtpTransport<Tokio1Executor>),
    Log,
}

#[derive(Clone)]
pub struct Mailer {
    from_address: String,
    from_name: String,
    backend: std::sync::Arc<Backend>,
}

impl Mailer {
    pub fn connect(config: &MailerConfig, from_address: &str, from_name: &str) -> MailerResult<Self> {
        let backend = match config {
            MailerConfig::Resend { api_key } => Backend::Resend {
                api_key: api_key.clone(),
                client: reqwest::Client::new(),
            },
            MailerConfig::Smtp {
                host,
                port,
                username,
                password,
                implicit_tls,
            } => {
                let builder = if *implicit_tls {
                    AsyncSmtpTransport::<Tokio1Executor>::relay(host)?
                } else {
                    AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)?
                };
                let transport = builder
                    .port(*port)
                    .credentials(Credentials::new(username.clone(), password.clone()))
                    .build();
                Backend::Smtp(transport)
            }
            MailerConfig::Log => Backend::Log,
        };
        Ok(Self {
            from_address: from_address.to_string(),
            from_name: from_name.to_string(),
            backend: std::sync::Arc::new(backend),
        })
    }

    /// Sends `html` to `to` — every caller in `cratebase-server` sends a
    /// prerendered HTML string (see `crates/server/src/mail.rs`), never a
    /// template, so this crate has no rendering concerns of its own.
    pub async fn send(&self, to: &str, subject: &str, html: &str) -> MailerResult<()> {
        match self.backend.as_ref() {
            Backend::Log => {
                tracing::info!(
                    to,
                    subject,
                    "mailer: MAIL_DRIVER unset — logging instead of sending; set RESEND_API_KEY or SMTP_* to deliver for real"
                );
                tracing::debug!(html, "mailer: logged email body");
                Ok(())
            }
            Backend::Smtp(transport) => {
                let message = Message::builder()
                    .from(
                        format!("{} <{}>", self.from_name, self.from_address)
                            .parse()
                            .map_err(|_| MailerError::InvalidAddress(self.from_address.clone()))?,
                    )
                    .to(to
                        .parse()
                        .map_err(|_| MailerError::InvalidAddress(to.to_string()))?)
                    .subject(subject)
                    .header(ContentType::TEXT_HTML)
                    .body(html.to_string())?;
                transport.send(message).await?;
                Ok(())
            }
            Backend::Resend { api_key, client } => {
                let res = client
                    .post("https://api.resend.com/emails")
                    .bearer_auth(api_key)
                    .json(&json!({
                        "from": format!("{} <{}>", self.from_name, self.from_address),
                        "to": [to],
                        "subject": subject,
                        "html": html,
                    }))
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn log_backend_never_fails() {
        let mailer = Mailer::connect(&MailerConfig::Log, "no-reply@test.local", "Test").unwrap();
        mailer
            .send("someone@example.com", "Hello", "<p>Hi</p>")
            .await
            .unwrap();
    }
}
