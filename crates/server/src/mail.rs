//! Embeds the built email templates (`web/email/dist`) into the compiled
//! binary — same trick `dashboard.rs` uses for the admin dashboard — and
//! substitutes `{{token}}` placeholders with real values before handing
//! the HTML to `AppState::mailer`. No template engine: the placeholders
//! are plain text react-email rendered verbatim (see
//! `web/email/scripts/build.tsx`), so a dumb `.replace()` is all that's
//! needed here.

use rust_embed::RustEmbed;

use crate::state::AppState;

#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../web/email/dist"]
struct EmailTemplates;

/// Renders `template` (e.g. `"verification.html"`) with `vars` substituted
/// in, then sends it. Falls back to a minimal plain-text body (still
/// carrying every link/value) if the template isn't built yet
/// (`bun run email:build`) — an email-dependent flow should never hard
/// fail just because the HTML polish hasn't been compiled in.
pub async fn send_template(
    app: &AppState,
    to: &str,
    subject: &str,
    template: &str,
    vars: &[(&str, &str)],
) -> Result<(), cratebase_mailer::MailerError> {
    let html = render(template, vars);
    app.mailer.send(to, subject, &html).await
}

fn render(template: &str, vars: &[(&str, &str)]) -> String {
    let mut html = match EmailTemplates::get(template) {
        Some(file) => String::from_utf8_lossy(&file.data).into_owned(),
        None => fallback_body(vars),
    };
    for (key, value) in vars {
        html = html.replace(&format!("{{{{{key}}}}}"), value);
    }
    html
}

fn fallback_body(vars: &[(&str, &str)]) -> String {
    let mut body = String::from("<p>");
    for (key, value) in vars {
        if *key == "actionUrl" {
            body.push_str(&format!("<a href=\"{value}\">{value}</a>"));
        }
    }
    body.push_str("</p>");
    body
}
