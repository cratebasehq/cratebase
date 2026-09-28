//! Shared full-text-search helpers used by both `cratebase-filter`
//! (compiling `search("query")`/`?search=` at query time) and
//! `cratebase-db` (building the Postgres generated `tsvector` column at
//! schema-sync time), so a collection's `searchLanguage` normalizes
//! identically wherever it's read — the query-time config always matches
//! the one the indexed column was actually built with.

/// Postgres text-search configs recognized without a live connection —
/// every config a stock `initdb` installs (the "snowball" stemmer set
/// plus `simple`). Anything else normalizes to `"simple"` (always
/// available, no extra extension) rather than producing DDL or a query
/// that references a config that might not exist on a given server, or
/// trusting an arbitrary operator-supplied string into
/// `to_tsvector('{lang}', ...)`/`websearch_to_tsquery('{lang}', ...)`
/// SQL text.
pub const KNOWN_TS_CONFIGS: &[&str] = &[
    "simple", "arabic", "armenian", "basque", "catalan", "danish", "dutch", "english", "finnish",
    "french", "german", "greek", "hindi", "hungarian", "indonesian", "irish", "italian",
    "lithuanian", "nepali", "norwegian", "portuguese", "romanian", "russian", "serbian",
    "spanish", "swedish", "tamil", "turkish", "yiddish",
];

/// Normalize a `searchLanguage` value against [`KNOWN_TS_CONFIGS`],
/// falling back to `"simple"`.
pub fn known_ts_config(lang: &str) -> &'static str {
    KNOWN_TS_CONFIGS
        .iter()
        .find(|c| **c == lang)
        .copied()
        .unwrap_or("simple")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_configs_pass_through_and_others_fall_back_to_simple() {
        assert_eq!(known_ts_config("english"), "english");
        assert_eq!(known_ts_config("indonesian"), "indonesian");
        assert_eq!(known_ts_config("simple"), "simple");
        assert_eq!(known_ts_config("bogus"), "simple");
        assert_eq!(known_ts_config(""), "simple");
        assert_eq!(known_ts_config("english; DROP TABLE x"), "simple");
    }
}
