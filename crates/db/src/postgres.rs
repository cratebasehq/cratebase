//! Postgres engine on top of `tokio-postgres` + `deadpool-postgres`.
//!
//! One pool; `begin` checks a connection out for the life of the
//! transaction. [`Sql`] maps to `TEXT` / `BIGINT` / `DOUBLE PRECISION` /
//! `BYTEA`; because Postgres infers parameter types from the statement,
//! a bound value is encoded according to the type the server asked for
//! (an `Sql::Int` bound to a `DOUBLE PRECISION` column is sent as a
//! float, an `Sql::Text` bound to a `JSONB` column as JSON, and so on),
//! and `Sql::Null` is a typed NULL for any column.
//!
//! Decoding goes by column type, collapsing Postgres' richer type system
//! onto the five [`Sql`] shapes: booleans become `0`/`1`, JSON becomes
//! its text, timestamps become PocketBase-formatted strings.

use std::error::Error as StdError;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::BytesMut;
use chrono::{NaiveDate, NaiveDateTime, Utc};
use cratebase_core::DateTime;
use cratebase_filter::Dialect;
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use tokio_postgres::tls::{MakeTlsConnect, TlsConnect};
use tokio_postgres::types::{to_sql_checked, FromSql, IsNull, ToSql, Type};
use tokio_postgres::{AsyncMessage, NoTls, Socket};
use tokio_postgres_rustls::MakeRustlsConnect;

use crate::engine::{Engine, Executor, Row, Sql, Transaction, TransactionImpl};
use crate::error::{DbError, DbResult};
use crate::pg_tools::{self, PgConnParams};
use crate::postgres_tls;

/// Default pool size when the caller has no better idea.
pub const DEFAULT_POOL_SIZE: usize = 10;

#[derive(Clone)]
pub struct PostgresEngine {
    pool: Pool,
    /// Kept alongside the pool so [`Engine::subscribe_realtime`] can open
    /// its own dedicated, unpooled `LISTEN` connections: a pooled
    /// connection can be recycled out from under a long-lived listener,
    /// and reconnecting on loss (below) needs a config to reconnect
    /// *with*.
    config: tokio_postgres::Config,
    /// The TLS config derived from `sslmode` (see `crate::postgres_tls`),
    /// or `None` for `sslmode=disable`. Kept alongside `config` for the
    /// same reason: `subscribe_realtime`'s dedicated connection has to
    /// negotiate TLS the same way the pool does, not silently fall back
    /// to a plaintext `NoTls` connection just because it dials directly.
    tls: Option<rustls::ClientConfig>,
    /// The original six-value `sslmode` `connect` parsed (see
    /// `postgres_tls::extract_sslmode`), kept alongside `config`/`tls`
    /// (which only preserve the *collapsed* three-value driver mode) so
    /// `conn_params` can hand `pg_dump`/`pg_restore` the exact value the
    /// operator configured via `PGSSLMODE`.
    ssl_mode: postgres_tls::SslMode,
    /// Set by `close()` so the `subscribe_realtime` reconnect loop —
    /// which holds its own dedicated connection invisible to
    /// `pool.close()` — actually stops instead of reconnecting forever
    /// after the engine is supposed to be shut down.
    listen_cancelled: Arc<AtomicBool>,
    /// Wakes a `subscribe_realtime` loop parked in a reconnect sleep or
    /// waiting on its driver so shutdown is prompt rather than waiting
    /// out the current backoff; `listen_cancelled` is what actually
    /// guarantees correctness; this is purely latency.
    listen_cancel: Arc<tokio::sync::Notify>,
}

