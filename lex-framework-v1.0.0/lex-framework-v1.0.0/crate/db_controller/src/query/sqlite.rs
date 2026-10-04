//! SQLite translation and execution; unsupported features fail explicitly.

use super::*;

#[derive(Clone)]
pub struct SqlitePoolExecutor {
    pool: SqlitePool,
}

impl SqlitePoolExecutor {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

pub struct SqliteTransactionExecutor<'a, 'tx> {
    transaction: &'a mut Transaction<'tx, Sqlite>,
}

impl<'a, 'tx> SqliteTransactionExecutor<'a, 'tx> {
    pub fn new(transaction: &'a mut Transaction<'tx, Sqlite>) -> Self {
        Self { transaction }
    }
}

pub(super) fn sqlite_protocol(message: impl Into<String>) -> SqlxError {
    SqlxError::Protocol(message.into())
}

/// Translate the portable SQL emitted by QuerySet into SQLite syntax. PostgreSQL-only
/// operations are rejected explicitly instead of being silently approximated.
pub(super) fn sqlite_translate_sql(mut sql: String) -> Result<String, SqlxError> {
    const UNSUPPORTED: &[(&str, &str)] = &[
        ("jsonb_populate_record", "bulk JSON record expansion"),
        ("jsonb_populate_recordset", "bulk JSON recordset expansion"),
        (" @> ", "PostgreSQL JSON containment"),
        (" ? $", "PostgreSQL JSON key operator"),
        ("EXTRACT(QUARTER", "EXTRACT(QUARTER)"),
        ("EXTRACT(WEEK", "EXTRACT(WEEK)"),
        ("FOR UPDATE", "row-level FOR UPDATE locking"),
        ("FOR NO KEY UPDATE", "row-level NO KEY UPDATE locking"),
        ("FOR SHARE", "row-level SHARE locking"),
        ("FOR KEY SHARE", "row-level KEY SHARE locking"),
        ("SKIP LOCKED", "SKIP LOCKED"),
        ("NOWAIT", "NOWAIT row locking"),
        ("ARRAY_AGG(", "ARRAY_AGG"),
        ("JSONB_AGG(", "JSONB_AGG"),
        ("BOOL_AND(", "BOOL_AND"),
        ("BOOL_OR(", "BOOL_OR"),
        ("STDDEV_SAMP(", "STDDEV_SAMP"),
        ("VAR_SAMP(", "VAR_SAMP"),
        ("EXPLAIN (", "PostgreSQL JSON EXPLAIN options"),
    ];
    let upper = sql.to_ascii_uppercase();
    for (needle, feature) in UNSUPPORTED {
        if upper.contains(&needle.to_ascii_uppercase()) {
            return Err(sqlite_protocol(format!(
                "{feature} is not portable to SQLite through this API; use a SQLite-compatible operation"
            )));
        }
    }

    // SQLite accepts numbered parameters as ?NNN. Preserve the parameter number so generated
    // SQL does not need to reorder DbValue bindings.
    let placeholder = regex::Regex::new(
        r"\$([1-9][0-9]*)(?:::(?:DOUBLE PRECISION|TIMESTAMP WITH TIME ZONE|[A-Za-z0-9_]+))?",
    )
    .map_err(|error| sqlite_protocol(error.to_string()))?;
    sql = placeholder.replace_all(&sql, "?$1").into_owned();

    // Portable equivalents for common PostgreSQL spellings emitted by QuerySet.
    sql = sql.replace(" ILIKE ", " LIKE ");
    sql = sql.replace(" ESCAPE E'\\\\'", " ESCAPE '\\'");
    sql = sql.replace("now()", "CURRENT_TIMESTAMP");
    sql = sql.replace("NOW()", "CURRENT_TIMESTAMP");
    sql = sql.replace("jsonb_build_object", "json_object");
    sql = sql.replace("JSONB_BUILD_OBJECT", "json_object");
    sql = sql.replace("jsonb_build_array", "json_array");
    sql = sql.replace("JSONB_BUILD_ARRAY", "json_array");
    sql = sql.replace("::jsonb", "");
    sql = sql.replace("::json", "");
    sql = sql.replace(" LIMIT ALL", " LIMIT -1");

    // EXTRACT(part FROM alias."column") is emitted only for validated date fields.
    let extract = regex::Regex::new(
        r#"(?i)EXTRACT\((YEAR|MONTH|DAY|HOUR|MINUTE|SECOND) FROM ([A-Za-z0-9_]+\."[A-Za-z0-9_]+")\)"#,
    ).map_err(|error| sqlite_protocol(error.to_string()))?;
    sql = extract
        .replace_all(&sql, |caps: &regex::Captures<'_>| {
            let fmt = match caps[1].to_ascii_uppercase().as_str() {
                "YEAR" => "%Y",
                "MONTH" => "%m",
                "DAY" => "%d",
                "HOUR" => "%H",
                "MINUTE" => "%M",
                "SECOND" => "%S",
                _ => "%Y",
            };
            format!("CAST(strftime('{fmt}', {}) AS INTEGER)", &caps[2])
        })
        .into_owned();
    Ok(sql)
}

