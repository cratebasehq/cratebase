//! DDL derived from a [`Collection`]: the physical table (or view)
//! named exactly after the collection, one column per field, and the
//! raw `CREATE INDEX` statements PocketBase keeps in `indexes[]`.
//!
//! [`sync`] diffs `previous` against `next` **by field id** so a renamed
//! field becomes `RENAME COLUMN` (data kept) while a field whose
//! physical shape changed (text → number, single → multi) is dropped and
//! re-added (data on that column is lost; there is no portable cast that
//! would not surprise someone).
//!
//! Every column is nullable and carries a zero default (`''`, `0`, or
//! `NULL` for JSON-shaped fields) so partial inserts and PocketBase-style
//! view queries behave the same on both backends.

use cratebase_core::{is_valid_identifier, Collection, Field, FieldType};

use crate::backend::Backend;
use crate::engine::{quote_ident, Executor};
use crate::error::{DbError, DbResult};

/// The three physical shapes a column can have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalKind {
    Text,
    Number,
    Bool,
}

/// Multi-valued select/file/relation fields store a JSON array as text.
pub fn is_multiple(field: &Field) -> bool {
    field.is_multiple()
}

pub fn physical_kind(field: &Field) -> PhysicalKind {
    if is_multiple(field) {
        return PhysicalKind::Text;
    }
    match field.field_type() {
        FieldType::Number => PhysicalKind::Number,
        FieldType::Bool => PhysicalKind::Bool,
        _ => PhysicalKind::Text,
    }
}

/// The SQL column type for `field` on `backend`.
pub fn physical_type(backend: Backend, field: &Field) -> &'static str {
    match physical_kind(field) {
        PhysicalKind::Text => backend.text_type(),
        PhysicalKind::Number => backend.number_type(),
        PhysicalKind::Bool => backend.bool_type(),
    }
}

/// The column default: JSON-shaped values default to NULL, everything
/// else to its zero value.
pub fn column_default(field: &Field) -> &'static str {
    if is_multiple(field) {
        return "DEFAULT NULL";
    }
    match field.field_type() {
        FieldType::Number | FieldType::Bool => "DEFAULT 0",
        FieldType::Json | FieldType::GeoPoint => "DEFAULT NULL",
        _ => "DEFAULT ''",
    }
}

fn ident(name: &str) -> DbResult<String> {
    if !is_valid_identifier(name) {
        return Err(DbError::InvalidIdentifier(name.to_string()));
    }
    Ok(quote_ident(name))
}

/// `"name" TYPE DEFAULT x` (or `"id" TEXT PRIMARY KEY NOT NULL`).
pub fn column_ddl(backend: Backend, field: &Field) -> DbResult<String> {
    let name = ident(&field.name)?;
    if field.is_primary_key() {
        return Ok(format!("{name} TEXT PRIMARY KEY NOT NULL"));
    }
    Ok(format!(
        "{name} {} {}",
        physical_type(backend, field),
        column_default(field)
    ))
}

/// `CREATE TABLE IF NOT EXISTS` for a base/auth collection.
pub fn create_table_sql(backend: Backend, collection: &Collection) -> DbResult<String> {
    let table = ident(&collection.name)?;
    let mut cols = Vec::with_capacity(collection.fields.len() + 1);
    if !collection.fields.iter().any(|f| f.name == "id") {
        cols.push("\"id\" TEXT PRIMARY KEY NOT NULL".to_string());
    }
    for f in &collection.fields {
        cols.push(column_ddl(backend, f)?);
    }
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {table} ({})",
        cols.join(", ")
    ))
}

// --- indexes -------------------------------------------------------------

/// The parts of a `CREATE INDEX` statement we need: enough to name it,
/// point it at the right table, and re-emit it in double-quoted form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexDef {
    pub unique: bool,
    pub name: String,
    pub table: String,
    /// The text between the parentheses, verbatim.
    pub columns_raw: String,
    /// The text after `WHERE`, verbatim, if any.
    pub where_raw: Option<String>,
}

impl IndexDef {
    /// Whether the index mentions `column` (as a whole word) in its
    /// column list or predicate.
    pub fn references_column(&self, column: &str) -> bool {
        let mentions = |text: &str| {
            text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .any(|w| w == column)
        };
        mentions(&self.columns_raw) || self.where_raw.as_deref().is_some_and(mentions)
    }
}