impl PostgresEngine {
    /// Connect to `url` (`postgres://user:pass@host/db?...`) with a pool
    /// of at most `max_size` connections. One connection is opened
    /// eagerly so a bad URL fails here rather than on first use.
    ///
    /// `sslmode` (see `crate::postgres_tls`) picks how the connection is
    /// secured: `disable` talks plaintext exactly as before this option
    /// existed; every other value negotiates TLS, which is what lets
    /// this reach a managed Postgres provider that requires it (Neon,
    /// Supabase, RDS) instead of failing outright.
    pub async fn connect(url: &str, max_size: usize) -> DbResult<Self> {
        let (sanitized, ssl_mode) = postgres_tls::extract_sslmode(url);
        let mut config = tokio_postgres::Config::from_str(&sanitized)?;
        config.ssl_mode(ssl_mode.driver_mode());
        let tls = postgres_tls::client_config(ssl_mode)?;

        let manager_config = ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        };
        let manager = match &tls {
            None => Manager::from_config(config.clone(), NoTls, manager_config),
            Some(tls) => Manager::from_config(
                config.clone(),
                MakeRustlsConnect::new(tls.clone()),
                manager_config,
            ),
        };
        let pool = Pool::builder(manager)
            .max_size(max_size.max(1))
            .build()
            .map_err(|e| DbError::Pool(e.to_string()))?;
        drop(pool.get().await.map_err(pool_err)?);
        Ok(PostgresEngine {
            pool,
            config,
            tls,
            ssl_mode,
            listen_cancelled: Arc::new(AtomicBool::new(false)),
            listen_cancel: Arc::new(tokio::sync::Notify::new()),
        })
    }

    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    /// Extract enough of `self.config` to shell out to `pg_dump`/
    /// `pg_restore` (`crate::pg_tools`). Errors only when the connection
    /// somehow has neither a host nor a password made of valid UTF-8 —
    /// both would already have made `connect` fail against a real
    /// server, so this is effectively unreachable outside of a
    /// hand-built `Config`.
    fn conn_params(&self) -> DbResult<PgConnParams> {
        let host = self
            .config
            .get_hosts()
            .first()
            .map(|h| match h {
                tokio_postgres::config::Host::Tcp(host) => host.clone(),
                #[cfg(unix)]
                tokio_postgres::config::Host::Unix(path) => path.to_string_lossy().into_owned(),
            })
            .ok_or_else(|| {
                DbError::Other(
                    "postgres connection has no host to pass to pg_dump/pg_restore".into(),
                )
            })?;
        let port = self.config.get_ports().first().copied().unwrap_or(5432);
        let user = self
            .config
            .get_user()
            .ok_or_else(|| DbError::Other("postgres connection has no user".into()))?
            .to_string();
        let password = self
            .config
            .get_password()
            .map(|bytes| {
                String::from_utf8(bytes.to_vec())
                    .map_err(|_| DbError::Other("postgres password is not valid UTF-8".into()))
            })
            .transpose()?;
        let dbname = self
            .config
            .get_dbname()
            .ok_or_else(|| DbError::Other("postgres connection has no database name".into()))?
            .to_string();
        Ok(PgConnParams {
            host,
            port,
            user,
            password,
            dbname,
            sslmode: self.ssl_mode,
        })
    }
}

fn pool_err(e: deadpool_postgres::PoolError) -> DbError {
    DbError::Pool(e.to_string())
}

/// Map driver errors onto the structured variants.
pub(crate) fn map_err(e: tokio_postgres::Error) -> DbError {
    if let Some(db) = e.as_db_error() {
        let code = db.code().code();
        if code == "23505" {
            return DbError::UniqueViolation(db.constraint().unwrap_or("").to_string());
        }
        if code.starts_with("23") {
            return DbError::Constraint(db.message().to_string());
        }
        // Every other server-reported error keeps the SQLSTATE and the
        // message. `tokio_postgres::Error`'s own Display is the bare
        // string "db error" for all of these, which once surfaced a
        // failed `CREATE VIEW` (an unquoted camelCase column folding to
        // lowercase) as the undiagnosable `Raw error: db error` of
        // issue #22.
        return DbError::Other(format!("db error {code}: {}", db.message()));
    }
    DbError::Postgres(e)
}

/// Bind adapter: encodes a [`Sql`] according to whatever type Postgres
/// inferred for the placeholder.
#[derive(Debug)]
struct Param<'a>(&'a Sql);

type BoxError = Box<dyn StdError + Sync + Send>;

impl ToSql for Param<'_> {
    fn to_sql(&self, ty: &Type, out: &mut BytesMut) -> Result<IsNull, BoxError> {
        match self.0 {
            Sql::Null => Ok(IsNull::Yes),
            Sql::Int(i) => match *ty {
                Type::INT2 => (*i as i16).to_sql(ty, out),
                Type::INT4 => (*i as i32).to_sql(ty, out),
                Type::FLOAT4 => (*i as f32).to_sql(ty, out),
                Type::FLOAT8 => (*i as f64).to_sql(ty, out),
                Type::BOOL => (*i != 0).to_sql(ty, out),
                Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::UNKNOWN => {
                    i.to_string().to_sql(&Type::TEXT, out)
                }
                Type::JSON | Type::JSONB => write_json(ty, &i.to_string(), out),
                Type::NUMERIC => write_numeric(&i.to_string(), out),
                _ => i.to_sql(&Type::INT8, out),
            },
            Sql::Real(f) => match *ty {
                Type::FLOAT4 => (*f as f32).to_sql(ty, out),
                Type::INT2 => (*f as i16).to_sql(ty, out),
                Type::INT4 => (*f as i32).to_sql(ty, out),
                Type::INT8 => (*f as i64).to_sql(ty, out),
                Type::BOOL => (*f != 0.0).to_sql(ty, out),
                Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::UNKNOWN => {
                    f.to_string().to_sql(&Type::TEXT, out)
                }
                Type::JSON | Type::JSONB => write_json(ty, &f.to_string(), out),
                Type::NUMERIC => write_numeric(&f.to_string(), out),
                _ => f.to_sql(&Type::FLOAT8, out),
            },
            Sql::Text(s) => match *ty {
                Type::JSON | Type::JSONB => write_json(ty, s, out),
                Type::INT2 => s.trim().parse::<i16>()?.to_sql(ty, out),
                Type::INT4 => s.trim().parse::<i32>()?.to_sql(ty, out),
                Type::INT8 => s.trim().parse::<i64>()?.to_sql(ty, out),
                Type::FLOAT4 => s.trim().parse::<f32>()?.to_sql(ty, out),
                Type::FLOAT8 => s.trim().parse::<f64>()?.to_sql(ty, out),
                Type::NUMERIC => write_numeric(s, out),
                Type::BOOL => {
                    let b = matches!(s.trim(), "1" | "true" | "t" | "TRUE" | "yes");
                    b.to_sql(ty, out)
                }
                Type::TIMESTAMP => match DateTime::parse(s) {
                    Some(dt) => dt.inner().naive_utc().to_sql(ty, out),
                    None => Ok(IsNull::Yes),
                },
                Type::TIMESTAMPTZ => match DateTime::parse(s) {
                    Some(dt) => dt.inner().to_sql(ty, out),
                    None => Ok(IsNull::Yes),
                },
                Type::DATE => match DateTime::parse(s) {
                    Some(dt) => dt.inner().date_naive().to_sql(ty, out),
                    None => Ok(IsNull::Yes),
                },
                Type::BYTEA => s.as_bytes().to_sql(ty, out),
                _ => s.as_str().to_sql(&Type::TEXT, out),
            },
            Sql::Blob(b) => match *ty {
                Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::UNKNOWN => {
                    String::from_utf8_lossy(b).as_ref().to_sql(&Type::TEXT, out)
                }
                _ => b.as_slice().to_sql(&Type::BYTEA, out),
            },
        }
    }

    fn accepts(_ty: &Type) -> bool {
        true
    }

    to_sql_checked!();
}

