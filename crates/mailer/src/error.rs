#[derive(Debug, thiserror::Error)]
pub enum MailerError {
    #[error("invalid email address: {0}")]
    InvalidAddress(String),
    #[error("smtp transport error: {0}")]
    Smtp(#[from] lettre::transport::smtp::Error),
    #[error("failed to build message: {0}")]
    Message(#[from] lettre::error::Error),
    #[error("resend api error ({status}): {body}")]
    Resend { status: u16, body: String },
    #[error("http request to resend failed: {0}")]
    Http(#[from] reqwest::Error),
}

pub type MailerResult<T> = Result<T, MailerError>;