macro_rules! bind_sqlite_value {
    ($query:expr, $value:expr) => {{
        match $value {
            DbValue::Bool(value) => $query.bind(value),
            DbValue::I16(value) => $query.bind(value),
            DbValue::I32(value) => $query.bind(value),
            DbValue::I64(value) => $query.bind(value),
            DbValue::F32(value) => $query.bind(value),
            DbValue::F64(value) => $query.bind(value),
            // SQLite has dynamic typing. Text is used for exact representations that should not
            // lose precision or formatting when round-tripped through an SQLite column.
            DbValue::Decimal(value) => $query.bind(value.to_string()),
            DbValue::Uuid(value) => $query.bind(value.to_string()),
            DbValue::DateTime(value) => $query.bind(value.to_rfc3339()),
            DbValue::String(value) => $query.bind(value),
            DbValue::Json(value) => $query.bind(value.to_string()),
            DbValue::Bytes(value) => $query.bind(value),
        }
    }};
}

#[async_trait]
impl QueryExecutor for SqlitePoolExecutor {
    type Row = SqliteRow;

    fn is_transactional(&self) -> bool {
        false
    }

    fn dialect(&self) -> DatabaseDialect {
        DatabaseDialect::Sqlite
    }

    async fn fetch_all<T>(&mut self, sql: String, params: Vec<DbValue>) -> Result<Vec<T>, SqlxError>
    where
        T: for<'r> FromRow<'r, SqliteRow> + Send + Unpin,
    {
        let sql = sqlite_translate_sql(sql)?;
        let mut query = sqlx::query_as::<_, T>(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_sqlite_value!(query, value);
        }
        query.fetch_all(&self.pool).await
    }

    async fn fetch_rows(
        &mut self,
        sql: String,
        params: Vec<DbValue>,
    ) -> Result<Vec<SqliteRow>, SqlxError> {
        let sql = sqlite_translate_sql(sql)?;
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_sqlite_value!(query, value);
        }
        query.fetch_all(&self.pool).await
    }

    async fn fetch_i64(&mut self, sql: String, params: Vec<DbValue>) -> Result<i64, SqlxError> {
        let sql = sqlite_translate_sql(sql)?;
        let mut query = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_sqlite_value!(query, value);
        }
        query.fetch_one(&self.pool).await
    }

    async fn fetch_bool(&mut self, sql: String, params: Vec<DbValue>) -> Result<bool, SqlxError> {
        let sql = sqlite_translate_sql(sql)?;
        let mut query = sqlx::query_scalar::<_, bool>(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_sqlite_value!(query, value);
        }
        query.fetch_one(&self.pool).await
    }

    async fn fetch_json(&mut self, sql: String, params: Vec<DbValue>) -> Result<Value, SqlxError> {
        let sql = sqlite_translate_sql(sql)?;
        let mut query = sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_sqlite_value!(query, value);
        }
        let raw = query.fetch_one(&self.pool).await?;
        serde_json::from_str(&raw).map_err(|error| sqlite_protocol(error.to_string()))
    }

    async fn fetch_json_all(
        &mut self,
        sql: String,
        params: Vec<DbValue>,
    ) -> Result<Vec<Value>, SqlxError> {
        let sql = sqlite_translate_sql(sql)?;
        let mut query = sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_sqlite_value!(query, value);
        }
        let raw = query.fetch_all(&self.pool).await?;
        raw.into_iter()
            .map(|value| {
                serde_json::from_str(&value).map_err(|error| sqlite_protocol(error.to_string()))
            })
            .collect()
    }

    async fn execute(&mut self, sql: String, params: Vec<DbValue>) -> Result<u64, SqlxError> {
        let sql = sqlite_translate_sql(sql)?;
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_sqlite_value!(query, value);
        }
        query
            .execute(&self.pool)
            .await
            .map(|result| result.rows_affected())
    }
}

