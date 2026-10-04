//! PostgreSQL pooled and transaction executors.

use super::*;

#[derive(Clone)]
pub struct PoolExecutor {
    pool: PgPool,
}

impl PoolExecutor {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

pub struct TransactionExecutor<'a, 'tx> {
    transaction: &'a mut Transaction<'tx, Postgres>,
}

impl<'a, 'tx> TransactionExecutor<'a, 'tx> {
    pub fn new(transaction: &'a mut Transaction<'tx, Postgres>) -> Self {
        Self { transaction }
    }
}

#[async_trait]
impl QueryExecutor for PoolExecutor {
    type Row = PgRow;

    fn is_transactional(&self) -> bool {
        false
    }

    fn dialect(&self) -> DatabaseDialect {
        DatabaseDialect::Postgres
    }

    async fn fetch_all<T>(&mut self, sql: String, params: Vec<DbValue>) -> Result<Vec<T>, SqlxError>
    where
        T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
    {
        let mut query = sqlx::query_as::<_, T>(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_value!(query, value);
        }
        query.fetch_all(&self.pool).await
    }

    async fn fetch_rows(
        &mut self,
        sql: String,
        params: Vec<DbValue>,
    ) -> Result<Vec<PgRow>, SqlxError> {
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_value!(query, value);
        }
        query.fetch_all(&self.pool).await
    }

    async fn fetch_i64(&mut self, sql: String, params: Vec<DbValue>) -> Result<i64, SqlxError> {
        let mut query = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_value!(query, value);
        }
        query.fetch_one(&self.pool).await
    }

    async fn fetch_bool(&mut self, sql: String, params: Vec<DbValue>) -> Result<bool, SqlxError> {
        let mut query = sqlx::query_scalar::<_, bool>(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_value!(query, value);
        }
        query.fetch_one(&self.pool).await
    }

    async fn fetch_json(&mut self, sql: String, params: Vec<DbValue>) -> Result<Value, SqlxError> {
        let mut query = sqlx::query_scalar::<_, Json<Value>>(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_value!(query, value);
        }
        query.fetch_one(&self.pool).await.map(|value| value.0)
    }

    async fn fetch_json_all(
        &mut self,
        sql: String,
        params: Vec<DbValue>,
    ) -> Result<Vec<Value>, SqlxError> {
        let mut query = sqlx::query_scalar::<_, Json<Value>>(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_value!(query, value);
        }
        query
            .fetch_all(&self.pool)
            .await
            .map(|values| values.into_iter().map(|value| value.0).collect())
    }

    async fn execute(&mut self, sql: String, params: Vec<DbValue>) -> Result<u64, SqlxError> {
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_value!(query, value);
        }
        query
            .execute(&self.pool)
            .await
            .map(|result| result.rows_affected())
    }
}

#[async_trait]
impl<'a, 'tx> QueryExecutor for TransactionExecutor<'a, 'tx> {
    type Row = PgRow;

    fn is_transactional(&self) -> bool {
        true
    }

    fn dialect(&self) -> DatabaseDialect {
        DatabaseDialect::Postgres
    }

    async fn fetch_all<T>(&mut self, sql: String, params: Vec<DbValue>) -> Result<Vec<T>, SqlxError>
    where
        T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
    {
        let mut query = sqlx::query_as::<_, T>(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_value!(query, value);
        }
        query.fetch_all(&mut **self.transaction).await
    }

    async fn fetch_rows(
        &mut self,
        sql: String,
        params: Vec<DbValue>,
    ) -> Result<Vec<PgRow>, SqlxError> {
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_value!(query, value);
        }
        query.fetch_all(&mut **self.transaction).await
    }

    async fn fetch_i64(&mut self, sql: String, params: Vec<DbValue>) -> Result<i64, SqlxError> {
        let mut query = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_value!(query, value);
        }
        query.fetch_one(&mut **self.transaction).await
    }

    async fn fetch_bool(&mut self, sql: String, params: Vec<DbValue>) -> Result<bool, SqlxError> {
        let mut query = sqlx::query_scalar::<_, bool>(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_value!(query, value);
        }
        query.fetch_one(&mut **self.transaction).await
    }

    async fn fetch_json(&mut self, sql: String, params: Vec<DbValue>) -> Result<Value, SqlxError> {
        let mut query = sqlx::query_scalar::<_, Json<Value>>(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_value!(query, value);
        }
        query
            .fetch_one(&mut **self.transaction)
            .await
            .map(|value| value.0)
    }

    async fn fetch_json_all(
        &mut self,
        sql: String,
        params: Vec<DbValue>,
    ) -> Result<Vec<Value>, SqlxError> {
        let mut query = sqlx::query_scalar::<_, Json<Value>>(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_value!(query, value);
        }
        query
            .fetch_all(&mut **self.transaction)
            .await
            .map(|values| values.into_iter().map(|value| value.0).collect())
    }

    async fn execute(&mut self, sql: String, params: Vec<DbValue>) -> Result<u64, SqlxError> {
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
        for value in params {
            query = bind_value!(query, value);
        }
        query
            .execute(&mut **self.transaction)
            .await
            .map(|result| result.rows_affected())
    }
}
