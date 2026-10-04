use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use sanitize_filename::sanitize;
use sqlx::{PgPool, Postgres, Row, Transaction};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tracing::info;

use crate::scanner::{
    MigrationFile, ensure_table, get_applied, read_migrations, read_migrations_tree,
};

const MIGRATION_ADVISORY_LOCK: i64 = 0x4d_54_44_42_4d_49_47;

pub fn write_migration_file(path: &Path, content: &str) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("migration path has no parent: {}", path.display()))?;

    fs::create_dir_all(parent)?;

    let temp = path.with_extension("sql.tmp");

    fs::write(&temp, content).with_context(|| format!("failed writing {}", temp.display()))?;

    fs::rename(&temp, path).with_context(|| format!("failed committing {}", path.display()))?;

    Ok(())
}

pub async fn handle_make_migration(dir: &str, name: &str, up: &str, down: &str) -> Result<()> {
    if up.trim().is_empty() || down.trim().is_empty() {
        return Err(anyhow!("UP and DOWN SQL must both be non-empty"));
    }

    let clean = sanitize(name);

    if clean.trim().is_empty() {
        return Err(anyhow!("migration name is empty after sanitization"));
    }

    fs::create_dir_all(dir)
        .with_context(|| format!("failed creating migration directory {dir}"))?;

    let timestamp = Utc::now().format("%Y%m%d%H%M%S%3f");
    let base = format!("{timestamp}_{clean}");

    let up_path = Path::new(dir).join(format!("{base}_up.sql"));
    let down_path = Path::new(dir).join(format!("{base}_down.sql"));

    if up_path.exists() || down_path.exists() {
        return Err(anyhow!("migration already exists: {base}"));
    }

    write_migration_file(&up_path, up)?;

    if let Err(error) = write_migration_file(&down_path, down) {
        let _ = fs::remove_file(&up_path);
        return Err(error);
    }

    info!(migration = %base, "migration files created");

    Ok(())
}

async fn acquire_migration_lock(tx: &mut Transaction<'_, Postgres>) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(MIGRATION_ADVISORY_LOCK)
        .execute(&mut **tx)
        .await
        .context("failed acquiring migration advisory lock")?;

    Ok(())
}

async fn applied_hash_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    name: &str,
) -> Result<Option<String>> {
    sqlx::query_scalar::<_, String>("SELECT hash FROM _migrations WHERE name = $1")
        .bind(name)
        .fetch_optional(&mut **tx)
        .await
        .with_context(|| format!("failed checking migration state for {name}"))
}