#[async_trait]
impl<'a, 'tx> QueryExecutor for SqliteTransactionExecutor<'a, 'tx> {
    type Row = SqliteRow;

    fn is_transactional(&self) -> bool {
        true
    }

    fn dialect(&self) -> DatabaseDialect {
        DatabaseDialect::Sqlite
    }

    async fn fetch_all<T>(&mut self, sql: String, params: Vec<DbValue>) -> Result<Vec<T>, SqlxError>
    where
        T: for<'r> FromRow<'r, SqliteRow> + Send + Unpin,
    {
        let sql = sqlite_translate_sql(sql)?;
        let mut query = sqlx::query_as::<_, T>(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_sqlite_value!(query, value);
        }
        query.fetch_all(&mut **self.transaction).await
    }

    async fn fetch_rows(
        &mut self,
        sql: String,
        params: Vec<DbValue>,
    ) -> Result<Vec<SqliteRow>, SqlxError> {
        let sql = sqlite_translate_sql(sql)?;
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_sqlite_value!(query, value);
        }
        query.fetch_all(&mut **self.transaction).await
    }

    async fn fetch_i64(&mut self, sql: String, params: Vec<DbValue>) -> Result<i64, SqlxError> {
        let sql = sqlite_translate_sql(sql)?;
        let mut query = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_sqlite_value!(query, value);
        }
        query.fetch_one(&mut **self.transaction).await
    }

    async fn fetch_bool(&mut self, sql: String, params: Vec<DbValue>) -> Result<bool, SqlxError> {
        let sql = sqlite_translate_sql(sql)?;
        let mut query = sqlx::query_scalar::<_, bool>(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_sqlite_value!(query, value);
        }
        query.fetch_one(&mut **self.transaction).await
    }

    async fn fetch_json(&mut self, sql: String, params: Vec<DbValue>) -> Result<Value, SqlxError> {
        let sql = sqlite_translate_sql(sql)?;
        let mut query = sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_sqlite_value!(query, value);
        }
        let raw = query.fetch_one(&mut **self.transaction).await?;
        serde_json::from_str(&raw).map_err(|error| sqlite_protocol(error.to_string()))
    }

    async fn fetch_json_all(
        &mut self,
        sql: String,
        params: Vec<DbValue>,
    ) -> Result<Vec<Value>, SqlxError> {
        let sql = sqlite_translate_sql(sql)?;
        let mut query = sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_sqlite_value!(query, value);
        }
        let raw = query.fetch_all(&mut **self.transaction).await?;
        raw.into_iter()
            .map(|value| {
                serde_json::from_str(&value).map_err(|error| sqlite_protocol(error.to_string()))
            })
            .collect()
    }

    async fn execute(&mut self, sql: String, params: Vec<DbValue>) -> Result<u64, SqlxError> {
        let sql = sqlite_translate_sql(sql)?;
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_sqlite_value!(query, value);
        }
        query
            .execute(&mut **self.transaction)
            .await
            .map(|result| result.rows_affected())
    }
}