/// `json` is sent as raw text; `jsonb` prefixes a one-byte version.
fn write_json(ty: &Type, text: &str, out: &mut BytesMut) -> Result<IsNull, BoxError> {
    if *ty == Type::JSONB {
        out.extend_from_slice(&[1]);
    }
    out.extend_from_slice(text.as_bytes());
    Ok(IsNull::No)
}

/// Encode a plain decimal string (`-12.5`, `42`, `.5`) as Postgres'
/// binary `numeric`: header `{ndigits, weight, sign, dscale}` then
/// base-10000 digit groups, all big-endian 16-bit. The server strips
/// leading/trailing zero groups itself, so no normalization is needed.
fn write_numeric(text: &str, out: &mut BytesMut) -> Result<IsNull, BoxError> {
    let t = text.trim();
    let (negative, t) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    if t.eq_ignore_ascii_case("nan") {
        out.extend_from_slice(&[0, 0, 0, 0, 0xC0, 0, 0, 0]);
        return Ok(IsNull::No);
    }
    let (int_part, frac_part) = t.split_once('.').unwrap_or((t, ""));
    let digits_only = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if (int_part.is_empty() && frac_part.is_empty())
        || !digits_only(int_part)
        || !digits_only(frac_part)
    {
        return Err(format!("cannot bind {text:?} as numeric").into());
    }
    let group = |s: &[u8]| -> u16 { s.iter().fold(0u16, |acc, b| acc * 10 + u16::from(b - b'0')) };
    let mut int_digits = vec![b'0'; (4 - int_part.len() % 4) % 4];
    int_digits.extend_from_slice(int_part.as_bytes());
    let mut frac_digits = frac_part.as_bytes().to_vec();
    frac_digits.resize(frac_digits.len().div_ceil(4) * 4, b'0');
    let int_groups: Vec<u16> = int_digits.chunks(4).map(group).collect();
    let frac_groups: Vec<u16> = frac_digits.chunks(4).map(group).collect();
    let ndigits = int_groups.len() + frac_groups.len();
    let weight = int_groups.len() as i16 - 1;
    let sign: u16 = if negative { 0x4000 } else { 0 };
    out.extend_from_slice(&(ndigits as i16).to_be_bytes());
    out.extend_from_slice(&weight.to_be_bytes());
    out.extend_from_slice(&sign.to_be_bytes());
    out.extend_from_slice(&(frac_part.len() as u16).to_be_bytes());
    for g in int_groups.iter().chain(frac_groups.iter()) {
        out.extend_from_slice(&g.to_be_bytes());
    }
    Ok(IsNull::No)
}

/// Minimal decoder for Postgres' binary `numeric` (base-10000 digits) to
/// `f64`, so aggregates like `AVG(int)` in view queries decode without
/// pulling in a decimal crate.
struct PgNumeric(f64);

impl<'a> FromSql<'a> for PgNumeric {
    fn from_sql(_ty: &Type, raw: &'a [u8]) -> Result<Self, BoxError> {
        if raw.len() < 8 {
            return Err("numeric: truncated header".into());
        }
        let rd = |i: usize| u16::from_be_bytes([raw[i], raw[i + 1]]);
        let ndigits = rd(0) as usize;
        let weight = rd(2) as i16 as i32;
        let sign = rd(4);
        if raw.len() < 8 + ndigits * 2 {
            return Err("numeric: truncated digits".into());
        }
        if sign == 0xC000 {
            return Ok(PgNumeric(f64::NAN));
        }
        let mut value = 0f64;
        for i in 0..ndigits {
            value += f64::from(rd(8 + 2 * i)) * 10000f64.powi(weight - i as i32);
        }
        if sign == 0x4000 {
            value = -value;
        }
        Ok(PgNumeric(value))
    }

