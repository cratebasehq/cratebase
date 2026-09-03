/// Which transport [`crate::Mailer`] sends through. `Log` is the
/// zero-config default: every send is written to the tracing log instead
/// of actually delivered, so email-dependent flows (verification,
/// password reset) are fully exercisable before an operator wires up a
/// real provider.
#[derive(Debug, Clone)]
pub enum MailerConfig {
    Resend {
        api_key: String,
    },
    Smtp {
        host: String,
        port: u16,
        username: String,
        password: String,
        /// Implicit TLS (port 465) vs STARTTLS (587/25). Most providers
        /// (including the common self-hosted ones) expect STARTTLS.
        implicit_tls: bool,
    },
    Log,
}