struct Cursor<'a> {
    s: &'a str,
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn skip_ws(&mut self) {
        while let Some(c) = self.s[self.pos..].chars().next() {
            if c.is_whitespace() {
                self.pos += c.len_utf8();
            } else {
                break;
            }
        }
    }

    fn rest(&self) -> &'a str {
        &self.s[self.pos..]
    }

    /// Consume `word` case-insensitively if it is the next whole word.
    fn keyword(&mut self, word: &str) -> Option<()> {
        self.skip_ws();
        let rest = self.rest();
        if rest.len() < word.len() || !rest.is_char_boundary(word.len()) {
            return None;
        }
        if !rest[..word.len()].eq_ignore_ascii_case(word) {
            return None;
        }
        let boundary = rest[word.len()..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        if !boundary {
            return None;
        }
        self.pos += word.len();
        Some(())
    }

    /// A bare or quoted identifier (`` `x` ``, `"x"`, `'x'`, `[x]`),
    /// optionally schema-qualified; returns the last segment.
    fn ident(&mut self) -> Option<String> {
        self.skip_ws();
        let mut last = self.single_ident()?;
        while self.rest().starts_with('.') {
            self.pos += 1;
            last = self.single_ident()?;
        }
        Some(last)
    }

    fn single_ident(&mut self) -> Option<String> {
        let rest = self.rest();
        let mut chars = rest.chars();
        let first = chars.next()?;
        let close = match first {
            '`' => Some('`'),
            '"' => Some('"'),
            '\'' => Some('\''),
            '[' => Some(']'),
            _ => None,
        };
        if let Some(close) = close {
            let body = &rest[1..];
            let end = body.find(close)?;
            self.pos += 1 + end + 1;
            return Some(body[..end].to_string());
        }
        let len = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .map(char::len_utf8)
            .sum::<usize>();
        if len == 0 {
            return None;
        }
        self.pos += len;
        Some(rest[..len].to_string())
    }

    /// After an opening `(`: the text up to the matching `)`.
    fn balanced(&mut self) -> Option<String> {
        self.skip_ws();
        if !self.rest().starts_with('(') {
            return None;
        }
        self.pos += 1;
        let start = self.pos;
        let mut depth = 1usize;
        let mut quote: Option<char> = None;
        for (i, c) in self.rest().char_indices() {
            match quote {
                Some(q) => {
                    if c == q {
                        quote = None;
                    }
                }
                None => match c {
                    '\'' | '"' | '`' => quote = Some(c),
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            let body = self.s[start..start + i].to_string();
                            self.pos = start + i + 1;
                            return Some(body);
                        }
                    }
                    _ => {}
                },
            }
        }
        None
    }
}

/// Tolerantly parse `CREATE [UNIQUE] INDEX [IF NOT EXISTS] name ON table
/// (cols) [WHERE expr]` in PocketBase's backtick style or plain SQL.
pub fn parse_index(stmt: &str) -> Option<IndexDef> {
    let s = stmt.trim().trim_end_matches(';').trim();
    let mut cur = Cursor { s, pos: 0 };
    cur.keyword("CREATE")?;
    let unique = cur.keyword("UNIQUE").is_some();
    cur.keyword("INDEX")?;
    if cur.keyword("IF").is_some() {
        cur.keyword("NOT")?;
        cur.keyword("EXISTS")?;
    }
    let name = cur.ident()?;
    cur.keyword("ON")?;
    let table = cur.ident()?;
    let columns_raw = cur.balanced()?.trim().to_string();
    if columns_raw.is_empty() {
        return None;
    }
    cur.skip_ws();
    let where_raw = if cur.rest().is_empty() {
        None
    } else {
        cur.keyword("WHERE")?;
        let expr = cur.rest().trim();
        if expr.is_empty() {
            return None;
        }
        Some(expr.to_string())
    };
    Some(IndexDef {
        unique,
        name,
        table,
        columns_raw,
        where_raw,
    })
}

/// Re-emit `stmt` as `CREATE [UNIQUE] INDEX IF NOT EXISTS "name" ON
/// "table" (...) [WHERE ...]` with backticks rewritten to double quotes
/// and the table forced to `table` (PocketBase does the same: an index
/// statement can only ever target its own collection).
pub fn normalize_index(stmt: &str, table: &str) -> DbResult<String> {
    let def = parse_index(stmt)
        .ok_or_else(|| DbError::InvalidIdentifier(format!("invalid index expression: {stmt}")))?;
    render_index(&def, table)
}

