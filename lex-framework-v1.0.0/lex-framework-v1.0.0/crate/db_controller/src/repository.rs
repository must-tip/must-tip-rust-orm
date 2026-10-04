use async_trait::async_trait;
use serde_json::{Map, Value};
use sqlx::{
    Error, FromRow, Postgres, Transaction,
    postgres::{PgPool, PgRow},
    types::Json,
};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    marker::PhantomData,
    pin::Pin,
};

use crate::{Entity, PoolExecutor, QuerySet, quote_identifier};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pagination {
    pub limit: u32,
    pub offset: u32,
}

pub type TxFuture<'a, R> = Pin<Box<dyn Future<Output = Result<R, Error>> + Send + 'a>>;

fn protocol(message: impl Into<String>) -> Error {
    Error::Protocol(message.into())
}

fn ident(value: &str) -> Result<String, Error> {
    quote_identifier(value).ok_or_else(|| protocol(format!("unsafe SQL identifier: {value}")))
}

#[async_trait]
pub trait Repository<T>
where
    T: Entity + for<'r> FromRow<'r, PgRow> + Send + Sync + Unpin,
{
    async fn create(&self, input: &T) -> Result<T, Error>;
    async fn get(&self, id: &str) -> Result<Option<T>, Error>;
    async fn update(&self, id: &str, input: &T) -> Result<T, Error>;
    async fn patch(&self, id: &str, input: &Value) -> Result<T, Error>;
    async fn delete(&self, id: &str) -> Result<(), Error>;
    async fn soft_delete(&self, id: &str) -> Result<(), Error>;
    async fn restore(&self, id: &str) -> Result<(), Error>;
    async fn list(
        &self,
        pagination: Option<Pagination>,
        order_by: Option<&str>,
    ) -> Result<Vec<T>, Error>;
    async fn exists(&self, id: &str) -> Result<bool, Error>;
    async fn count(&self, filter: Option<&Value>) -> Result<i64, Error>;
    async fn transaction<F, R>(&self, f: F) -> Result<R, Error>
    where
        F: for<'c> FnOnce(&'c mut Transaction<'_, Postgres>) -> TxFuture<'c, R> + Send,
        R: Send;
}

pub struct ModelManager<T>
where
    T: Entity + for<'r> FromRow<'r, PgRow> + Send + Sync + Unpin,
{
    pool: PgPool,
    _phantom: PhantomData<T>,
}

