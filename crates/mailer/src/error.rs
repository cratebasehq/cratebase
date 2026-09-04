/// Errors from building a [`crate::Mailer`] or delivering a message.
#[derive(Debug, thiserror::Error)]
pub enum MailerError {
    /// A `from`/`to`/`cc`/`bcc` address didn't parse as an email address.
    #[error("invalid email address: {0}")]
    InvalidAddress(String),
    /// Bad SMTP settings (empty host, unusable TLS parameters, ...).
    #[error("invalid mailer configuration: {0}")]
    Config(String),
    #[error("smtp transport error: {0}")]
    Smtp(#[from] lettre::transport::smtp::Error),
    #[error("failed to build message: {0}")]
    Message(#[from] lettre::error::Error),
    #[error("resend api error ({status}): {body}")]
    Resend { status: u16, body: String },
    #[error("twilio api error ({status}): {body}")]
    Twilio { status: u16, body: String },
    #[error("http request to resend failed: {0}")]
    Http(#[from] reqwest::Error),
}

pub type MailerResult<T> = Result<T, MailerError>;