    fn accepts(ty: &Type) -> bool {
        *ty == Type::NUMERIC
    }
}

fn pb_naive(dt: NaiveDateTime) -> Sql {
    Sql::Text(DateTime::from_utc(dt.and_utc()).to_pb_string())
}

fn decode_column(row: &tokio_postgres::Row, i: usize, ty: &Type) -> DbResult<Sql> {
    let v = match *ty {
        Type::BOOL => row
            .try_get::<_, Option<bool>>(i)?
            .map(|b| Sql::Int(b as i64)),
        Type::INT2 => row
            .try_get::<_, Option<i16>>(i)?
            .map(|n| Sql::Int(n as i64)),
        Type::INT4 => row
            .try_get::<_, Option<i32>>(i)?
            .map(|n| Sql::Int(n as i64)),
        Type::INT8 => row.try_get::<_, Option<i64>>(i)?.map(Sql::Int),
        Type::OID => row
            .try_get::<_, Option<u32>>(i)?
            .map(|n| Sql::Int(n as i64)),
        Type::FLOAT4 => row
            .try_get::<_, Option<f32>>(i)?
            .map(|n| Sql::Real(n as f64)),
        Type::FLOAT8 => row.try_get::<_, Option<f64>>(i)?.map(Sql::Real),
        Type::NUMERIC => row
            .try_get::<_, Option<PgNumeric>>(i)?
            .map(|n| Sql::Real(n.0)),
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::UNKNOWN => {
            row.try_get::<_, Option<String>>(i)?.map(Sql::Text)
        }
        Type::BYTEA => row.try_get::<_, Option<Vec<u8>>>(i)?.map(Sql::Blob),
        Type::JSON | Type::JSONB => row
            .try_get::<_, Option<serde_json::Value>>(i)?
            .map(|v| Sql::Text(v.to_string())),
        Type::TIMESTAMP => row.try_get::<_, Option<NaiveDateTime>>(i)?.map(pb_naive),
        Type::TIMESTAMPTZ => row
            .try_get::<_, Option<chrono::DateTime<Utc>>>(i)?
            .map(|dt| Sql::Text(DateTime::from_utc(dt).to_pb_string())),
        Type::DATE => row
            .try_get::<_, Option<NaiveDate>>(i)?
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .map(pb_naive),
        _ => {
            return Err(DbError::Unsupported(format!(
                "postgres column type {ty} (column {i})"
            )))
        }
    };
    Ok(v.unwrap_or(Sql::Null))
}

fn decode_rows(rows: Vec<tokio_postgres::Row>) -> DbResult<Vec<Row>> {
    let Some(first) = rows.first() else {
        return Ok(Vec::new());
    };
    let columns: Arc<[String]> = first
        .columns()
        .iter()
        .map(|c| c.name().to_string())
        .collect::<Vec<_>>()
        .into();
    let types: Vec<Type> = first.columns().iter().map(|c| c.type_().clone()).collect();
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let mut values = Vec::with_capacity(types.len());
        for (i, ty) in types.iter().enumerate() {
            values.push(decode_column(row, i, ty)?);
        }
        out.push(Row {
            columns: columns.clone(),
            values,
        });
    }
    Ok(out)
}

async fn client_query(
    client: &tokio_postgres::Client,
    sql: &str,
    params: &[Sql],
) -> DbResult<Vec<Row>> {
    let stmt = client.prepare(sql).await.map_err(map_err)?;
    let bound: Vec<Param<'_>> = params.iter().map(Param).collect();
    let refs: Vec<&(dyn ToSql + Sync)> = bound.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
    let rows = client.query(&stmt, &refs).await.map_err(map_err)?;
    decode_rows(rows)
}

async fn client_execute(
    client: &tokio_postgres::Client,
    sql: &str,
    params: &[Sql],
) -> DbResult<u64> {
    let stmt = client.prepare(sql).await.map_err(map_err)?;
    let bound: Vec<Param<'_>> = params.iter().map(Param).collect();
    let refs: Vec<&(dyn ToSql + Sync)> = bound.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
    client.execute(&stmt, &refs).await.map_err(map_err)
}

#[async_trait]
impl Executor for PostgresEngine {
    fn dialect(&self) -> Dialect {
        Dialect::Postgres
    }

    async fn query(&self, sql: &str, params: &[Sql]) -> DbResult<Vec<Row>> {
        let client = self.pool.get().await.map_err(pool_err)?;
        let stmt = client.prepare_cached(sql).await.map_err(map_err)?;
        let bound: Vec<Param<'_>> = params.iter().map(Param).collect();
        let refs: Vec<&(dyn ToSql + Sync)> =
            bound.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
        let rows = client.query(&stmt, &refs).await.map_err(map_err)?;
        decode_rows(rows)
    }

