use anyhow::{Context, Result, anyhow};
use sqlx::{Row, SqlitePool};
use tracing::info;

use crate::scanner::{MigrationFile, read_migrations, read_migrations_tree};

async fn ensure_table(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS _migrations (\
         name TEXT PRIMARY KEY NOT NULL, \
         hash TEXT NOT NULL, \
         applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP\
         )",
    )
    .execute(pool)
    .await
    .context("failed creating SQLite migration table")?;
    Ok(())
}

async fn get_applied(pool: &SqlitePool) -> Result<std::collections::HashMap<String, String>> {
    ensure_table(pool).await?;
    let rows = sqlx::query("SELECT name, hash FROM _migrations")
        .fetch_all(pool)
        .await
        .context("failed reading SQLite migration state")?;
    let mut out = std::collections::HashMap::with_capacity(rows.len());
    for row in rows {
        out.insert(row.try_get("name")?, row.try_get("hash")?);
    }
    Ok(out)
}

async fn applied_hash_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    name: &str,
) -> Result<Option<String>> {
    sqlx::query_scalar::<_, String>("SELECT hash FROM _migrations WHERE name = ?1")
        .bind(name)
        .fetch_optional(&mut **tx)
        .await
        .with_context(|| format!("failed checking SQLite migration state for {name}"))
}

async fn apply_one(pool: &SqlitePool, migration: &MigrationFile) -> Result<()> {
    ensure_table(pool).await?;
    let mut tx = pool
        .begin()
        .await
        .context("failed starting SQLite migration transaction")?;
    match applied_hash_in_tx(&mut tx, &migration.name).await? {
        Some(hash) if hash == migration.hash => {
            tx.commit().await?;
            return Ok(());
        }
        Some(_) => {
            return Err(anyhow!(
                "migration drift detected for {}; applied migration files must never be modified",
                migration.name
            ));
        }
        None => {}
    }

    sqlx::raw_sql(sqlx::AssertSqlSafe(migration.up_sql.as_str()))
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed applying SQLite migration {}", migration.name))?;
    sqlx::query("INSERT INTO _migrations (name, hash) VALUES (?1, ?2)")
        .bind(&migration.name)
        .bind(&migration.hash)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    info!(migration = %migration.name, backend = "sqlite", "migration applied");
    Ok(())
}

async fn migrate_set(pool: &SqlitePool, migrations: Vec<MigrationFile>) -> Result<()> {
    let applied = get_applied(pool).await?;
    for migration in migrations {
        match applied.get(&migration.name) {
            Some(hash) if hash == &migration.hash => {}
            Some(_) => return Err(anyhow!("migration drift detected for {}", migration.name)),
            None => apply_one(pool, &migration).await?,
        }
    }
    Ok(())
}

pub async fn handle_migrate_sqlite(pool: &SqlitePool, dir: &str) -> Result<()> {
    migrate_set(pool, read_migrations(dir)?).await
}

pub async fn handle_migrate_tree_sqlite(pool: &SqlitePool, root: &str) -> Result<()> {
    migrate_set(pool, read_migrations_tree(root)?).await
}

async fn pending_from_set(
    pool: &SqlitePool,
    migrations: Vec<MigrationFile>,
) -> Result<Vec<MigrationFile>> {
    let applied = get_applied(pool).await?;
    let mut pending = Vec::new();
    for migration in migrations {
        match applied.get(&migration.name) {
            Some(hash) if hash == &migration.hash => {}
            Some(_) => return Err(anyhow!("migration drift detected for {}", migration.name)),
            None => pending.push(migration),
        }
    }
    Ok(pending)
}

pub async fn pending_migrations_sqlite(pool: &SqlitePool, dir: &str) -> Result<Vec<MigrationFile>> {
    pending_from_set(pool, read_migrations(dir)?).await
}

pub async fn pending_migrations_tree_sqlite(
    pool: &SqlitePool,
    root: &str,
) -> Result<Vec<MigrationFile>> {
    pending_from_set(pool, read_migrations_tree(root)?).await
}

