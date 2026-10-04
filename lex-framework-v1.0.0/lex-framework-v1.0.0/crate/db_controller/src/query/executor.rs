//! Database dialect and execution contracts.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseDialect {
    Postgres,
    Sqlite,
}

impl DatabaseDialect {
    pub const fn is_postgres(self) -> bool {
        matches!(self, Self::Postgres)
    }

    pub const fn is_sqlite(self) -> bool {
        matches!(self, Self::Sqlite)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryPreview {
    pub sql: String,
    pub parameter_count: usize,
}

#[async_trait]
pub trait QueryExecutor: Send {
    type Row: Row + Send + Unpin;

    fn is_transactional(&self) -> bool;
    fn dialect(&self) -> DatabaseDialect;

    async fn fetch_all<T>(
        &mut self,
        sql: String,
        params: Vec<DbValue>,
    ) -> Result<Vec<T>, SqlxError>
    where
        T: for<'r> FromRow<'r, Self::Row> + Send + Unpin;

    async fn fetch_rows(
        &mut self,
        sql: String,
        params: Vec<DbValue>,
    ) -> Result<Vec<Self::Row>, SqlxError>;
    async fn fetch_i64(&mut self, sql: String, params: Vec<DbValue>) -> Result<i64, SqlxError>;
    async fn fetch_bool(&mut self, sql: String, params: Vec<DbValue>) -> Result<bool, SqlxError>;
    async fn fetch_json(&mut self, sql: String, params: Vec<DbValue>) -> Result<Value, SqlxError>;
    async fn fetch_json_all(
        &mut self,
        sql: String,
        params: Vec<DbValue>,
    ) -> Result<Vec<Value>, SqlxError>;
    async fn execute(&mut self, sql: String, params: Vec<DbValue>) -> Result<u64, SqlxError>;
}

// Borrowed executors retain the caller's transaction and connection state across successive
// terminal operations; they never create another connection behind the caller's back.
#[async_trait]
impl<E: QueryExecutor + ?Sized> QueryExecutor for &mut E {
    type Row = E::Row;
    fn is_transactional(&self) -> bool {
        (**self).is_transactional()
    }
    fn dialect(&self) -> DatabaseDialect {
        (**self).dialect()
    }
    async fn fetch_all<T>(&mut self, sql: String, params: Vec<DbValue>) -> Result<Vec<T>, SqlxError>
    where
        T: for<'r> FromRow<'r, Self::Row> + Send + Unpin,
    {
        (**self).fetch_all(sql, params).await
    }
    async fn fetch_rows(
        &mut self,
        sql: String,
        params: Vec<DbValue>,
    ) -> Result<Vec<Self::Row>, SqlxError> {
        (**self).fetch_rows(sql, params).await
    }
    async fn fetch_i64(&mut self, sql: String, params: Vec<DbValue>) -> Result<i64, SqlxError> {
        (**self).fetch_i64(sql, params).await
    }
    async fn fetch_bool(&mut self, sql: String, params: Vec<DbValue>) -> Result<bool, SqlxError> {
        (**self).fetch_bool(sql, params).await
    }
    async fn fetch_json(&mut self, sql: String, params: Vec<DbValue>) -> Result<Value, SqlxError> {
        (**self).fetch_json(sql, params).await
    }
    async fn fetch_json_all(
        &mut self,
        sql: String,
        params: Vec<DbValue>,
    ) -> Result<Vec<Value>, SqlxError> {
        (**self).fetch_json_all(sql, params).await
    }
    async fn execute(&mut self, sql: String, params: Vec<DbValue>) -> Result<u64, SqlxError> {
        (**self).execute(sql, params).await
    }
}
