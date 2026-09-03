use cratebase_core::{new_id, now};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::error::{DbError, DbResult};
use crate::pool::Db;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Admin {
    pub id: String,
    pub email: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub created: String,
    pub updated: String,
}

pub async fn count_admins(db: &Db) -> DbResult<i64> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _admins")
        .fetch_one(&db.pool)
        .await?;
    Ok(count)
}

pub async fn create_admin(db: &Db, email: &str, password_hash: &str) -> DbResult<Admin> {
    let id = new_id();
    let ts = now();
    sqlx::query("INSERT INTO _admins (id, email, password_hash, created, updated) VALUES ($1, $2, $3, $4, $5)")
        .bind(&id)
        .bind(email)
        .bind(password_hash)
        .bind(&ts)
        .bind(&ts)
        .execute(&db.pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(db_err) = &e {
                if db_err.is_unique_violation() {
                    return DbError::UniqueViolation("email".into());
                }
            }
            DbError::Sqlx(e)
        })?;
    get_admin_by_id(db, &id).await
}

pub async fn get_admin_by_email(db: &Db, email: &str) -> DbResult<Admin> {
    let row = sqlx::query("SELECT * FROM _admins WHERE email = $1")
        .bind(email)
        .fetch_optional(&db.pool)
        .await?
        .ok_or(DbError::NotFound)?;
    row_to_admin(&row)
}

pub async fn get_admin_by_id(db: &Db, id: &str) -> DbResult<Admin> {
    let row = sqlx::query("SELECT * FROM _admins WHERE id = $1")
        .bind(id)
        .fetch_optional(&db.pool)
        .await?
        .ok_or(DbError::NotFound)?;
    row_to_admin(&row)
}

pub async fn list_admins(db: &Db) -> DbResult<Vec<Admin>> {
    let rows = sqlx::query("SELECT * FROM _admins ORDER BY created")
        .fetch_all(&db.pool)
        .await?;
    rows.iter().map(row_to_admin).collect()
}

pub async fn update_admin_password(db: &Db, id: &str, password_hash: &str) -> DbResult<()> {
    let result = sqlx::query("UPDATE _admins SET password_hash = $1, updated = $2 WHERE id = $3")
        .bind(password_hash)
        .bind(now())
        .bind(id)
        .execute(&db.pool)
        .await?;
    if result.rows_affected() == 0 {
        Err(DbError::NotFound)
    } else {
        Ok(())
    }
}

pub async fn delete_admin(db: &Db, id: &str) -> DbResult<()> {
    let result = sqlx::query("DELETE FROM _admins WHERE id = $1")
        .bind(id)
        .execute(&db.pool)
        .await?;
    if result.rows_affected() == 0 {
        Err(DbError::NotFound)
    } else {
        Ok(())
    }
}

fn row_to_admin(row: &sqlx::any::AnyRow) -> DbResult<Admin> {
    Ok(Admin {
        id: row.try_get("id")?,
        email: row.try_get("email")?,
        password_hash: row.try_get("password_hash")?,
        created: row.try_get("created")?,
        updated: row.try_get("updated")?,
    })
}