    async fn execute(&self, sql: &str, params: &[Sql]) -> DbResult<u64> {
        let client = self.pool.get().await.map_err(pool_err)?;
        let stmt = client.prepare_cached(sql).await.map_err(map_err)?;
        let bound: Vec<Param<'_>> = params.iter().map(Param).collect();
        let refs: Vec<&(dyn ToSql + Sync)> =
            bound.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
        client.execute(&stmt, &refs).await.map_err(map_err)
    }

    /// Real cancellation, not just a bounded wait — see the trait doc.
    /// `crates/server/src/routes/sql_console.rs` is this method's only
    /// caller today, and it needs a stronger guarantee than "bound the
    /// wait": that module's `is_read_statement` treats anything starting
    /// with `WITH` as a read purely by syntax ("a CTE... only ever feeds
    /// a `SELECT`"), which is a guess, not a fact — a data-modifying CTE
    /// like `WITH x AS (UPDATE _superusers SET role='owner' RETURNING 1)
    /// SELECT * FROM x` would otherwise run for real through this same
    /// `query` path. Running it inside `BEGIN READ ONLY` instead makes
    /// the *server* reject any write the statement attempts, regardless
    /// of whether `crate::routes::sql_console::references_table`'s own
    /// coarse, no-parser scan happens to recognize the table it touches.
    /// Always rolled back afterward: a read-only transaction never has
    /// anything to commit, but the pooled connection still has to be
    /// handed back clean.
    ///
    /// `SET LOCAL statement_timeout` runs inside that same transaction
    /// so a runaway statement is actually cancelled *server-side* once
    /// `timeout` elapses, closing the gap the trait doc used to call out
    /// for every backend without SQLite's `sqlite3_interrupt`. Failing to
    /// set it is only logged, not fatal: the read-only transaction above
    /// is the guard this override exists for, and a lost timeout just
    /// falls back to the old bounded-wait behavior for this one call.
    async fn query_interruptible(
        &self,
        sql: &str,
        params: &[Sql],
        timeout: Duration,
    ) -> DbResult<Vec<Row>> {
        let client = self.pool.get().await.map_err(pool_err)?;
        client
            .batch_execute("BEGIN READ ONLY")
            .await
            .map_err(map_err)?;
        let timeout_ms = timeout.as_millis().max(1);
        if let Err(e) = client
            .batch_execute(&format!("SET LOCAL statement_timeout = {timeout_ms}"))
            .await
        {
            tracing::warn!(
                error = %e,
                "failed to set statement_timeout for an ad-hoc SQL console query"
            );
        }
        let result = client_query(&client, sql, params).await;
        if let Err(e) = client.batch_execute("ROLLBACK").await {
            tracing::warn!(
                error = %e,
                "failed to close the read-only transaction for an ad-hoc SQL console query"
            );
        }
        result
    }

    /// See the trait doc: a real server-side `prepare` (the extended-query
    /// `Parse` message) both validates syntax and refuses more than one
    /// statement, so this is a stronger check on Postgres than on SQLite.
    async fn prepare_check(&self, sql: &str) -> DbResult<()> {
        let client = self.pool.get().await.map_err(pool_err)?;
        client.prepare(sql).await.map(|_| ()).map_err(map_err)
    }
}

#[async_trait]
impl Engine for PostgresEngine {
    async fn begin(&self) -> DbResult<Transaction> {
        let client = self.pool.get().await.map_err(pool_err)?;
        client.batch_execute("BEGIN").await.map_err(map_err)?;
        Ok(Transaction::new(Box::new(PgTransaction {
            client: Some(client),
            open: true,
        })))
    }

    async fn table_exists(&self, table: &str) -> DbResult<bool> {
        let rows = self
            .query(
                "SELECT 1 FROM information_schema.tables \
                 WHERE table_schema = current_schema() AND table_name = $1",
                &[Sql::from(table)],
            )
            .await?;
        Ok(!rows.is_empty())
    }