fn render_index(def: &IndexDef, table: &str) -> DbResult<String> {
    let name = ident(&def.name)?;
    let table = ident(table)?;
    let unique = if def.unique { "UNIQUE " } else { "" };
    let mut sql = format!(
        "CREATE {unique}INDEX IF NOT EXISTS {name} ON {table} ({})",
        quote_identifiers(&def.columns_raw)
    );
    if let Some(w) = &def.where_raw {
        sql.push_str(" WHERE ");
        sql.push_str(&quote_identifiers(w));
    }
    Ok(sql)
}

/// Words that may appear bare inside an index column list or predicate
/// and must not be quoted.
const INDEX_KEYWORDS: &[&str] = &[
    "ASC",
    "DESC",
    "COLLATE",
    "NULLS",
    "FIRST",
    "LAST",
    "AND",
    "OR",
    "NOT",
    "IS",
    "NULL",
    "IN",
    "LIKE",
    "ILIKE",
    "GLOB",
    "BETWEEN",
    "TRUE",
    "FALSE",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "CAST",
    "AS",
    "ESCAPE",
    "DISTINCT",
    "EXISTS",
    "CURRENT_TIMESTAMP",
    "CURRENT_DATE",
    "CURRENT_TIME",
];

/// Double-quote every bare identifier in an index expression so
/// camelCase column names survive Postgres' lowercase folding, and turn
/// backtick quoting into double quotes. Keywords, numbers, string
/// literals, function names (a word followed by `(`), and the token
/// after `COLLATE`/`AS` are left alone.
pub fn quote_identifiers(expr: &str) -> String {
    let mut out = String::with_capacity(expr.len() + 8);
    let chars: Vec<char> = expr.chars().collect();
    let mut i = 0;
    let mut skip_next_word = false;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' => {
                // String literal, copied verbatim (with '' escapes).
                let start = i;
                i += 1;
                while i < chars.len() {
                    if chars[i] == '\'' {
                        if chars.get(i + 1) == Some(&'\'') {
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                out.extend(&chars[start..i]);
            }
            '"' | '`' => {
                // Already-quoted identifier: normalize to double quotes.
                i += 1;
                out.push('"');
                while i < chars.len() && chars[i] != c {
                    out.push(chars[i]);
                    i += 1;
                }
                out.push('"');
                i += 1;
                skip_next_word = false;
            }
            _ if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                let upper = word.to_ascii_uppercase();
                let next = chars[i..].iter().find(|ch| !ch.is_whitespace());
                let is_call = next == Some(&'(');
                if skip_next_word || is_call || INDEX_KEYWORDS.contains(&upper.as_str()) {
                    out.push_str(&word);
                    skip_next_word = matches!(upper.as_str(), "COLLATE" | "AS");
                } else {
                    out.push('"');
                    out.push_str(&word);
                    out.push('"');
                    skip_next_word = false;
                }
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

fn parse_indexes(collection: &Collection) -> DbResult<Vec<(IndexDef, String)>> {
    collection
        .indexes
        .iter()
        .map(|stmt| {
            let def = parse_index(stmt).ok_or_else(|| {
                DbError::InvalidIdentifier(format!("invalid index expression: {stmt}"))
            })?;
            let sql = render_index(&def, &collection.name)?;
            Ok((def, sql))
        })
        .collect()
}

// --- sync ---------------------------------------------------------------

/// Bring the physical table/view of `next` in line with its definition.
/// `previous` is the definition currently materialized (`None` on
/// create). Run inside the caller's transaction where possible: DDL is
/// transactional on both SQLite and Postgres.
pub async fn sync(
    ex: &dyn Executor,
    backend: Backend,
    previous: Option<&Collection>,
    next: &Collection,
) -> DbResult<()> {
    let table = ident(&next.name)?;
    for f in &next.fields {
        ident(&f.name)?;
    }

    if next.is_view() {
        if let Some(prev) = previous {
            drop_object(ex, prev).await?;
        }
        ex.execute(&format!("DROP VIEW IF EXISTS {table}"), &[])
            .await?;
        if next.view_query.trim().is_empty() {
            return Err(DbError::InvalidIdentifier(
                "view collection is missing viewQuery".into(),
            ));
        }
        ex.execute(
            &format!("CREATE VIEW {table} AS {}", next.view_query.trim()),
            &[],
        )
        .await?;
        // SQLite only parses a view's SELECT at creation; run it once so
        // a query against a missing table or column fails now, inside
        // the caller's transaction, rather than on the first request.
        ex.query(&format!("SELECT * FROM {table} LIMIT 0"), &[])
            .await?;
        return Ok(());
    }

    let prev = match previous {
        Some(p) if p.is_view() => {
            drop_object(ex, p).await?;
            None
        }
        other => other,
    };

    let next_indexes = parse_indexes(next)?;

    let Some(prev) = prev else {
        ex.execute(&create_table_sql(backend, next)?, &[]).await?;
        for (_, sql) in &next_indexes {
            ex.execute(sql, &[]).await?;
        }
        return Ok(());
    };

    if prev.name != next.name {
        ex.execute(
            &format!("ALTER TABLE {} RENAME TO {table}", ident(&prev.name)?),
            &[],
        )
        .await?;
    }

    // Diff by field id. Primary keys are never touched.
    let mut dropped: Vec<&Field> = Vec::new();
    let mut retyped: Vec<(&Field, &Field)> = Vec::new();
    let mut renamed: Vec<(&Field, &Field)> = Vec::new();
    let mut added: Vec<&Field> = Vec::new();
    for pf in prev.fields.iter().filter(|f| !f.is_primary_key()) {
        match next.fields.iter().find(|f| f.id == pf.id) {
            None => dropped.push(pf),
            Some(nf) if nf.is_primary_key() => {}
            Some(nf) if physical_kind(pf) != physical_kind(nf) => retyped.push((pf, nf)),
            Some(nf) if pf.name != nf.name => renamed.push((pf, nf)),
            Some(_) => {}
        }
    }
    for nf in next.fields.iter().filter(|f| !f.is_primary_key()) {
        if !prev.fields.iter().any(|f| f.id == nf.id) {
            added.push(nf);
        }
    }

    // Indexes that changed, disappeared, or touch a column about to be
    // dropped go first (SQLite refuses to drop an indexed column).
    let prev_indexes = parse_indexes(prev)?;
    let doomed_columns: Vec<&str> = dropped
        .iter()
        .map(|f| f.name.as_str())
        .chain(retyped.iter().map(|(pf, _)| pf.name.as_str()))
        .collect();
    for (def, sql) in &prev_indexes {
        let unchanged = next_indexes
            .iter()
            .any(|(n, nsql)| n.name == def.name && nsql == sql);
        let touches_doomed = doomed_columns.iter().any(|c| def.references_column(c));
        if !unchanged || touches_doomed {
            ex.execute(&format!("DROP INDEX IF EXISTS {}", ident(&def.name)?), &[])
                .await?;
        }
    }

    for f in dropped {
        ex.execute(
            &format!("ALTER TABLE {table} DROP COLUMN {}", ident(&f.name)?),
            &[],
        )
        .await?;
    }
    for (pf, nf) in retyped {
        ex.execute(
            &format!("ALTER TABLE {table} DROP COLUMN {}", ident(&pf.name)?),
            &[],
        )
        .await?;
        ex.execute(&add_column_sql(backend, &table, nf)?, &[])
            .await?;
    }
    for (pf, nf) in renamed {
        ex.execute(
            &format!(
                "ALTER TABLE {table} RENAME COLUMN {} TO {}",
                ident(&pf.name)?,
                ident(&nf.name)?
            ),
            &[],
        )
        .await?;
    }
    for f in added {
        ex.execute(&add_column_sql(backend, &table, f)?, &[])
            .await?;
    }
    for (_, sql) in &next_indexes {
        ex.execute(sql, &[]).await?;
    }
    Ok(())
}

fn add_column_sql(backend: Backend, table: &str, field: &Field) -> DbResult<String> {
    let if_missing = match backend {
        Backend::Postgres => "IF NOT EXISTS ",
        Backend::Sqlite => "",
    };
    Ok(format!(
        "ALTER TABLE {table} ADD COLUMN {if_missing}{}",
        column_ddl(backend, field)?
    ))
}

/// Drop the table or view behind `collection`.
pub async fn drop_object(ex: &dyn Executor, collection: &Collection) -> DbResult<()> {
    let table = ident(&collection.name)?;
    let sql = if collection.is_view() {
        format!("DROP VIEW IF EXISTS {table}")
    } else {
        format!("DROP TABLE IF EXISTS {table}")
    };
    ex.execute(&sql, &[]).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Engine, Sql};
    use crate::sqlite::SqliteEngine;
    use cratebase_core::{CollectionType, FieldKind};

    #[test]
    fn parses_pocketbase_style_indexes() {
        let def = parse_index(
            "CREATE UNIQUE INDEX `idx_email_pbc_1` ON `users` (`email`) WHERE `email` != ''",
        )
        .unwrap();
        assert!(def.unique);
        assert_eq!(def.name, "idx_email_pbc_1");
        assert_eq!(def.table, "users");
        assert_eq!(def.columns_raw, "`email`");
        assert_eq!(def.where_raw.as_deref(), Some("`email` != ''"));
        assert!(def.references_column("email"));
        assert!(!def.references_column("mail"));

        let def = parse_index(
            "create index if not exists idx_x on main.posts (lower(title), \"created\" DESC)",
        )
        .unwrap();
        assert!(!def.unique);
        assert_eq!(def.table, "posts");
        assert_eq!(def.columns_raw, "lower(title), \"created\" DESC");
        assert!(def.where_raw.is_none());
        assert!(def.references_column("title"));

        assert!(parse_index("CREATE TABLE x (a)").is_none());
        assert!(parse_index("CREATE INDEX x ON t").is_none());
        assert!(parse_index("CREATE INDEX x ON t (a) WHERE").is_none());
        assert!(parse_index("CREATE INDEX x ON t (a) garbage").is_none());
    }

    #[test]
    fn normalizes_index_to_the_collection_table() {
        let sql = normalize_index(
            "CREATE UNIQUE INDEX `idx_a` ON `other` (`a`, `b`) WHERE `a` != ''",
            "posts",
        )
        .unwrap();
        assert_eq!(
            sql,
            "CREATE UNIQUE INDEX IF NOT EXISTS \"idx_a\" ON \"posts\" (\"a\", \"b\") WHERE \"a\" != ''"
        );
        assert!(normalize_index("nope", "posts").is_err());
        assert!(normalize_index("CREATE INDEX `bad name` ON t (a)", "posts").is_err());

        // Bare camelCase identifiers get quoted so Postgres keeps their case.
        let sql = normalize_index(
            "CREATE UNIQUE INDEX idx_externalAuths_record_provider ON _externalAuths (collectionRef, recordRef, provider)",
            "_externalAuths",
        )
        .unwrap();
        assert_eq!(
            sql,
            "CREATE UNIQUE INDEX IF NOT EXISTS \"idx_externalAuths_record_provider\" ON \"_externalAuths\" (\"collectionRef\", \"recordRef\", \"provider\")"
        );
        assert_eq!(
            quote_identifiers("lower(title) COLLATE NOCASE DESC, `created` ASC"),
            "lower(\"title\") COLLATE NOCASE DESC, \"created\" ASC"
        );
        assert_eq!(
            quote_identifiers("email != '' AND verified = 1 AND CAST(x AS TEXT) IS NOT NULL"),
            "\"email\" != '' AND \"verified\" = 1 AND CAST(\"x\" AS TEXT) IS NOT NULL"
        );
        assert_eq!(quote_identifiers("name = 'it''s'"), "\"name\" = 'it''s'");
    }

    fn posts() -> Collection {
        let mut c = Collection::new("posts", CollectionType::Base);
        let pos = c.fields.len() - 2;
        c.fields.insert(
            pos,
            Field::new("title", FieldKind::default_for(FieldType::Text)),
        );
        c.fields.insert(
            pos + 1,
            Field::new("views", FieldKind::default_for(FieldType::Number)),
        );
        c.fields
            .insert(pos + 2, Field::new("published", FieldKind::Bool {}));
        c.indexes = vec!["CREATE INDEX `idx_posts_title` ON `posts` (`title`)".into()];
        c
    }

    #[tokio::test]
    async fn create_add_drop_rename_retype_and_indexes() {
        let e = SqliteEngine::open_memory().unwrap();
        let c1 = posts();
        sync(&e, Backend::Sqlite, None, &c1).await.unwrap();
        assert_eq!(
            e.table_columns("posts").await.unwrap(),
            vec!["id", "title", "views", "published", "created", "updated"]
        );
        assert_eq!(
            e.table_indexes("posts").await.unwrap(),
            vec!["idx_posts_title"]
        );
        e.execute(
            "INSERT INTO posts (id, title, views) VALUES ('a', 'hello', 2)",
            &[],
        )
        .await
        .unwrap();
        // Defaults fill the columns we did not mention.
        let row = e.query("SELECT * FROM posts", &[]).await.unwrap().remove(0);
        assert_eq!(row.get("published"), Some(&Sql::Int(0)));
        assert_eq!(row.get_str("created"), Some(""));

        // Rename title → heading (same id), retype views → text, drop
        // published, add tags (multi select), change the index.
        let mut c2 = c1.clone();
        c2.field_mut("title").name = "heading".into();
        c2.field_mut("views").kind = FieldKind::default_for(FieldType::Text);
        c2.fields.retain(|f| f.name != "published");
        let mut tags = Field::new(
            "tags",
            FieldKind::Select {
                values: vec!["a".into()],
                max_select: 3,
            },
        );
        tags.id = "select_tags".into();
        c2.fields.insert(c2.fields.len() - 2, tags);
        c2.indexes = vec![
            "CREATE UNIQUE INDEX `idx_posts_heading` ON `posts` (`heading`) WHERE `heading` != ''"
                .into(),
        ];
        sync(&e, Backend::Sqlite, Some(&c1), &c2).await.unwrap();
        assert_eq!(
            e.table_columns("posts").await.unwrap(),
            vec!["id", "heading", "created", "updated", "views", "tags"]
        );
        assert_eq!(
            e.table_indexes("posts").await.unwrap(),
            vec!["idx_posts_heading"]
        );
        let row = e.query("SELECT * FROM posts", &[]).await.unwrap().remove(0);
        assert_eq!(row.get_str("heading"), Some("hello"));
        assert_eq!(row.get_str("views"), Some(""));
        assert!(row.get("tags").unwrap().is_null());
        // The partial unique index is live.
        e.execute("INSERT INTO posts (id, heading) VALUES ('b', '')", &[])
            .await
            .unwrap();
        e.execute("INSERT INTO posts (id, heading) VALUES ('c', '')", &[])
            .await
            .unwrap();
        let err = e
            .execute("INSERT INTO posts (id, heading) VALUES ('d', 'hello')", &[])
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::UniqueViolation(_)));

        // Collection rename moves the table.
        let mut c3 = c2.clone();
        c3.name = "articles".into();
        sync(&e, Backend::Sqlite, Some(&c2), &c3).await.unwrap();
        assert!(e.table_exists("articles").await.unwrap());
        assert!(!e.table_exists("posts").await.unwrap());

        // Views.
        let mut v = Collection::new("titles", CollectionType::View);
        v.view_query = "SELECT id, heading FROM articles".into();
        sync(&e, Backend::Sqlite, None, &v).await.unwrap();
        assert_eq!(
            e.table_columns("titles").await.unwrap(),
            vec!["id", "heading"]
        );
        let mut v2 = v.clone();
        v2.view_query = "SELECT id FROM articles".into();
        sync(&e, Backend::Sqlite, Some(&v), &v2).await.unwrap();
        assert_eq!(e.table_columns("titles").await.unwrap(), vec!["id"]);
        drop_object(&e, &v2).await.unwrap();
        assert!(!e.table_exists("titles").await.unwrap());
        drop_object(&e, &c3).await.unwrap();
        assert!(!e.table_exists("articles").await.unwrap());
    }

    #[tokio::test]
    async fn auth_collection_gets_its_unique_indexes() {
        let e = SqliteEngine::open_memory().unwrap();
        let users = Collection::default_users();
        sync(&e, Backend::Sqlite, None, &users).await.unwrap();
        let idx = e.table_indexes("users").await.unwrap();
        assert_eq!(idx.len(), 2);
        e.execute(
            "INSERT INTO users (id, email, tokenKey) VALUES ('a', 'x@y', 't1')",
            &[],
        )
        .await
        .unwrap();
        let err = e
            .execute(
                "INSERT INTO users (id, email, tokenKey) VALUES ('b', 'x@y', 't2')",
                &[],
            )
            .await
            .unwrap_err();
        match err {
            DbError::UniqueViolation(d) => assert_eq!(d, "users.email"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rejects_invalid_identifiers() {
        let mut c = posts();
        c.name = "bad name".into();
        assert!(create_table_sql(Backend::Sqlite, &c).is_err());
        assert_eq!(
            physical_type(Backend::Postgres, c.field("views").unwrap()),
            "DOUBLE PRECISION"
        );
        assert_eq!(
            physical_type(Backend::Sqlite, c.field("views").unwrap()),
            "REAL"
        );
        assert_eq!(
            physical_type(Backend::Sqlite, c.field("published").unwrap()),
            "INTEGER"
        );
    }

    trait FieldMut {
        fn field_mut(&mut self, name: &str) -> &mut Field;
    }
    impl FieldMut for Collection {
        fn field_mut(&mut self, name: &str) -> &mut Field {
            self.fields.iter_mut().find(|f| f.name == name).unwrap()
        }
    }
}