pub async fn handle_show_migrations_sqlite(pool: &SqlitePool) -> Result<()> {
    ensure_table(pool).await?;
    let rows =
        sqlx::query("SELECT name, hash, applied_at FROM _migrations ORDER BY applied_at, name")
            .fetch_all(pool)
            .await?;
    for row in rows {
        let name: String = row.try_get("name")?;
        let hash: String = row.try_get("hash")?;
        let applied: String = row.try_get("applied_at")?;
        println!("{name} {} {applied}", &hash[..hash.len().min(12)]);
    }
    Ok(())
}

fn find_migration<'a>(
    migrations: &'a [MigrationFile],
    name: &str,
    hash: &str,
) -> Result<&'a MigrationFile> {
    let migration = migrations
        .iter()
        .find(|migration| migration.name == name)
        .ok_or_else(|| anyhow!("applied migration {name} is missing from disk"))?;
    if migration.hash != hash {
        return Err(anyhow!("migration drift detected for {name}"));
    }
    Ok(migration)
}

async fn rollback_one(pool: &SqlitePool, migration: &MigrationFile) -> Result<()> {
    let mut tx = pool
        .begin()
        .await
        .context("failed starting SQLite rollback transaction")?;
    let current_hash = applied_hash_in_tx(&mut tx, &migration.name)
        .await?
        .ok_or_else(|| anyhow!("migration {} is no longer applied", migration.name))?;
    if current_hash != migration.hash {
        return Err(anyhow!("migration drift detected for {}", migration.name));
    }
    sqlx::raw_sql(sqlx::AssertSqlSafe(migration.down_sql.as_str()))
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed rolling back SQLite migration {}", migration.name))?;
    sqlx::query("DELETE FROM _migrations WHERE name = ?1")
        .bind(&migration.name)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn handle_rollback_sqlite(pool: &SqlitePool, root: &str) -> Result<()> {
    ensure_table(pool).await?;
    let row = sqlx::query(
        "SELECT name, hash FROM _migrations ORDER BY applied_at DESC, name DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(());
    };
    let name: String = row.try_get("name")?;
    let hash: String = row.try_get("hash")?;
    let migrations = read_migrations_tree(root)?;
    rollback_one(pool, find_migration(&migrations, &name, &hash)?).await
}

pub async fn handle_reapply_sqlite(pool: &SqlitePool, root: &str) -> Result<()> {
    handle_rollback_sqlite(pool, root).await?;
    handle_migrate_tree_sqlite(pool, root).await
}

pub async fn handle_reset_sqlite(pool: &SqlitePool, root: &str) -> Result<()> {
    ensure_table(pool).await?;
    let rows =
        sqlx::query("SELECT name, hash FROM _migrations ORDER BY applied_at DESC, name DESC")
            .fetch_all(pool)
            .await?;
    let migrations = read_migrations_tree(root)?;
    for row in rows {
        let name: String = row.try_get("name")?;
        let hash: String = row.try_get("hash")?;
        rollback_one(pool, find_migration(&migrations, &name, &hash)?).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::MigrationFile;

    #[tokio::test]
    async fn sqlite_tracks_and_detects_migration_drift() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("sqlite");
        let migration = MigrationFile {
            name: "0001_create_demo".to_owned(),
            up_path: std::path::PathBuf::from("0001_create_demo_up.sql"),
            down_path: std::path::PathBuf::from("0001_create_demo_down.sql"),
            up_sql: "CREATE TABLE demo (id INTEGER PRIMARY KEY);".to_owned(),
            down_sql: "DROP TABLE demo;".to_owned(),
            hash: "abc123".to_owned(),
        };
        apply_one(&pool, &migration).await.expect("apply");
        apply_one(&pool, &migration)
            .await
            .expect("idempotent apply");
        let changed = MigrationFile {
            hash: "different".to_owned(),
            ..migration
        };
        assert!(apply_one(&pool, &changed).await.is_err());
        pool.close().await;
    }
}