    async fn table_columns(&self, table: &str) -> DbResult<Vec<String>> {
        let rows = self
            .query(
                "SELECT column_name::text FROM information_schema.columns \
                 WHERE table_schema = current_schema() AND table_name = $1 \
                 ORDER BY ordinal_position",
                &[Sql::from(table)],
            )
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|r| r.values.into_iter().next().map(Sql::into_string))
            .collect())
    }

    async fn table_indexes(&self, table: &str) -> DbResult<Vec<String>> {
        let rows = self
            .query(
                // Explicit indexes only: the ones backing PRIMARY KEY /
                // UNIQUE constraints are implicit, like SQLite's
                // `sqlite_autoindex_*`.
                "SELECT i.indexname::text FROM pg_indexes i \
                 WHERE i.schemaname = current_schema() AND i.tablename = $1 \
                 AND NOT EXISTS (SELECT 1 FROM pg_constraint c \
                                 WHERE c.conname = i.indexname AND c.contype IN ('p', 'u')) \
                 ORDER BY i.indexname",
                &[Sql::from(table)],
            )
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|r| r.values.into_iter().next().map(Sql::into_string))
            .collect())
    }

    async fn optimize(&self) -> DbResult<()> {
        self.execute("ANALYZE", &[]).await.map(|_| ())
    }

    async fn snapshot_to(&self, dest_path: &str) -> DbResult<()> {
        let params = self.conn_params()?;
        let pg_dump = pg_tools::find_pg_tool("pg_dump", pg_tools::CB_PG_DUMP_PATH)?;
        let invocation =
            pg_tools::build_pg_dump_invocation(pg_dump, &params, std::path::Path::new(dest_path));
        pg_tools::run_tool(invocation).await
    }

    async fn restore_from(&self, source_path: &str) -> DbResult<()> {
        let params = self.conn_params()?;
        let pg_restore = pg_tools::find_pg_tool("pg_restore", pg_tools::CB_PG_RESTORE_PATH)?;
        let invocation = pg_tools::build_pg_restore_invocation(
            pg_restore,
            &params,
            std::path::Path::new(source_path),
        );
        pg_tools::run_tool(invocation).await
    }

    async fn close(&self) -> DbResult<()> {
        self.listen_cancelled.store(true, Ordering::SeqCst);
        self.listen_cancel.notify_waiters();
        self.pool.close();
        Ok(())
    }

    fn supports_cross_node(&self) -> bool {
        true
    }

    async fn notify_realtime(&self, payload: &str) -> DbResult<()> {
        if payload.len() >= NOTIFY_PAYLOAD_LIMIT {
            return Err(DbError::Other(format!(
                "realtime NOTIFY payload is {} bytes, at or over postgres's {NOTIFY_PAYLOAD_LIMIT}-byte \
                 limit (postgres itself rejects a payload once strlen(payload) >= that limit); \
                 crate::realtime::publish should only ever send an id/collection/action (and, \
                 for a delete, one record's snapshot), never an unbounded blob",
                payload.len()
            )));
        }
        // `pg_notify` rather than a literal `NOTIFY channel, 'payload'`
        // string: the channel name can't be a bound parameter but the
        // payload can, which avoids hand-quoting it.
        self.execute(
            "SELECT pg_notify($1, $2)",
            &[Sql::from(REALTIME_CHANNEL), Sql::from(payload)],
        )
        .await?;
        Ok(())
    }

    fn subscribe_realtime(&self, on_notify: Arc<dyn Fn(String) + Send + Sync>) {
        // Dial with the same TLS settings the pool uses (see the `tls`
        // field's doc) rather than a hard-coded `NoTls`: this dedicated
        // connection has to satisfy `sslmode` just as much as any pooled
        // one does, or a `require`-and-up deployment would have every
        // *pooled* connection encrypted while this one alone connected
        // in the clear (and would fail outright once the server refuses
        // a plaintext connection).
        match &self.tls {
            None => self.spawn_listen_loop(NoTls, on_notify),
            Some(tls) => self.spawn_listen_loop(MakeRustlsConnect::new(tls.clone()), on_notify),
        }
    }
}

impl PostgresEngine {
    /// The body of [`Engine::subscribe_realtime`], generic over the TLS
    /// connector so it can be driven with either `NoTls` or
    /// [`MakeRustlsConnect`] without a shared runtime type for the two
    /// (`Manager::from_config` sidesteps the same problem by boxing
    /// internally; `tokio_postgres::Config::connect` doesn't, so this
    /// function is generic instead). The trait bounds are copied from
    /// `deadpool_postgres::Manager::from_config`'s own, which is exactly
    /// what a `MakeTlsConnect` needs to be usable across an internal
    /// reconnect loop like this one.
    fn spawn_listen_loop<T>(&self, connector: T, on_notify: Arc<dyn Fn(String) + Send + Sync>)
    where
        T: MakeTlsConnect<Socket> + Clone + Sync + Send + 'static,
        T::Stream: Sync + Send,
        T::TlsConnect: Sync + Send,
        <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
    {
        let config = self.config.clone();
        let cancelled = self.listen_cancelled.clone();
        let cancel = self.listen_cancel.clone();
        tokio::spawn(async move {
            loop {
                if cancelled.load(Ordering::SeqCst) {
                    tracing::debug!("realtime LISTEN loop stopping: engine closed");
                    return;
                }
                let (client, mut connection) = match config.connect(connector.clone()).await {
                    Ok(pair) => pair,
                    Err(e) => {
                        tracing::warn!(error = %e, "realtime LISTEN connect failed; retrying");
                        tokio::select! {
                            _ = tokio::time::sleep(Duration::from_secs(3)) => {}
                            _ = cancel.notified() => {}
                        }
                        continue;
                    }
                };
                if cancelled.load(Ordering::SeqCst) {
                    return;
                }

                // Drives the socket *and* surfaces `NOTIFY` payloads.
                // `Connection::poll_message`'s own docs say to use this
                // instead of spawning the connection as a bare `Future`
                // exactly when the caller wants async messages, which is
                // the whole point here.
                let on_notify = on_notify.clone();
                let driver = tokio::spawn(async move {
                    loop {
                        match std::future::poll_fn(|cx| connection.poll_message(cx)).await {
                            Some(Ok(AsyncMessage::Notification(n))) => {
                                on_notify(n.payload().to_string());
                            }
                            Some(Ok(_)) => {}
                            Some(Err(e)) => {
                                tracing::warn!(error = %e, "realtime LISTEN connection error");
                                return;
                            }
                            None => return,
                        }
                    }
                });

                if let Err(e) = client
                    .batch_execute(&format!("LISTEN {REALTIME_CHANNEL}"))
                    .await
                {
                    tracing::warn!(error = %e, "realtime LISTEN setup failed; retrying");
                    driver.abort();
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(3)) => {}
                        _ = cancel.notified() => {}
                    }
                    continue;
                }
                tracing::debug!("realtime cross-node LISTEN connected");

                // `client` must outlive the driver: `Connection` shuts
                // itself down once its `Client` has dropped and any
                // outstanding work finishes, and an idle `LISTEN` isn't
                // "outstanding work" that would keep it alive on its own.
                let _client = client;
                tokio::select! {
                    _ = driver => {}
                    // `close()` was called: stop driving this connection
                    // and exit the loop instead of reconnecting forever
                    // on a pool the caller believes is fully shut down.
                    _ = cancel.notified() => return,
                }
                if cancelled.load(Ordering::SeqCst) {
                    return;
                }
                tracing::warn!("realtime LISTEN connection lost; reconnecting");
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(3)) => {}
                    _ = cancel.notified() => {}
                }
            }
        });
    }
}

