use cratebase_core::field::is_valid_identifier;

use crate::error::{DbError, DbResult};

/// The two SQL backends Cratebase can run on. Selected at connect time from
/// the `DATABASE_URL` scheme (`sqlite:` / `postgres:`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Sqlite,
    Postgres,
}

impl Backend {
    pub fn from_url(url: &str) -> DbResult<Self> {
        if url.starts_with("sqlite:") || url.starts_with("sqlite::") {
            Ok(Backend::Sqlite)
        } else if url.starts_with("postgres:") || url.starts_with("postgresql:") {
            Ok(Backend::Postgres)
        } else {
            Err(DbError::InvalidIdentifier(format!(
                "unsupported DATABASE_URL scheme: {url}"
            )))
        }
    }

    pub fn dialect(&self) -> cratebase_filter::Dialect {
        match self {
            Backend::Sqlite => cratebase_filter::Dialect::Sqlite,
            Backend::Postgres => cratebase_filter::Dialect::Postgres,
        }
    }

    /// The SQL column type used to store TEXT-ish data (strings, dates,
    /// json-as-text, single/multi select & relation ids). Both backends use
    /// plain text so records never require lossy type coercion when a field
    /// changes shape, and `cratebase-filter` never needs per-backend JSON
    /// path syntax.
    pub fn text_type(&self) -> &'static str {
        "TEXT"
    }

    pub fn number_type(&self) -> &'static str {
        match self {
            Backend::Sqlite => "REAL",
            Backend::Postgres => "DOUBLE PRECISION",
        }
    }

    /// sqlx's `Any` driver cannot decode SQLite's native `BOOLEAN` type
    /// (it only bridges NULL/INTEGER/REAL/TEXT/BLOB), so booleans are
    /// stored as `0`/`1` integers on both backends and converted at the
    /// JSON boundary instead of relying on a native bool column type.
    pub fn bool_type(&self) -> &'static str {
        "INTEGER"
    }

    /// Quote a validated identifier (table/column name) for safe
    /// interpolation into DDL/DML. Callers MUST validate with
    /// [`is_valid_identifier`] first; this only applies dialect quoting.
    pub fn quote_ident(&self, ident: &str) -> DbResult<String> {
        if !is_valid_identifier(ident) {
            return Err(DbError::InvalidIdentifier(ident.to_string()));
        }
        Ok(format!("\"{ident}\""))
    }

    /// `INSERT ... ON CONFLICT DO NOTHING` fragment, identical on both
    /// backends since sqlite adopted the postgres upsert syntax.
    pub fn autoincrement_pk(&self) -> &'static str {
        "TEXT PRIMARY KEY"
    }
}
