//! Storage engine for Cratebase.
//!
//! - [`engine`]: the backend-agnostic [`Engine`] / [`Executor`] /
//!   [`Transaction`] contract and the [`Sql`] value type.
//! - [`sqlite`] / [`postgres`]: the two implementations (rusqlite with a
//!   read pool and a single writer; tokio-postgres behind deadpool).
//! - [`schema`]: DDL derived from a [`cratebase_core::Collection`].
//! - [`collections`]: `_collections` persistence and the arc-swapped
//!   in-memory [`CollectionStore`] every request reads from.
//! - [`params`], [`migrations`], [`logs`]: the remaining system tables.
//! - [`Db`]: the handle bundling the main engine, the logs engine, the
//!   backend and the collection store.
//!
//! Record reads/writes live one layer up, still inside this crate:
//! - [`context`]: the per-request values rules see, and the
//!   [`cratebase_filter::Resolver`] bridge.
//! - [`rules`]: `listRule`/`viewRule`/`createRule`/... evaluation.
//! - [`query`]: the `SELECT` builder (joins, sort, pagination).
//! - [`records`]: the public read/write API the server layer calls.
//! - [`expand`]: relation expansion, one query per collection per level.
//! - [`validate`]: per-field validation with PocketBase's codes.

pub mod backend;
pub mod collections;
pub mod context;
pub mod db;
pub mod engine;
pub mod error;
pub mod expand;
pub mod logs;
pub mod migrations;
pub mod params;
pub mod postgres;
pub mod query;
pub mod records;
pub mod rules;
pub mod schema;
pub mod sqlite;
pub mod system;
pub mod validate;

pub use backend::Backend;
pub use collections::{CollectionStore, Snapshot};
pub use context::{AuthContext, CollectionResolver, RequestContext};
pub use cratebase_filter::Dialect;
pub use db::Db;
pub use engine::{quote_ident, Engine, Executor, Row, Sql, Transaction, TransactionImpl};
pub use error::{DbError, DbResult};
pub use postgres::PostgresEngine;
pub use records::{FileRef, ListParams, ListResult};
pub use rules::RuleOutcome;
pub use sqlite::SqliteEngine;
pub use validate::UploadMeta;