/// Channel every `PostgresEngine` publishes realtime events to and
/// listens on; see `Engine::notify_realtime`/`subscribe_realtime`.
const REALTIME_CHANNEL: &str = "cratebase_realtime";

/// Postgres's own hard ceiling on a `NOTIFY` payload, enforced
/// server-side. Checked up front so an oversized payload is a clear
/// error here rather than an opaque one from the driver (or, worse, a
/// payload the server truncates without telling us).
pub const NOTIFY_PAYLOAD_LIMIT: usize = 8000;

/// A transaction pinned to one pooled connection. Dropping it without
/// `commit` detaches the connection from the pool and rolls back on a
/// background task, so a half-done transaction can never leak into the
/// next checkout.
struct PgTransaction {
    client: Option<deadpool_postgres::Object>,
    open: bool,
}

impl PgTransaction {
    fn client(&self) -> DbResult<&tokio_postgres::Client> {
        self.client
            .as_deref()
            .map(|c| &**c)
            .ok_or_else(|| DbError::Pool("transaction already finished".into()))
    }

    async fn finish(mut self: Box<Self>, stmt: &str) -> DbResult<()> {
        self.client()?.batch_execute(stmt).await.map_err(map_err)?;
        self.open = false;
        Ok(())
    }
}

#[async_trait]
impl Executor for PgTransaction {
    fn dialect(&self) -> Dialect {
        Dialect::Postgres
    }

    async fn query(&self, sql: &str, params: &[Sql]) -> DbResult<Vec<Row>> {
        client_query(self.client()?, sql, params).await
    }

    async fn execute(&self, sql: &str, params: &[Sql]) -> DbResult<u64> {
        client_execute(self.client()?, sql, params).await
    }

    async fn prepare_check(&self, sql: &str) -> DbResult<()> {
        self.client()?
            .prepare(sql)
            .await
            .map(|_| ())
            .map_err(map_err)
    }
}

#[async_trait]
impl TransactionImpl for PgTransaction {
    async fn commit(self: Box<Self>) -> DbResult<()> {
        self.finish("COMMIT").await
    }

    async fn rollback(self: Box<Self>) -> DbResult<()> {
        self.finish("ROLLBACK").await
    }
}

