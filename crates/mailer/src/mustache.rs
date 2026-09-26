//! `{{var}}` rendering for the `_emailTemplates` store: dotted paths into
//! a JSON data object, HTML-escaped by default with `{{{raw}}}` for
//! unescaped output. Distinct from [`crate::template::render_template`]'s
//! plain `{PLACEHOLDER}` substitution, which the five per-collection auth
//! templates (and their built-in defaults) keep using unchanged — see
//! that module's doc comment for why the two coexist.

use serde_json::Value;

use crate::template::escape_html;

/// Looks up a dotted path (`"user.name"`) in a JSON object tree. Missing
/// at any segment, or a non-object encountered along the way, yields
/// `None` — which callers render as an empty string, not the literal
/// `{{path}}`.
fn lookup<'a>(data: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = data;
    for segment in path.split('.') {
        current = current.as_object()?.get(segment)?;
    }
    Some(current)
}

/// Stringifies a resolved value for interpolation: strings are used
/// as-is, numbers/bools render their plain form; `null`, arrays and
/// objects have nothing sensible to inline and render as empty.
fn stringify(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

/// Renders every `{{path}}` and `{{{path}}}` in `text` against `data`.
/// `{{{path}}}` is always inserted raw; `{{path}}` is HTML-escaped when
/// `escape` is `true` (the HTML body) and raw when `false` (the subject
/// line and the plain-text alternative, neither of which is an HTML
/// context). A path that resolves to nothing renders as an empty string.
pub fn render_mustache(text: &str, data: &Value, escape: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if text[i..].starts_with("{{{") {
            if let Some(end) = text[i + 3..].find("}}}") {
                let path = text[i + 3..i + 3 + end].trim();
                let value = lookup(data, path).map(stringify).unwrap_or_default();
                out.push_str(&value);
                i += 3 + end + 3;
                continue;
            }
        } else if text[i..].starts_with("{{") {
            if let Some(end) = text[i + 2..].find("}}") {
                let path = text[i + 2..i + 2 + end].trim();
                let value = lookup(data, path).map(stringify).unwrap_or_default();
                if escape {
                    out.push_str(&escape_html(&value));
                } else {
                    out.push_str(&value);
                }
                i += 2 + end + 2;
                continue;
            }
        }
        // No closing delimiter found (or no delimiter at all) — advance
        // by one *char*, not one byte, to stay on UTF-8 boundaries.
        let ch = text[i..].chars().next().expect("i < text.len()");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// A crude but dependency-free HTML-to-plain-text conversion, used to
/// auto-derive a `text` alternative when a template leaves it empty:
/// tags are stripped, block-level closing tags and `<br>` become
/// newlines, a handful of common entities are decoded, and runs of blank
/// lines collapse to one.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut tag = String::new();
    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let t = tag.to_ascii_lowercase();
                if t.starts_with("br")
                    || t.starts_with("/p")
                    || t.starts_with("/div")
                    || t.starts_with("/tr")
                    || t.starts_with("/h1")
                    || t.starts_with("/h2")
                    || t.starts_with("/h3")
                {
                    out.push('\n');
                }
            }
            _ if in_tag => tag.push(c),
            _ => out.push(c),
        }
    }
    let out = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");

    let mut collapsed: Vec<&str> = Vec::new();
    let mut last_blank = false;
    for line in out.lines() {
        let line = line.trim();
        let blank = line.is_empty();
        if blank && last_blank {
            continue;
        }
        collapsed.push(line);
        last_blank = blank;
    }
    collapsed.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_dotted_paths_and_escapes_by_default() {
        let data = json!({ "user": { "name": "<b>Bob & Alice</b>" } });
        assert_eq!(
            render_mustache("Hi {{user.name}}!", &data, true),
            "Hi &lt;b&gt;Bob &amp; Alice&lt;/b&gt;!"
        );
    }

    #[test]
    fn triple_braces_are_always_raw_even_in_escaped_context() {
        let data = json!({ "html": "<b>bold</b>" });
        assert_eq!(render_mustache("{{{html}}}", &data, true), "<b>bold</b>");
    }

    #[test]
    fn escape_false_leaves_html_alone() {
        let data = json!({ "name": "<b>x</b>" });
        assert_eq!(render_mustache("{{name}}", &data, false), "<b>x</b>");
    }

    #[test]
    fn missing_vars_render_as_empty_string() {
        let data = json!({});
        assert_eq!(render_mustache("[{{missing}}]", &data, true), "[]");
        assert_eq!(render_mustache("[{{a.b.c}}]", &data, true), "[]");
    }

    #[test]
    fn non_object_in_the_middle_of_a_path_is_treated_as_missing() {
        let data = json!({ "user": "not an object" });
        assert_eq!(render_mustache("[{{user.name}}]", &data, true), "[]");
    }

    #[test]
    fn numbers_and_bools_stringify() {
        let data = json!({ "n": 3, "b": true });
        assert_eq!(render_mustache("{{n}}-{{b}}", &data, true), "3-true");
    }

    #[test]
    fn html_to_text_strips_tags_and_keeps_line_breaks() {
        let html = "<p>Hello <b>World</b></p><p>Second &amp; line</p>";
        assert_eq!(html_to_text(html), "Hello World\nSecond & line");
    }

    #[test]
    fn html_to_text_collapses_blank_lines() {
        let html = "<p>One</p><br><br><p>Two</p>";
        let text = html_to_text(html);
        assert!(!text.contains("\n\n\n"));
        assert_eq!(text, "One\n\nTwo");
    }
}