pub(crate) async fn apply_one(pool: &PgPool, migration: &MigrationFile) -> Result<()> {
    let mut tx = pool
        .begin()
        .await
        .context("failed starting migration transaction")?;

    acquire_migration_lock(&mut tx).await?;

    match applied_hash_in_tx(&mut tx, &migration.name).await? {
        Some(hash) if hash == migration.hash => {
            tx.commit()
                .await
                .context("failed committing migration check")?;

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

    // Reviewed compiler/migration boundary.
    //
    // Migration SQL comes exclusively from trusted, checksum-controlled
    // deployment migration artifacts. It is intentionally executed as
    // arbitrary DDL rather than through parameterized query construction.
    sqlx::raw_sql(sqlx::AssertSqlSafe(migration.up_sql.as_str()))
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed applying migration {}", migration.name))?;

    sqlx::query("INSERT INTO _migrations (name, hash) VALUES ($1, $2)")
        .bind(&migration.name)
        .bind(&migration.hash)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed tracking migration {}", migration.name))?;

    tx.commit()
        .await
        .context("failed committing migration transaction")?;

    info!(migration = %migration.name, "migration applied");

    Ok(())
}

async fn migrate_set(pool: &PgPool, migrations: Vec<MigrationFile>) -> Result<()> {
    ensure_table(pool).await?;

    let applied = get_applied(pool).await?;

    for migration in migrations {
        match applied.get(&migration.name) {
            Some(hash) if hash == &migration.hash => continue,

            Some(_) => {
                return Err(anyhow!(
                    "migration drift detected for {}; applied migration files must never be modified",
                    migration.name
                ));
            }

            None => apply_one(pool, &migration).await?,
        }
    }

    Ok(())
}

pub async fn handle_migrate(pool: &PgPool, dir: &str) -> Result<()> {
    migrate_set(pool, read_migrations(dir)?).await
}

/// Apply every migration under a root directory using one globally ordered migration set.
pub async fn handle_migrate_tree(pool: &PgPool, root: &str) -> Result<()> {
    migrate_set(pool, read_migrations_tree(root)?).await
}

pub async fn handle_show_migrations(pool: &PgPool) -> Result<()> {
    ensure_table(pool).await?;

    let rows =
        sqlx::query("SELECT name, hash, applied_at FROM _migrations ORDER BY applied_at, name")
            .fetch_all(pool)
            .await
            .context("failed reading migration status")?;

    for row in rows {
        let name: String = row.try_get("name")?;
        let hash: String = row.try_get("hash")?;
        let applied: chrono::DateTime<Utc> = row.try_get("applied_at")?;

        println!("{name} {} {applied}", &hash[..hash.len().min(12)]);
    }

    Ok(())
}

async fn pending_from_set(
    pool: &PgPool,
    migrations: Vec<MigrationFile>,
) -> Result<Vec<MigrationFile>> {
    ensure_table(pool).await?;

    let applied = get_applied(pool).await?;
    let mut pending = Vec::new();

    for migration in migrations {
        match applied.get(&migration.name) {
            Some(hash) if hash == &migration.hash => {}

            Some(_) => {
                return Err(anyhow!("migration drift detected for {}", migration.name));
            }

            None => pending.push(migration),
        }
    }

    Ok(pending)
}

pub async fn pending_migrations(pool: &PgPool, dir: &str) -> Result<Vec<MigrationFile>> {
    pending_from_set(pool, read_migrations(dir)?).await
}

pub async fn pending_migrations_tree(pool: &PgPool, root: &str) -> Result<Vec<MigrationFile>> {
    pending_from_set(pool, read_migrations_tree(root)?).await
}

pub async fn handle_show_pending(pool: &PgPool, dir: &str) -> Result<()> {
    for migration in pending_migrations(pool, dir).await? {
        println!("{}", migration.name);
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

async fn rollback_one(pool: &PgPool, migration: &MigrationFile) -> Result<()> {
    let mut tx = pool
        .begin()
        .await
        .context("failed starting rollback transaction")?;

    acquire_migration_lock(&mut tx).await?;

    let current_hash = applied_hash_in_tx(&mut tx, &migration.name)
        .await?
        .ok_or_else(|| anyhow!("migration {} is no longer applied", migration.name))?;

    if current_hash != migration.hash {
        return Err(anyhow!("migration drift detected for {}", migration.name));
    }

    // Reviewed compiler/migration boundary.
    //
    // Rollback SQL comes from the same checksum-verified migration
    // artifact as its UP migration.
    sqlx::raw_sql(sqlx::AssertSqlSafe(migration.down_sql.as_str()))
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed rolling back {}", migration.name))?;

    sqlx::query("DELETE FROM _migrations WHERE name = $1")
        .bind(&migration.name)
        .execute(&mut *tx)
        .await
        .context("failed removing migration record")?;

    tx.commit().await.context("failed committing rollback")?;

    info!(migration = %migration.name, "migration rolled back");

    Ok(())
}

pub async fn handle_rollback(pool: &PgPool, dir: &str) -> Result<()> {
    ensure_table(pool).await?;

    let row = sqlx::query(
        "SELECT name, hash FROM _migrations \
         ORDER BY applied_at DESC, name DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .context("failed reading latest migration")?;

    let Some(row) = row else {
        return Ok(());
    };

    let name: String = row.try_get("name")?;
    let hash: String = row.try_get("hash")?;

    let migrations = read_migrations_tree(dir)?;
    let migration = find_migration(&migrations, &name, &hash)?;

    rollback_one(pool, migration).await
}

pub async fn handle_reapply(pool: &PgPool, dir: &str) -> Result<()> {
    handle_rollback(pool, dir).await?;
    handle_migrate_tree(pool, dir).await
}

pub async fn handle_reset(pool: &PgPool, dir: &str) -> Result<()> {
    ensure_table(pool).await?;

    let rows = sqlx::query(
        "SELECT name, hash FROM _migrations \
         ORDER BY applied_at DESC, name DESC",
    )
    .fetch_all(pool)
    .await
    .context("failed reading applied migrations")?;

    let migrations = read_migrations_tree(dir)?;

    for row in rows {
        let name: String = row.try_get("name")?;
        let hash: String = row.try_get("hash")?;

        let migration = find_migration(&migrations, &name, &hash)?;

        rollback_one(pool, migration).await?;
    }

    Ok(())
}

pub fn migration_path(dir: &Path, name: &str, up: bool) -> PathBuf {
    dir.join(format!("{name}_{}.sql", if up { "up" } else { "down" },))
}