impl<T> ModelManager<T>
where
    T: Entity + for<'r> FromRow<'r, PgRow> + Send + Sync + Unpin,
{
    pub fn new(pool: PgPool) -> Self {
        T::register();
        Self {
            pool,
            _phantom: PhantomData,
        }
    }

    pub fn objects(pool: PgPool) -> Self {
        Self::new(pool)
    }

    /// Preferred composable query API. Tenant-aware models require `for_tenant(...)` or an
    /// explicitly authorized `global_scope(&authorization)` before execution.
    pub fn queryset(&self) -> QuerySet<T, PoolExecutor> {
        QuerySet::from_executor(PoolExecutor::new(self.pool.clone()))
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    fn legacy_scope_guard() -> Result<(), Error> {
        if T::tenant_column().is_some() {
            return Err(protocol(format!(
                "tenant-aware model {} requires queryset().for_tenant(...) or an explicitly authorized global_scope(&authorization)",
                T::table_name()
            )));
        }
        Ok(())
    }

    fn schema() -> HashMap<String, crate::ColumnSpec> {
        T::schema()
    }

    fn schema_names() -> HashSet<String> {
        Self::schema().into_keys().collect()
    }

    fn select_columns() -> Result<String, Error> {
        let mut columns = Self::schema().into_keys().collect::<Vec<_>>();
        columns.sort();
        columns
            .iter()
            .map(|column| ident(column))
            .collect::<Result<Vec<_>, _>>()
            .map(|values| values.join(", "))
    }

    fn soft_delete_column() -> Result<Option<String>, Error> {
        if Self::schema_names().contains("deleted_at") {
            ident("deleted_at").map(Some)
        } else {
            Ok(None)
        }
    }

    fn live_predicate() -> Result<String, Error> {
        Ok(match Self::soft_delete_column()? {
            Some(column) => format!("{column} IS NULL"),
            None => "TRUE".to_owned(),
        })
    }

    fn persistable_map(
        input: HashMap<String, Value>,
        include_primary_key: bool,
    ) -> Result<Map<String, Value>, Error> {
        let schema = Self::schema();
        let primary_key = T::primary_key();
        let mut output = Map::new();
        for (key, value) in input {
            let Some(spec) = schema.get(&key) else {
                return Err(protocol(format!("model emitted unknown field: {key}")));
            };
            if !include_primary_key && key == primary_key {
                continue;
            }
            if value.is_null() && !spec.nullable {
                return Err(protocol(format!("field {key} cannot be null")));
            }
            output.insert(key, value);
        }
        Ok(output)
    }

    fn patch_map(input: &Value) -> Result<Map<String, Value>, Error> {
        let object = input
            .as_object()
            .ok_or_else(|| protocol("patch input must be a JSON object"))?;
        let schema = Self::schema();
        let mut map = Map::new();
        for (key, value) in object {
            if key == T::primary_key() || key == "deleted_at" {
                return Err(protocol(format!("field {key} cannot be patched")));
            }
            let spec = schema
                .get(key)
                .ok_or_else(|| protocol(format!("unknown patch field: {key}")))?;
            if value.is_null() && !spec.nullable {
                return Err(protocol(format!("field {key} cannot be null")));
            }
            map.insert(key.clone(), value.clone());
        }
        if map.is_empty() {
            return Err(protocol("patch has no mutable fields"));
        }
        Ok(map)
    }

    pub async fn save(&self, model: &T) -> Result<T, Error> {
        let map = model.to_map();
        if let Some(value) = map.get(T::primary_key()) {
            let id = match value {
                Value::String(value) => Some(value.clone()),
                Value::Number(value) => Some(value.to_string()),
                _ => None,
            };
            if let Some(id) = id
                && self.exists(&id).await?
            {
                return self.update(&id, model).await;
            }
        }
        self.create(model).await
    }
}

#[async_trait]
impl<T> Repository<T> for ModelManager<T>
where
    T: Entity + for<'r> FromRow<'r, PgRow> + Send + Sync + Unpin,
{
    async fn create(&self, input: &T) -> Result<T, Error> {
        Self::legacy_scope_guard()?;
        let map = Self::persistable_map(input.to_map(), true)?;
        if map.is_empty() {
            return Err(protocol("create has no persistable fields"));
        }
        let mut columns = map.keys().cloned().collect::<Vec<_>>();
        columns.sort();
        let columns_sql = columns
            .iter()
            .map(|column| ident(column))
            .collect::<Result<Vec<_>, _>>()?
            .join(", ");
        let table = ident(T::table_name())?;
        let returning = Self::select_columns()?;
        let sql = format!(
            "INSERT INTO {table} ({columns_sql}) SELECT {columns_sql} \
             FROM jsonb_populate_record(NULL::{table}, $1::jsonb) RETURNING {returning}"
        );
        sqlx::query_as::<_, T>(sqlx::AssertSqlSafe(sql))
            .bind(Json(Value::Object(map)))
            .fetch_one(&self.pool)
            .await
    }

    async fn get(&self, id: &str) -> Result<Option<T>, Error> {
        Self::legacy_scope_guard()?;
        let table = ident(T::table_name())?;
        let primary_key = ident(T::primary_key())?;
        let sql = format!(
            "SELECT {} FROM {table} WHERE {primary_key}::text = $1 AND {}",
            Self::select_columns()?,
            Self::live_predicate()?
        );
        sqlx::query_as::<_, T>(sqlx::AssertSqlSafe(sql))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    async fn update(&self, id: &str, input: &T) -> Result<T, Error> {
        Self::legacy_scope_guard()?;
        let mut map = Self::persistable_map(input.to_map(), false)?;
        map.remove("deleted_at");
        if map.is_empty() {
            return Err(protocol("update has no mutable fields"));
        }
        update_from_map::<T>(&self.pool, id, map).await
    }

    async fn patch(&self, id: &str, input: &Value) -> Result<T, Error> {
        Self::legacy_scope_guard()?;
        let map = Self::patch_map(input)?;
        update_from_map::<T>(&self.pool, id, map).await
    }

    async fn delete(&self, id: &str) -> Result<(), Error> {
        Self::legacy_scope_guard()?;
        let sql = format!(
            "DELETE FROM {} WHERE {}::text = $1",
            ident(T::table_name())?,
            ident(T::primary_key())?
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn soft_delete(&self, id: &str) -> Result<(), Error> {
        Self::legacy_scope_guard()?;
        let deleted_at = Self::soft_delete_column()?
            .ok_or_else(|| protocol("model does not declare a deleted_at field"))?;
        let sql = format!(
            "UPDATE {} SET {deleted_at} = now(), \"updated_at\" = now() \
             WHERE {}::text = $1 AND {deleted_at} IS NULL",
            ident(T::table_name())?,
            ident(T::primary_key())?
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn restore(&self, id: &str) -> Result<(), Error> {
        Self::legacy_scope_guard()?;
        let deleted_at = Self::soft_delete_column()?
            .ok_or_else(|| protocol("model does not declare a deleted_at field"))?;
        let sql = format!(
            "UPDATE {} SET {deleted_at} = NULL, \"updated_at\" = now() \
             WHERE {}::text = $1",
            ident(T::table_name())?,
            ident(T::primary_key())?
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list(
        &self,
        pagination: Option<Pagination>,
        order_by: Option<&str>,
    ) -> Result<Vec<T>, Error> {
        Self::legacy_scope_guard()?;
        let mut sql = format!(
            "SELECT {} FROM {} WHERE {}",
            Self::select_columns()?,
            ident(T::table_name())?,
            Self::live_predicate()?
        );
        if let Some(order) = order_by {
            let (descending, field) = order
                .strip_prefix('-')
                .map_or((false, order), |value| (true, value));
            if !Self::schema_names().contains(field) {
                return Err(protocol("order_by field is not in model schema"));
            }
            sql.push_str(&format!(
                " ORDER BY {} {}",
                ident(field)?,
                if descending { "DESC" } else { "ASC" }
            ));
        }
        if let Some(page) = pagination {
            if page.limit == 0 || page.limit > 10_000 {
                return Err(protocol("pagination limit must be 1..=10000"));
            }
            sql.push_str(" LIMIT $1 OFFSET $2");
            sqlx::query_as::<_, T>(sqlx::AssertSqlSafe(sql))
                .bind(i64::from(page.limit))
                .bind(i64::from(page.offset))
                .fetch_all(&self.pool)
                .await
        } else {
            sqlx::query_as::<_, T>(sqlx::AssertSqlSafe(sql))
                .fetch_all(&self.pool)
                .await
        }
    }

    async fn exists(&self, id: &str) -> Result<bool, Error> {
        Self::legacy_scope_guard()?;
        let sql = format!(
            "SELECT EXISTS(SELECT 1 FROM {} WHERE {}::text = $1 AND {})",
            ident(T::table_name())?,
            ident(T::primary_key())?,
            Self::live_predicate()?
        );
        sqlx::query_scalar::<_, bool>(sqlx::AssertSqlSafe(sql))
            .bind(id)
            .fetch_one(&self.pool)
            .await
    }

    async fn count(&self, filter: Option<&Value>) -> Result<i64, Error> {
        Self::legacy_scope_guard()?;
        let mut sql = format!(
            "SELECT COUNT(*) FROM {} WHERE {}",
            ident(T::table_name())?,
            Self::live_predicate()?
        );
        let mut values = Vec::new();
        if let Some(value) = filter {
            let object = value
                .as_object()
                .ok_or_else(|| protocol("count filter must be a JSON object"))?;
            let allowed = Self::schema_names();
            for (key, value) in object {
                if !allowed.contains(key) {
                    return Err(protocol(format!("unknown filter field: {key}")));
                }
                let column = ident(key)?;
                if value.is_null() {
                    sql.push_str(&format!(" AND {column} IS NULL"));
                } else {
                    values.push(value.clone());
                    sql.push_str(&format!(
                        " AND to_jsonb({column}) = ${}::jsonb",
                        values.len()
                    ));
                }
            }
        }
        let mut query = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql));
        for value in values {
            query = query.bind(Json(value));
        }
        query.fetch_one(&self.pool).await
    }

    async fn transaction<F, R>(&self, f: F) -> Result<R, Error>
    where
        F: for<'c> FnOnce(&'c mut Transaction<'_, Postgres>) -> TxFuture<'c, R> + Send,
        R: Send,
    {
        Self::legacy_scope_guard()?;
        let mut transaction = self.pool.begin().await?;
        match f(&mut transaction).await {
            Ok(value) => {
                transaction.commit().await?;
                Ok(value)
            }
            Err(operation_error) => match transaction.rollback().await {
                Ok(()) => Err(operation_error),
                Err(rollback_error) => Err(protocol(format!(
                    "transaction operation failed ({operation_error}); rollback also failed ({rollback_error})"
                ))),
            },
        }
    }
}

async fn update_from_map<T>(pool: &PgPool, id: &str, map: Map<String, Value>) -> Result<T, Error>
where
    T: Entity + for<'r> FromRow<'r, PgRow> + Send + Unpin,
{
    let mut columns = map.keys().cloned().collect::<Vec<_>>();
    columns.sort();
    let sets = columns
        .iter()
        .map(|column| {
            let quoted = ident(column)?;
            Ok(format!("{quoted} = src.{quoted}"))
        })
        .collect::<Result<Vec<_>, Error>>()?
        .join(", ");
    let table = ident(T::table_name())?;
    let primary_key = ident(T::primary_key())?;
    let mut returning = T::schema().into_keys().collect::<Vec<_>>();
    returning.sort();
    let returning = returning
        .iter()
        .map(|column| ident(column).map(|quoted| format!("target.{quoted}")))
        .collect::<Result<Vec<_>, _>>()?
        .join(", ");
    let sql = format!(
        "UPDATE {table} AS target SET {sets}, \"updated_at\" = now() \
         FROM jsonb_populate_record(NULL::{table}, $1::jsonb) AS src \
         WHERE target.{primary_key}::text = $2 RETURNING {returning}"
    );
    sqlx::query_as::<_, T>(sqlx::AssertSqlSafe(sql))
        .bind(Json(Value::Object(map)))
        .bind(id)
        .fetch_one(pool)
        .await
}

/// SQLite-backed model manager. It deliberately mirrors the primary manager's QuerySet entry
/// point while keeping PostgreSQL-only transaction/RLS APIs out of the SQLite surface.
pub struct SqliteModelManager<T>
where
    T: Entity + for<'r> FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Sync + Unpin,
{
    pool: sqlx::SqlitePool,
    _phantom: PhantomData<T>,
}

impl<T> Clone for SqliteModelManager<T>
where
    T: Entity + for<'r> FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Sync + Unpin,
{
    fn clone(&self) -> Self {
        Self {
            pool: self.pool.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<T> SqliteModelManager<T>
where
    T: Entity + for<'r> FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Sync + Unpin,
{
    pub fn new(pool: sqlx::SqlitePool) -> Self {
        T::register();
        Self {
            pool,
            _phantom: PhantomData,
        }
    }

    pub fn objects(pool: sqlx::SqlitePool) -> Self {
        Self::new(pool)
    }

    /// Bootstrap a new SQLite development database for this model and return its manager.
    /// Existing tables are left untouched; schema evolution still belongs to migrations.
    pub async fn initialize(pool: sqlx::SqlitePool) -> Result<Self, crate::ModelError> {
        crate::ensure_sqlite_model::<T>(&pool).await?;
        Ok(Self::new(pool))
    }

    pub fn queryset(&self) -> QuerySet<T, crate::SqlitePoolExecutor> {
        QuerySet::from_executor(crate::SqlitePoolExecutor::new(self.pool.clone()))
    }

    pub fn pool(&self) -> &sqlx::SqlitePool {
        &self.pool
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ColumnSpec;
    use serde_json::json;

    #[derive(Debug, sqlx::FromRow)]
    struct DemoRow {
        id: i64,
        name: String,
        note: Option<String>,
    }

    impl Entity for DemoRow {
        fn table_name() -> &'static str {
            "demo_rows"
        }

        fn primary_key() -> &'static str {
            "id"
        }

        fn schema() -> HashMap<String, ColumnSpec> {
            HashMap::from([
                (
                    "id".to_owned(),
                    ColumnSpec {
                        sql_type: "BIGINT".to_owned(),
                        nullable: false,
                        unique: false,
                        primary_key: true,
                        default: None,
                        encrypted: false,
                    },
                ),
                (
                    "name".to_owned(),
                    ColumnSpec {
                        sql_type: "TEXT".to_owned(),
                        nullable: false,
                        unique: false,
                        primary_key: false,
                        default: None,
                        encrypted: false,
                    },
                ),
                (
                    "note".to_owned(),
                    ColumnSpec {
                        sql_type: "TEXT".to_owned(),
                        nullable: true,
                        unique: false,
                        primary_key: false,
                        default: None,
                        encrypted: false,
                    },
                ),
            ])
        }

        fn to_map(&self) -> HashMap<String, Value> {
            HashMap::from([
                ("id".to_owned(), json!(self.id)),
                ("name".to_owned(), json!(self.name)),
                ("note".to_owned(), json!(self.note)),
            ])
        }
    }

    #[test]
    fn persistence_map_rejects_unknown_and_invalid_null_fields() {
        let valid = HashMap::from([
            ("name".to_owned(), json!("alice")),
            ("note".to_owned(), Value::Null),
        ]);
        let output = ModelManager::<DemoRow>::persistable_map(valid, false)
            .unwrap_or_else(|error| panic!("persistable map failed: {error}"));
        assert_eq!(output.get("note"), Some(&Value::Null));

        let unknown = HashMap::from([("other".to_owned(), json!(1))]);
        assert!(ModelManager::<DemoRow>::persistable_map(unknown, false).is_err());

        let null_name = HashMap::from([("name".to_owned(), Value::Null)]);
        assert!(ModelManager::<DemoRow>::persistable_map(null_name, false).is_err());
    }

    #[test]
    fn patch_allows_nullable_clear_and_blocks_primary_key_mutation() {
        let patch = json!({"note": null});
        let output = ModelManager::<DemoRow>::patch_map(&patch)
            .unwrap_or_else(|error| panic!("patch failed: {error}"));
        assert_eq!(output.get("note"), Some(&Value::Null));

        assert!(ModelManager::<DemoRow>::patch_map(&json!({"id": 2})).is_err());
        assert!(ModelManager::<DemoRow>::patch_map(&json!({"name": null})).is_err());
        assert!(ModelManager::<DemoRow>::patch_map(&json!({"unknown": 1})).is_err());
    }

    #[derive(Debug, sqlx::FromRow)]
    struct TenantDemoRow {
        tenant_id: uuid::Uuid,
    }

    impl Entity for TenantDemoRow {
        fn table_name() -> &'static str {
            "tenant_demo_rows"
        }

        fn schema() -> HashMap<String, ColumnSpec> {
            HashMap::from([(
                "tenant_id".to_owned(),
                ColumnSpec {
                    sql_type: "UUID".to_owned(),
                    nullable: false,
                    unique: false,
                    primary_key: false,
                    default: None,
                    encrypted: false,
                },
            )])
        }

        fn to_map(&self) -> HashMap<String, Value> {
            HashMap::from([("tenant_id".to_owned(), json!(self.tenant_id))])
        }
    }

    #[test]
    fn legacy_repository_fails_closed_for_tenant_models() {
        let error = ModelManager::<TenantDemoRow>::legacy_scope_guard()
            .expect_err("tenant model must require scoped QuerySet API");
        assert!(error.to_string().contains("tenant-aware model"));
    }

    #[test]
    fn models_without_soft_delete_are_not_implicitly_scoped() {
        assert_eq!(
            ModelManager::<DemoRow>::live_predicate()
                .unwrap_or_else(|error| panic!("predicate failed: {error}")),
            "TRUE"
        );
    }
}
