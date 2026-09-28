//! Rendering of collection-level email templates the way PocketBase does
//! it: plain `{PLACEHOLDER}` substitution into the template's `subject`
//! and `body`, with the body then wrapped in a fixed HTML layout.
//!
//! Supported placeholders are whatever the caller passes in `vars`;
//! PocketBase's are `{APP_NAME}`, `{APP_URL}`, `{TOKEN}`, `{OTP}`,
//! `{ALERT_INFO}` and `{RECORD:field}` (pass the key as `RECORD:name`).

use cratebase_core::settings::Meta;
use cratebase_core::EmailTemplate;
use serde_json::Value;

use crate::mustache::{html_to_text, render_mustache};

/// The outer HTML shell every rendered body is placed into, modelled on
/// PocketBase's `mails/layout.html`: a centered, single-column, responsive
/// card with a `.btn` style for the call-to-action link the default
/// templates use. `{CONTENT}` is replaced with the rendered body.
pub const DEFAULT_LAYOUT: &str = r#"<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta http-equiv="X-UA-Compatible" content="IE=edge">
  <meta name="x-apple-disable-message-reformatting">
  <style>
    body, html {
      padding: 0;
      margin: 0;
      border: 0;
      color: #16161a;
      background: #fff;
      font-size: 14px;
      line-height: 20px;
      font-weight: normal;
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
    }
    body {
      padding: 20px 30px;
    }
    strong {
      font-weight: bold;
    }
    em {
      font-style: italic;
    }
    p {
      display: block;
      margin: 10px 0;
      font-family: inherit;
    }
    small {
      font-size: 12px;
      line-height: 16px;
    }
    hr {
      display: block;
      height: 1px;
      border: 0;
      width: 100%;
      background: #e1e6ea;
      margin: 10px 0;
    }
    a {
      color: inherit;
    }
    .hidden {
      display: none !important;
    }
    .btn {
      display: inline-block;
      vertical-align: top;
      border: 0;
      cursor: pointer;
      color: #fff !important;
      background: #16161a !important;
      text-decoration: none !important;
      line-height: 40px;
      width: auto;
      min-width: 150px;
      text-align: center;
      padding: 0 20px;
      margin: 5px 0;
      font-family: inherit;
      font-size: 14px;
      font-weight: bold;
      border-radius: 6px;
      box-sizing: border-box;
    }
    .wrapper {
      max-width: 560px;
      margin: 0 auto;
    }
  </style>
</head>
<body>
  <div class="wrapper">
{CONTENT}
  </div>
</body>
</html>
"#;

/// Escapes `&`, `<`, `>`, `"` and `'` for safe interpolation into HTML.
pub fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Replaces every `{KEY}` in `text` with the matching value from `vars`,
/// passing each value through `map` first.
fn substitute(text: &str, vars: &[(&str, &str)], map: impl Fn(&str) -> String) -> String {
    let mut out = text.to_string();
    for (key, value) in vars {
        let placeholder = format!("{{{key}}}");
        if out.contains(&placeholder) {
            out = out.replace(&placeholder, &map(value));
        }
    }
    out
}

/// Substitutes `vars` into `template.body` (HTML-escaping every value)
/// and wraps the result in [`DEFAULT_LAYOUT`]. The subject gets the same
/// substitution with the raw values, since it is a plain-text header.
///
/// Returns `(subject, html)`.
pub fn render_template(template: &EmailTemplate, vars: &[(&str, &str)]) -> (String, String) {
    let subject = substitute(&template.subject, vars, str::to_string);
    let body = substitute(&template.body, vars, escape_html);
    let html = DEFAULT_LAYOUT.replacen("{CONTENT}", &body, 1);
    (subject, html)
}

/// Fallback CTA/accent color when `meta.brand_color` is blank — a near-
/// black neutral that reads fine on both a white and a dark card.
pub const DEFAULT_BRAND_COLOR: &str = "#171717";

/// Soft page background the branded card sits on (light mode).
const CARD_PAGE_BG: &str = "#f4f4f5";

