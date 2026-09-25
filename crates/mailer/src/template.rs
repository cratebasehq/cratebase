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

/// [`DEFAULT_LAYOUT`], but with the `.btn` accent color swapped for
/// `meta.brand_color` (when set) and an optional logo (`meta.logo_url`)
/// inserted above `content_html`. Used for `_emailTemplates` rows with
/// `layout: true`; the five legacy per-collection auth templates keep
/// using the unbranded [`DEFAULT_LAYOUT`] via [`render_template`].
pub fn render_layout(content_html: &str, meta: &Meta) -> String {
    let accent = meta.brand_color.trim();
    let layout = if accent.is_empty() {
        DEFAULT_LAYOUT.to_string()
    } else {
        // This exact string appears exactly once in `DEFAULT_LAYOUT`, in
        // the `.btn` rule — the body's own `color: #16161a;` is left
        // alone so ordinary text doesn't turn into the brand color.
        DEFAULT_LAYOUT.replace(
            "background: #16161a !important;",
            &format!("background: {accent} !important;"),
        )
    };
    let logo_url = meta.logo_url.trim();
    let logo = if logo_url.is_empty() {
        String::new()
    } else {
        format!(
            "<p style=\"text-align:center;margin:0 0 20px;\"><img src=\"{}\" alt=\"{}\" style=\"max-height:40px;border:0;\"></p>\n",
            escape_html(logo_url),
            escape_html(&meta.app_name),
        )
    };
    layout.replacen("{CONTENT}", &format!("{logo}{content_html}"), 1)
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
/// `{{appUrl}}` always available (from `meta`, overriding any same-named
/// key in `data` — they are built-ins, not caller data). Returns
/// `(subject, html, text)`.
pub fn render_email_template(doc: &TemplateDoc, data: &Value, meta: &Meta) -> (String, String, String) {
    let mut map = data.as_object().cloned().unwrap_or_default();
    map.insert("appName".into(), Value::String(meta.app_name.clone()));
    map.insert("appUrl".into(), Value::String(meta.app_url.clone()));
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
    fn render_layout_swaps_brand_color_and_leaves_body_text_color_alone() {
        let meta = Meta {
            brand_color: "#ff0000".into(),
            ..Meta::default()
        };
        let html = render_layout("<p>hi</p>", &meta);
        assert!(html.contains("background: #ff0000 !important;"));
        assert!(html.contains("color: #16161a;"), "body text color untouched");
        assert!(html.contains("<p>hi</p>"));
    }

    #[test]
    fn render_layout_with_no_brand_color_is_unchanged() {
        let html = render_layout("<p>hi</p>", &Meta::default());
        assert!(html.contains("background: #16161a !important;"));
    }

    #[test]
    fn render_layout_inserts_logo_before_content() {
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
