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
//! Record reads/writes/validation live one layer up (`records`,
//! `resolver`, `validate`) and are built on the executors exposed here.

pub mod backend;
pub mod collections;
pub mod db;
pub mod engine;
pub mod error;
pub mod logs;
pub mod migrations;
pub mod params;
pub mod postgres;
pub mod schema;
pub mod sqlite;
pub mod system;

pub use backend::Backend;
pub use collections::{CollectionStore, Snapshot};
pub use cratebase_filter::Dialect;
pub use db::Db;
pub use engine::{quote_ident, Engine, Executor, Row, Sql, Transaction, TransactionImpl};
pub use error::{DbError, DbResult};
pub use postgres::PostgresEngine;
pub use sqlite::SqliteEngine;
