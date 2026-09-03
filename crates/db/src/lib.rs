//! Storage engine for Cratebase: connects to either SQLite or Postgres
//! through sqlx's `Any` driver, keeps collection metadata + their physical
//! tables in sync, and performs record CRUD with API-rule and filter
//! enforcement pushed down into SQL.

pub mod admins;
pub mod backend;
pub mod collections;
pub mod error;
pub mod external_auths;
pub mod otp;
pub mod pool;
pub mod records;
pub mod resolver;
pub mod system;
pub mod validate;
pub mod value;

pub use backend::Backend;
pub use error::{DbError, DbResult};
pub use pool::Db;
pub use resolver::{AuthContext, CollectionResolver, RequestContext, RuleOutcome};
