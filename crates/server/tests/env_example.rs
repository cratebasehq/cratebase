//! Guards against `.env.example` documenting an environment variable the
//! server doesn't actually read. `crates/server/src/config.rs`'s
//! `KNOWN_ENV_VARS` is the canonical list; every `KEY=...` assignment in
//! `.env.example` — live or shown commented-out as an example — must name
//! a key from that list, or this test fails the build.
//!
//! This is a one-way check (`.env.example` ⊆ `KNOWN_ENV_VARS`): a few
//! `KNOWN_ENV_VARS` entries (`AUTH_RATE_LIMIT_ENABLED`, `DB_POOL_SIZE`)
//! are accepted ahead of the code that reads them landing on another
//! branch, and `CB_SECRET` (an alias of `AUTH_SECRET`) is deliberately
//! not shown in `.env.example` to avoid encouraging two names for the
//! same setting — so the reverse direction isn't enforced here.

use cratebase_server::config::KNOWN_ENV_VARS;
use std::path::Path;

/// Pulls the `KEY` out of a `.env.example` line, whether it's a live
/// assignment (`KEY=value`) or shown commented-out as an example
/// (`# KEY=value`, with any amount of space after the `#`). Returns
/// `None` for blank lines, section headers, and prose comments that
/// don't look like an assignment at all.
fn env_key(line: &str) -> Option<&str> {
    let line = line.trim();
    let candidate = line.strip_prefix('#').map(str::trim_start).unwrap_or(line);

    let eq = candidate.find('=')?;
    let key = &candidate[..eq];
    if key.is_empty() {
        return None;
    }
    let mut chars = key.chars();
    let first = chars.next()?;
    if !(first.is_ascii_uppercase() || first == '_') {
        return None;
    }
    if !chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
        return None;
    }
    Some(key)
}

#[test]
fn env_example_only_documents_known_vars() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env.example");
    let contents =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path:?}: {e}"));

    let mut unknown = Vec::new();
    for (lineno, line) in contents.lines().enumerate() {
        if let Some(key) = env_key(line) {
            if !KNOWN_ENV_VARS.contains(&key) {
                unknown.push(format!("  line {}: {key}", lineno + 1));
            }
        }
    }

    assert!(
        unknown.is_empty(),
        ".env.example documents env var(s) that crates/server/src/config.rs's \
         KNOWN_ENV_VARS doesn't list (server never reads them, or the list is \
         stale) — add them to KNOWN_ENV_VARS if they're real, otherwise remove \
         them from .env.example:\n{}",
        unknown.join("\n")
    );
}

#[test]
fn known_env_vars_has_no_duplicates() {
    let mut seen = std::collections::HashSet::new();
    for var in KNOWN_ENV_VARS {
        assert!(seen.insert(var), "KNOWN_ENV_VARS lists {var:?} twice");
    }
}

#[test]
fn env_key_parses_assignments_and_ignores_prose() {
    assert_eq!(env_key("PORT=8090"), Some("PORT"));
    assert_eq!(env_key("# PORT=8090"), Some("PORT"));
    assert_eq!(env_key("#   PORT=8090"), Some("PORT"));
    assert_eq!(env_key("AUTH_SECRET="), Some("AUTH_SECRET"));
    assert_eq!(env_key(""), None);
    assert_eq!(env_key("# --- server ---"), None);
    assert_eq!(
        env_key("# Comma-separated list, or `*` for any origin."),
        None
    );
    assert_eq!(
        env_key("#   DATABASE_URL=postgres://a=b"),
        Some("DATABASE_URL")
    );
}
