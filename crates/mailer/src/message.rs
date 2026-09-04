//! The transport-agnostic email envelope handed to a [`crate::MailBackend`].

/// An `(address, display name)` pair; the name may be empty.
pub type Address = (String, String);

/// One outgoing email. Built by the server (subject/body rendered from a
/// collection's [`cratebase_core::EmailTemplate`]) and handed to whichever
/// backend the [`crate::Mailer`] wraps.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Message {
    /// Recipients as `(address, name)`.
    pub to: Vec<Address>,
    /// Sender as `(address, name)`.
    pub from: Address,
    pub subject: String,
    /// HTML body.
    pub html: String,
    /// Optional plain-text alternative; backends that support it send a
    /// `multipart/alternative` message when present.
    pub text: Option<String>,
    /// Extra headers as `(name, value)`, e.g. `("X-Custom", "value")`.
    pub headers: Vec<(String, String)>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
}

impl Message {
    /// A single-recipient HTML message with no extras.
    pub fn new(
        from: Address,
        to: Address,
        subject: impl Into<String>,
        html: impl Into<String>,
    ) -> Self {
        Message {
            to: vec![to],
            from,
            subject: subject.into(),
            html: html.into(),
            ..Default::default()
        }
    }

    /// Sets the plain-text alternative.
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    /// Appends a header.
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

/// Formats an [`Address`] for a `From:`/`To:` header: `Name <addr>` when
/// a name is present, the bare address otherwise.
pub fn format_address((addr, name): &Address) -> String {
    if name.trim().is_empty() {
        addr.clone()
    } else {
        format!("{name} <{addr}>")
    }
}