impl Drop for PgTransaction {
    fn drop(&mut self) {
        if !self.open {
            return;
        }
        let Some(object) = self.client.take() else {
            return;
        };
        let client = deadpool_postgres::Object::take(object);
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    if let Err(e) = client.batch_execute("ROLLBACK").await {
                        tracing::warn!(error = %e, "rollback on dropped transaction failed");
                    }
                    drop(client);
                });
            }
            // No runtime: dropping the detached client closes the socket,
            // which the server treats as a rollback.
            Err(_) => drop(client),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Postgres tests run only when `TEST_POSTGRES_URL` points at a
    /// scratch database; they skip silently otherwise so the default
    /// `cargo test` needs no server.
    pub(crate) async fn test_engine() -> Option<PostgresEngine> {
        let url = std::env::var("TEST_POSTGRES_URL").ok()?;
        Some(PostgresEngine::connect(&url, 4).await.expect("connect"))
    }

    #[tokio::test]
    async fn round_trips_values_and_maps_errors() {
        let Some(e) = test_engine().await else { return };
        e.execute("DROP TABLE IF EXISTS cb_pg_engine_t", &[])
            .await
            .unwrap();
        e.execute(
            "CREATE TABLE cb_pg_engine_t (id TEXT PRIMARY KEY, n DOUBLE PRECISION, i BIGINT, \
             b INTEGER, j JSONB, t TIMESTAMP, bin BYTEA, num NUMERIC)",
            &[],
        )
        .await
        .unwrap();
        e.execute(
            "INSERT INTO cb_pg_engine_t VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[
                Sql::from("a"),
                Sql::Int(3),
                Sql::Real(4.0),
                Sql::from(true),
                Sql::from("{\"k\":[1]}"),
                Sql::from("2026-09-03 12:44:06.146Z"),
                Sql::Blob(vec![9]),
                Sql::from("-12.5"),
            ],
        )
        .await
        .unwrap();
        let rows = e.query("SELECT * FROM cb_pg_engine_t", &[]).await.unwrap();
        let r = &rows[0];
        assert_eq!(r.get("n"), Some(&Sql::Real(3.0)));
        assert_eq!(r.get("i"), Some(&Sql::Int(4)));
        assert_eq!(r.get("b"), Some(&Sql::Int(1)));
        assert_eq!(r.get_str("j"), Some("{\"k\":[1]}"));
        assert_eq!(r.get_str("t"), Some("2026-09-03 12:44:06.146Z"));
        assert_eq!(r.get("bin"), Some(&Sql::Blob(vec![9])));
        assert_eq!(r.get("num"), Some(&Sql::Real(-12.5)));

        // Numeric encoding round-trips every Sql shape through the server.
        let rows = e
            .query(
                "SELECT $1::numeric AS a, $2::numeric AS b, $3::numeric AS c, $4::numeric AS d",
                &[
                    Sql::Int(123_456_789),
                    Sql::Real(0.25),
                    Sql::from("-12345.6789"),
                    Sql::from(".5"),
                ],
            )
            .await
            .unwrap();
        assert_eq!(rows[0].get("a"), Some(&Sql::Real(123_456_789.0)));
        assert_eq!(rows[0].get("b"), Some(&Sql::Real(0.25)));
        assert_eq!(rows[0].get("c"), Some(&Sql::Real(-12345.6789)));
        assert_eq!(rows[0].get("d"), Some(&Sql::Real(0.5)));

        let err = e
            .execute(
                "INSERT INTO cb_pg_engine_t (id) VALUES ($1)",
                &[Sql::from("a")],
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::UniqueViolation(_)), "{err:?}");

        let tx = e.begin().await.unwrap();
        tx.execute(
            "INSERT INTO cb_pg_engine_t (id) VALUES ($1)",
            &[Sql::from("dropped")],
        )
        .await
        .unwrap();
        drop(tx);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let rows = e
            .query(
                "SELECT COUNT(*) FROM cb_pg_engine_t WHERE id = $1",
                &[Sql::from("dropped")],
            )
            .await
            .unwrap();
        assert_eq!(rows[0].values[0], Sql::Int(0));

        assert!(e.table_exists("cb_pg_engine_t").await.unwrap());
        assert_eq!(e.table_columns("cb_pg_engine_t").await.unwrap()[0], "id");
        // Constraint-backed indexes (the pkey) are hidden; explicit ones show.
        assert!(e.table_indexes("cb_pg_engine_t").await.unwrap().is_empty());
        e.execute("CREATE INDEX cb_pg_engine_t_i ON cb_pg_engine_t (i)", &[])
            .await
            .unwrap();
        assert_eq!(
            e.table_indexes("cb_pg_engine_t").await.unwrap(),
            vec!["cb_pg_engine_t_i"]
        );
        e.optimize().await.unwrap();
        e.execute("DROP TABLE cb_pg_engine_t", &[]).await.unwrap();
    }

    /// Two independent `PostgresEngine`s against the same database,
    /// standing in for two app processes: a `notify_realtime` on one
    /// arrives at the other's `subscribe_realtime` listener. This is the
    /// plumbing `crates/server/src/realtime.rs` builds cross-node
    /// fan-out on top of.
    #[tokio::test]
    async fn realtime_notify_reaches_a_second_engine() {
        let Some(sender) = test_engine().await else {
            return;
        };
        let receiver = test_engine().await.expect("second connection");

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        receiver.subscribe_realtime(Arc::new(move |payload: String| {
            let _ = tx.send(payload);
        }));

        // `subscribe_realtime` has no readiness signal by design (see its
        // doc comment), so poll rather than guess a fixed delay: keep
        // notifying until the listener has had time to come up and catch
        // one.
        let mut delivered = None;
        for _ in 0..50 {
            sender.notify_realtime("hello-from-sender").await.unwrap();
            if let Ok(Some(payload)) =
                tokio::time::timeout(Duration::from_millis(200), rx.recv()).await
            {
                delivered = Some(payload);
                break;
            }
        }
        assert_eq!(delivered.as_deref(), Some("hello-from-sender"));

        // An oversized payload is rejected up front, not silently
        // dropped or truncated.
        let huge = "x".repeat(NOTIFY_PAYLOAD_LIMIT + 1);
        let err = sender.notify_realtime(&huge).await.unwrap_err();
        assert!(matches!(err, DbError::Other(_)), "{err:?}");
    }
}