/// The world-class, table-based outer shell used for `_emailTemplates`
/// rows with `layout: true` (via [`render_email_template`]): a
/// centered ~600px card on a soft page background, a logo or text
/// wordmark header, a muted footer, mobile padding, and a
/// `prefers-color-scheme: dark` stylesheet with `!important` overrides
/// on a handful of `cb-*` classes so per-template content (headings,
/// body copy, the CTA button's plain-text fallback, boxed info/OTP
/// blocks — see [`render_email_template`]'s callers in
/// `crates/db/src/migrations.rs`) stays legible in both themes without
/// relying on JS or external stylesheets. `{HEADER}`, `{CONTENT}` and
/// `{FOOTER_APP}` are the only substitution points — every other brace
/// in this string is literal CSS/VML and is never touched by
/// `str::replacen`. The five legacy per-collection auth templates keep
/// using the unbranded [`DEFAULT_LAYOUT`] via [`render_template`] — see
/// that function's own doc comment for why the two shells coexist.
const BRANDED_LAYOUT: &str = r#"<!DOCTYPE html>
<html lang="en" xmlns="http://www.w3.org/1999/xhtml" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta http-equiv="X-UA-Compatible" content="IE=edge">
<meta name="x-apple-disable-message-reformatting">
<meta name="color-scheme" content="light dark">
<meta name="supported-color-schemes" content="light dark">
<!--[if mso]>
<noscript><xml><o:OfficeDocumentSettings><o:PixelsPerInch>96</o:PixelsPerInch></o:OfficeDocumentSettings></xml></noscript>
<![endif]-->
<style>
  body, table, td, a { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; }
  body { -webkit-text-size-adjust: 100%; -ms-text-size-adjust: 100%; }
  a { text-decoration: none; }
  @media only screen and (max-width: 620px) {
    .cb-container { width: 100% !important; }
    .cb-px { padding-left: 20px !important; padding-right: 20px !important; }
    .cb-card { border-radius: 0 !important; }
  }
  @media (prefers-color-scheme: dark) {
    .cb-bg { background: #0b0b0c !important; }
    .cb-card { background: #17171a !important; border-color: #2a2a2e !important; }
    .cb-text { color: #e4e4e7 !important; }
    .cb-heading { color: #fafafa !important; }
    .cb-muted { color: #9a9aa1 !important; }
    .cb-box { background: #232326 !important; }
    .cb-box-text { color: #d4d4d8 !important; }
    .cb-otp-code { color: #f4f4f5 !important; }
    .cb-fallback-link { color: #c4c4c9 !important; }
  }
</style>
</head>
<body class="cb-bg" style="margin:0;padding:0;background:{PAGE_BG};">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" class="cb-bg" style="background:{PAGE_BG};">
  <tr>
    <td align="center" style="padding:40px 16px;">
      <table role="presentation" width="600" cellpadding="0" cellspacing="0" border="0" class="cb-container" style="width:600px;max-width:600px;">
        <tr>
          <td align="center" class="cb-px" style="padding:0 8px 24px;">{HEADER}</td>
        </tr>
        <tr>
          <td class="cb-card cb-px" style="background:#ffffff;border:1px solid #e5e5e7;border-radius:12px;padding:40px;">
            <div class="cb-text" style="color:#171717;font-size:15px;line-height:24px;">
{CONTENT}
            </div>
          </td>
        </tr>
        <tr>
          <td align="center" class="cb-px" style="padding:24px 8px 0;">
            <p class="cb-muted" style="margin:0;color:#8a8a90;font-size:12px;line-height:18px;">{FOOTER_APP} &middot; you're receiving this because you have an account with {FOOTER_APP}.</p>
          </td>
        </tr>
      </table>
    </td>
  </tr>
</table>
</body>
</html>
"#;

/// [`BRANDED_LAYOUT`] with `meta`'s logo (or a text wordmark fallback
/// when no `logo_url` is set) inserted as the header, and the footer's
/// app name filled in. Used for `_emailTemplates` rows with
/// `layout: true`; the five legacy per-collection auth templates keep
/// using the unbranded [`DEFAULT_LAYOUT`] via [`render_template`].
///
/// The CTA button's accent color is *not* handled here — it's a
/// `{{brandColor}}` mustache built-in (see [`render_email_template`])
/// that per-template content interpolates itself, since not every
/// template has a button and the layout has no way to know.
pub fn render_layout(content_html: &str, meta: &Meta) -> String {
    let app_name = meta.app_name.trim();
    let logo_url = meta.logo_url.trim();
    let header = if !logo_url.is_empty() {
        format!(
            r#"<img src="{}" alt="{}" height="32" style="display:block;height:32px;width:auto;border:0;outline:none;text-decoration:none;margin:0 auto;">"#,
            escape_html(logo_url),
            escape_html(app_name),
        )
    } else if !app_name.is_empty() {
        format!(
            r#"<span class="cb-heading" style="font-size:18px;font-weight:700;color:#111827;letter-spacing:-0.01em;">{}</span>"#,
            escape_html(app_name),
        )
    } else {
        String::new()
    };
    let footer_app = escape_html(if app_name.is_empty() {
        "this app"
    } else {
        app_name
    });
    BRANDED_LAYOUT
        .replace("{PAGE_BG}", CARD_PAGE_BG)
        .replacen("{HEADER}", &header, 1)
        .replacen("{CONTENT}", content_html, 1)
        .replace("{FOOTER_APP}", &footer_app)
}

/// One `_emailTemplates` row's renderable content: `{{var}}`-style
/// (dotted paths, HTML-escaped by default, `{{{raw}}}` for unescaped),
/// distinct from the legacy `{PLACEHOLDER}` syntax [`render_template`]
/// uses for the five per-collection auth templates.
pub struct TemplateDoc<'a> {
    pub subject: &'a str,
    pub html: &'a str,
    /// Plain-text alternative; when empty, derived from `html` via
    /// [`html_to_text`] after rendering.
    pub text: &'a str,
    /// Wrap `html` in [`render_layout`] (with `meta`'s logo/brand color)
    /// before returning it.
    pub layout: bool,
}

/// Renders a [`TemplateDoc`] against `data`, with `{{appName}}`/
/// `{{appUrl}}`/`{{brandColor}}` always available (from `meta`,
/// overriding any same-named key in `data` — they are built-ins, not
/// caller data). `{{brandColor}}` is `meta.brand_color` when set, else
/// [`DEFAULT_BRAND_COLOR`] — per-template content interpolates it
/// directly (a CTA button, an OTP box) since [`render_layout`] itself
/// has no per-send knowledge of which content needs an accent color.
/// Returns `(subject, html, text)`.
pub fn render_email_template(
    doc: &TemplateDoc,
    data: &Value,
    meta: &Meta,
) -> (String, String, String) {
    let mut map = data.as_object().cloned().unwrap_or_default();
    map.insert("appName".into(), Value::String(meta.app_name.clone()));
    map.insert("appUrl".into(), Value::String(meta.app_url.clone()));
    let brand_color = meta.brand_color.trim();
    let brand_color = if brand_color.is_empty() {
        DEFAULT_BRAND_COLOR
    } else {
        brand_color
    };
    map.insert(
        "brandColor".into(),
        Value::String(brand_color.to_string()),
    );
    let data = Value::Object(map);

    // The subject line and the plain-text alternative are not HTML
    // contexts, so their `{{var}}`s are never escaped.
    let subject = render_mustache(doc.subject, &data, false);
    let body_html = render_mustache(doc.html, &data, true);
    let html = if doc.layout {
        render_layout(&body_html, meta)
    } else {
        body_html.clone()
    };
    let text = if doc.text.trim().is_empty() {
        html_to_text(&body_html)
    } else {
        render_mustache(doc.text, &data, false)
    };
    (subject, html, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_all_vars_and_wraps_in_layout() {
        let template = EmailTemplate {
            subject: "Verify your {APP_NAME} email".into(),
            body: "<p>Hi {RECORD:name},</p><a class=\"btn\" href=\"{APP_URL}/_/#/auth/confirm-verification/{TOKEN}\">Verify</a><p>{OTP} {ALERT_INFO}</p>".into(),
        };
        let (subject, html) = render_template(
            &template,
            &[
                ("APP_NAME", "Acme"),
                ("APP_URL", "http://localhost:8090"),
                ("TOKEN", "eyJ.abc.def"),
                ("OTP", "12345678"),
                ("ALERT_INFO", "Mozilla, 127.0.0.1"),
                ("RECORD:name", "Nico"),
            ],
        );
        assert_eq!(subject, "Verify your Acme email");
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains(".btn {"));
        assert!(html.contains("<p>Hi Nico,</p>"));
        assert!(html
            .contains("href=\"http://localhost:8090/_/#/auth/confirm-verification/eyJ.abc.def\""));
        assert!(html.contains("<p>12345678 Mozilla, 127.0.0.1</p>"));
        assert!(!html.contains("{CONTENT}"));
        assert!(!html.contains("{APP_NAME}"));
    }

    #[test]
    fn escapes_html_in_body_but_not_subject() {
        let template = EmailTemplate {
            subject: "Hello {APP_NAME}".into(),
            body: "<p>{APP_NAME}</p>".into(),
        };
        let (subject, html) =
            render_template(&template, &[("APP_NAME", "<b>A&B</b> \"quoted\" 'x'")]);
        assert_eq!(subject, "Hello <b>A&B</b> \"quoted\" 'x'");
        assert!(html.contains("<p>&lt;b&gt;A&amp;B&lt;/b&gt; &quot;quoted&quot; &#39;x&#39;</p>"));
        assert!(!html.contains("<b>A&B</b>"));
    }

    #[test]
    fn unknown_placeholders_are_left_alone() {
        let template = EmailTemplate {
            subject: "{UNKNOWN}".into(),
            body: "{ALSO_UNKNOWN}".into(),
        };
        let (subject, html) = render_template(&template, &[("APP_NAME", "x")]);
        assert_eq!(subject, "{UNKNOWN}");
        assert!(html.contains("{ALSO_UNKNOWN}"));
    }

    #[test]
    fn render_layout_is_a_centered_card_on_a_soft_background() {
        let html = render_layout("<p>hi</p>", &Meta::default());
        assert!(html.contains("width=\"600\""), "600px-wide card table");
        assert!(html.contains("<p>hi</p>"));
        assert!(html.contains(CARD_PAGE_BG), "soft page background");
        assert!(html.contains("border-radius:12px"), "rounded card");
        assert!(!html.contains("{CONTENT}"));
        assert!(!html.contains("{HEADER}"));
        assert!(!html.contains("{FOOTER_APP}"));
        assert!(!html.contains("{PAGE_BG}"));
    }

    #[test]
    fn render_layout_is_dark_mode_friendly_with_safe_fallbacks() {
        let html = render_layout("<p>hi</p>", &Meta::default());
        assert!(html.contains(r#"<meta name="color-scheme" content="light dark">"#));
        assert!(html.contains("prefers-color-scheme: dark"));
        // Every dark override is `!important`, so a client without media
        // query support (classic Outlook) just gets the light styles —
        // a safe fallback, not a broken one.
        assert!(html.contains(".cb-card { background: #17171a !important;"));
        assert!(html.contains(".cb-text { color: #e4e4e7 !important; }"));
    }

    #[test]
    fn render_layout_is_responsive_on_mobile() {
        let html = render_layout("<p>hi</p>", &Meta::default());
        assert!(html.contains("@media only screen and (max-width: 620px)"));
        assert!(html.contains(".cb-px { padding-left: 20px !important;"));
        assert!(html.contains(r#"<meta name="viewport" content="width=device-width, initial-scale=1">"#));
    }

    #[test]
    fn render_layout_inserts_logo_before_content_when_set() {
        let meta = Meta {
            logo_url: "https://example.com/logo.png".into(),
            app_name: "Acme".into(),
            ..Meta::default()
        };
        let html = render_layout("<p>hi</p>", &meta);
        let logo_pos = html.find("logo.png").unwrap();
        let content_pos = html.find("<p>hi</p>").unwrap();
        assert!(logo_pos < content_pos);
        assert!(html.contains("alt=\"Acme\""));
    }

    #[test]
    fn render_layout_falls_back_to_a_text_wordmark_without_a_logo() {
        let meta = Meta {
            app_name: "Acme".into(),
            ..Meta::default()
        };
        let html = render_layout("<p>hi</p>", &meta);
        assert!(!html.contains("<img"), "no logo configured, no <img>");
        assert!(html.contains(">Acme<"), "app name shown as a text wordmark");
    }

    #[test]
    fn render_layout_footer_mentions_the_app_name() {
        let meta = Meta {
            app_name: "Acme".into(),
            ..Meta::default()
        };
        let html = render_layout("<p>hi</p>", &meta);
        assert_eq!(html.matches("Acme").count(), 3, "wordmark + footer x2");
    }

    #[test]
    fn render_layout_footer_falls_back_without_an_app_name() {
        let meta = Meta {
            app_name: String::new(),
            ..Meta::default()
        };
        let html = render_layout("<p>hi</p>", &meta);
        assert!(html.contains("this app"));
    }

    #[test]
    fn render_email_template_uses_mustache_and_builtins() {
        let doc = TemplateDoc {
            subject: "Welcome {{user.name}}",
            html: "<p>Hi {{user.name}}, visit {{appUrl}}</p>",
            text: "",
            layout: true,
        };
        let meta = Meta {
            app_name: "Acme".into(),
            app_url: "https://acme.test".into(),
            ..Meta::default()
        };
        let data = serde_json::json!({ "user": { "name": "<Bob>" } });
        let (subject, html, text) = render_email_template(&doc, &data, &meta);
        assert_eq!(subject, "Welcome <Bob>", "subject is not HTML-escaped");
        assert!(html.contains("Hi &lt;Bob&gt;, visit https://acme.test"));
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert_eq!(text, "Hi <Bob>, visit https://acme.test");
    }

    #[test]
    fn render_email_template_exposes_brand_color_builtin() {
        let doc = TemplateDoc {
            subject: "S",
            html: r#"<a style="background:{{brandColor}}">Go</a>"#,
            text: "",
            layout: false,
        };
        let with_brand = render_email_template(
            &doc,
            &serde_json::json!({}),
            &Meta {
                brand_color: "#ff9900".into(),
                ..Meta::default()
            },
        );
        assert!(with_brand.1.contains("background:#ff9900"));

        let (_, without_brand, _) =
            render_email_template(&doc, &serde_json::json!({}), &Meta::default());
        assert!(
            without_brand.contains(DEFAULT_BRAND_COLOR),
            "falls back to the default accent when meta.brand_color is blank"
        );
    }

    #[test]
    fn render_email_template_without_layout_skips_the_shell() {
        let doc = TemplateDoc {
            subject: "S",
            html: "<p>body only</p>",
            text: "",
            layout: false,
        };
        let (_, html, _) = render_email_template(&doc, &serde_json::json!({}), &Meta::default());
        assert_eq!(html, "<p>body only</p>");
    }

    #[test]
    fn render_email_template_prefers_explicit_text_over_derived() {
        let doc = TemplateDoc {
            subject: "S",
            html: "<p>Hi {{name}}</p>",
            text: "Plain hi {{name}}",
            layout: false,
        };
        let data = serde_json::json!({ "name": "Bob" });
        let (_, _, text) = render_email_template(&doc, &data, &Meta::default());
        assert_eq!(text, "Plain hi Bob");
    }

    #[test]
    fn default_pocketbase_templates_render() {
        let auth = cratebase_core::collection::AuthOptions::default();
        let (subject, html) = render_template(
            &auth.verification_template,
            &[
                ("APP_NAME", "Acme"),
                ("APP_URL", "http://localhost:8090"),
                ("TOKEN", "tok"),
            ],
        );
        assert_eq!(subject, "Verify your Acme email");
        assert!(html.contains("http://localhost:8090/_/#/auth/confirm-verification/tok"));
        assert!(html.contains("Acme team"));
    }
}
