//! Which SQL backend a [`crate::Db`] talks to, and the three physical
//! column types every record table is built from.

use cratebase_filter::Dialect;

use crate::error::{DbError, DbResult};

/// The two SQL backends Cratebase can run on. Selected at connect time
/// from the `DATABASE_URL` scheme (`sqlite:` / `postgres:`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Backend {
    Sqlite,
    Postgres,
}

impl Backend {
    pub fn from_url(url: &str) -> DbResult<Self> {
        if url.starts_with("sqlite:") {
            Ok(Backend::Sqlite)
        } else if url.starts_with("postgres:") || url.starts_with("postgresql:") {
            Ok(Backend::Postgres)
        } else {
            Err(DbError::InvalidIdentifier(format!(
                "unsupported DATABASE_URL scheme: {url}"
            )))
        }
    }

    pub fn from_dialect(dialect: Dialect) -> Self {
        match dialect {
            Dialect::Sqlite => Backend::Sqlite,
            Dialect::Postgres => Backend::Postgres,
        }
    }

    pub fn dialect(&self) -> Dialect {
        match self {
            Backend::Sqlite => Dialect::Sqlite,
            Backend::Postgres => Dialect::Postgres,
        }
    }

    pub fn is_sqlite(&self) -> bool {
        matches!(self, Backend::Sqlite)
    }

    pub fn is_postgres(&self) -> bool {
        matches!(self, Backend::Postgres)
    }

    /// The column type for everything string-shaped: text, dates, JSON
    /// (as text), single and multi select/relation/file ids. Plain text on
    /// both backends so a field can change shape without a lossy cast and
    /// `cratebase-filter` never needs per-backend JSON path syntax.
    pub fn text_type(&self) -> &'static str {
        "TEXT"
    }

    pub fn number_type(&self) -> &'static str {
        match self {
            Backend::Sqlite => "REAL",
            Backend::Postgres => "DOUBLE PRECISION",
        }
    }

    /// Booleans are stored as `0`/`1` integers on both backends and
    /// converted at the JSON boundary, so the filter compiler emits one
    /// comparison for both and PocketBase-style view queries
    /// (`WHERE verified = 1`) keep working.
    pub fn bool_type(&self) -> &'static str {
        "INTEGER"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_scheme() {
        assert_eq!(
            Backend::from_url("sqlite:data/x.db").unwrap(),
            Backend::Sqlite
        );
        assert_eq!(
            Backend::from_url("sqlite://:memory:").unwrap(),
            Backend::Sqlite
        );
        assert_eq!(
            Backend::from_url("postgres://u:p@h/db").unwrap(),
            Backend::Postgres
        );
        assert_eq!(
            Backend::from_url("postgresql://u:p@h/db").unwrap(),
            Backend::Postgres
        );
        assert!(Backend::from_url("mysql://x").is_err());
    }
}
