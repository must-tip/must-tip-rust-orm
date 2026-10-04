use crate::{Entity, ModelError, generate_create_table_sql_sqlite, render_index_spec_sqlite};
use sqlx::SqlitePool;

/// Create the SQLite table and compatible indexes for one model if they do not already exist.
///
/// This is intentionally non-destructive: it bootstraps a new developer database, but it does
/// not pretend to be a schema-diff migration engine for an existing database. Schema evolution
/// should still use reviewed migrations.
pub async fn ensure_sqlite_model<T: Entity>(pool: &SqlitePool) -> Result<(), ModelError> {
    crate::query::validate_sqlite_decimal_schema::<T>()?;
    let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(pool)
        .await?;
    if foreign_keys != 1 {
        return Err(ModelError::InvalidQuery("SQLite pools must enable foreign_keys on every connection through SqliteConnectOptions".into()));
    }
    let ddl = generate_create_table_sql_sqlite::<T>();
    sqlx::query(sqlx::AssertSqlSafe(ddl)).execute(pool).await?;
    let mut indexes = T::typed_indexes();
    indexes.sort_by(|left, right| left.name.cmp(right.name));
    for index in indexes {
        let sql = render_index_spec_sqlite::<T>(&index).map_err(ModelError::SqlBuildError)?;
        sqlx::query(sqlx::AssertSqlSafe(sql)).execute(pool).await?;
    }
    Ok(())
}
