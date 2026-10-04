use db_controller::ColumnSpec;
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::collections::HashMap;

use crate::{
    command_handlers::{
        handle_migrate_tree, handle_reapply, handle_reset, handle_rollback, handle_show_migrations,
        handle_show_pending,
    },
    scanner::{MigrationFile, ensure_table, get_applied},
};

pub async fn ensure_migration_table(pool: &PgPool) -> Result<(), sqlx::Error> {
    ensure_table(pool)
        .await
        .map_err(|error| sqlx::Error::Protocol(error.to_string()))
}

pub async fn fetch_applied(pool: &PgPool) -> Result<HashMap<String, String>, sqlx::Error> {
    get_applied(pool)
        .await
        .map_err(|error| sqlx::Error::Protocol(error.to_string()))
}

pub async fn apply_migration(pool: &PgPool, migration: &MigrationFile) -> Result<(), sqlx::Error> {
    ensure_migration_table(pool).await?;
    crate::command_handlers::apply_one(pool, migration)
        .await
        .map_err(|error| sqlx::Error::Protocol(error.to_string()))
}

pub async fn apply_migrations(url: &str, statements: &[String]) -> Result<(), String> {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await
        .map_err(|error| format!("failed to connect to database: {error}"))?;

    let result = async {
        let mut tx = pool
            .begin()
            .await
            .map_err(|error| format!("failed to begin transaction: {error}"))?;

        for statement in statements {
            // Reviewed migration boundary:
            // migration SQL is intentionally operator-supplied DDL and must be executed as a raw batch.
            sqlx::raw_sql(sqlx::AssertSqlSafe(statement.as_str()))
                .execute(&mut *tx)
                .await
                .map_err(|error| format!("failed applying SQL batch: {error}"))?;
        }

        tx.commit()
            .await
            .map_err(|error| format!("failed to commit SQL batch: {error}"))
    }
    .await;

    pool.close().await;
    result
}

pub async fn load_current_db_schema(
    url: &str,
) -> Result<HashMap<String, HashMap<String, ColumnSpec>>, String> {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await
        .map_err(|error| format!("failed to connect to database: {error}"))?;

    let result = async {
        let rows = sqlx::query(
            r#"SELECT c.table_name,c.column_name,c.data_type,c.udt_name,c.is_nullable,c.column_default,
              EXISTS (
                SELECT 1 FROM information_schema.table_constraints tc
                JOIN information_schema.key_column_usage kcu
                  ON tc.constraint_name=kcu.constraint_name AND tc.table_schema=kcu.table_schema
                WHERE tc.table_schema=c.table_schema AND tc.table_name=c.table_name
                  AND kcu.column_name=c.column_name AND tc.constraint_type='PRIMARY KEY'
              ) AS is_primary,
              EXISTS (
                SELECT 1 FROM information_schema.table_constraints tc
                JOIN information_schema.key_column_usage kcu
                  ON tc.constraint_name=kcu.constraint_name AND tc.table_schema=kcu.table_schema
                WHERE tc.table_schema=c.table_schema AND tc.table_name=c.table_name
                  AND kcu.column_name=c.column_name AND tc.constraint_type='UNIQUE'
              ) AS is_unique
              FROM information_schema.columns c
              WHERE c.table_schema=current_schema() AND c.table_name <> '_migrations'
              ORDER BY c.table_name,c.ordinal_position"#,
        )
        .fetch_all(&pool)
        .await
        .map_err(|error| format!("failed reading information_schema: {error}"))?;

        let mut schema: HashMap<String, HashMap<String, ColumnSpec>> = HashMap::new();

        for row in rows {
            let table: String = row
                .try_get("table_name")
                .map_err(|error| error.to_string())?;

            let column: String = row
                .try_get("column_name")
                .map_err(|error| error.to_string())?;

            let data_type: String = row
                .try_get("data_type")
                .map_err(|error| error.to_string())?;

            let udt: String = row
                .try_get("udt_name")
                .map_err(|error| error.to_string())?;

            let nullable: String = row
                .try_get("is_nullable")
                .map_err(|error| error.to_string())?;

            let mut default: Option<String> = row
                .try_get("column_default")
                .map_err(|error| error.to_string())?;

            let primary: bool = row
                .try_get("is_primary")
                .map_err(|error| error.to_string())?;

            let unique: bool = row
                .try_get("is_unique")
                .map_err(|error| error.to_string())?;

            let mut sql_type = normalize_pg_type(&data_type, &udt);

            if primary
                && udt == "int8"
                && default
                    .as_deref()
                    .is_some_and(|value| value.trim_start().starts_with("nextval("))
            {
                // BIGSERIAL is represented by PostgreSQL as BIGINT + sequence default.
                // Convert it back to the logical model type to avoid perpetual migration drift.
                sql_type = "BIGSERIAL".to_owned();
                default = None;
            }

            schema.entry(table).or_default().insert(
                column,
                ColumnSpec {
                    sql_type,
                    nullable: nullable == "YES",
                    unique,
                    primary_key: primary,
                    default,
                    encrypted: false,
                },
            );
        }

        Ok(schema)
    }
    .await;

    pool.close().await;
    result
}

fn normalize_pg_type(data_type: &str, udt: &str) -> String {
    match udt {
        "int2" => "SMALLINT",
        "int4" => "INTEGER",
        "int8" => "BIGINT",
        "float4" => "REAL",
        "float8" => "DOUBLE PRECISION",
        "bool" => "BOOLEAN",
        "uuid" => "UUID",
        "timestamptz" => "TIMESTAMPTZ",
        "timestamp" => "TIMESTAMP",
        "numeric" => "NUMERIC",
        "json" => "JSON",
        "jsonb" => "JSONB",
        "bytea" => "BYTEA",
        "text" => "TEXT",
        "varchar" => "CHARACTER VARYING",
        _ => data_type,
    }
    .to_ascii_uppercase()
}

pub async fn dispatch_command(command: &str, db_url: &str, dir: &str) -> Result<(), String> {
    let pool = PgPoolOptions::new()
        .connect(db_url)
        .await
        .map_err(|error| format!("failed to connect: {error}"))?;

    let result = match command {
        "migrate" => handle_migrate_tree(&pool, dir)
            .await
            .map_err(|error| error.to_string()),

        "reset" => handle_reset(&pool, dir)
            .await
            .map_err(|error| error.to_string()),

        "reapply" => handle_reapply(&pool, dir)
            .await
            .map_err(|error| error.to_string()),

        "rollback" => handle_rollback(&pool, dir)
            .await
            .map_err(|error| error.to_string()),

        "pending" => handle_show_pending(&pool, dir)
            .await
            .map_err(|error| error.to_string()),

        "list" | "show" => handle_show_migrations(&pool)
            .await
            .map_err(|error| error.to_string()),

        "make" => Err(
            "make requires an explicit migration name and UP/DOWN SQL; use handle_make_migration"
                .to_owned(),
        ),

        _ => Err(format!("unknown migration command: {command}")),
    };

    pool.close().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_types_are_normalized_to_model_types() {
        assert_eq!(normalize_pg_type("bigint", "int8"), "BIGINT");
        assert_eq!(
            normalize_pg_type("timestamp with time zone", "timestamptz"),
            "TIMESTAMPTZ"
        );
        assert_eq!(normalize_pg_type("USER-DEFINED", "custom"), "USER-DEFINED");
    }
}
