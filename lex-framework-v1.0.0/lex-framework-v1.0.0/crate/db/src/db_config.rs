use crate::config::{DatabaseBackend, Settings};
use anyhow::{Context, Result};
use sqlx::{
    PgPool, SqlitePool,
    postgres::{PgConnectOptions, PgPoolOptions},
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tokio::time::sleep;
use tracing::{info, warn};

#[derive(Clone)]
pub enum DatabasePool {
    Postgres(PgPool),
    Sqlite(SqlitePool),
}

impl DatabasePool {
    pub fn backend(&self) -> DatabaseBackend {
        match self {
            Self::Postgres(_) => DatabaseBackend::Postgres,
            Self::Sqlite(_) => DatabaseBackend::Sqlite,
        }
    }

    pub fn postgres(&self) -> Option<&PgPool> {
        match self {
            Self::Postgres(pool) => Some(pool),
            Self::Sqlite(_) => None,
        }
    }

    pub fn sqlite(&self) -> Option<&SqlitePool> {
        match self {
            Self::Sqlite(pool) => Some(pool),
            Self::Postgres(_) => None,
        }
    }
}

pub struct Database {
    pool: DatabasePool,
    read_pools: Vec<PgPool>,
    next_replica: AtomicUsize,
}

impl Database {
    pub async fn initialize() -> Result<Self> {
        let settings = Settings::load(None)?;
        Self::initialize_with(&settings).await
    }

    pub async fn initialize_with(settings: &Settings) -> Result<Self> {
        settings.validate()?;
        match settings.database_backend()? {
            DatabaseBackend::Postgres => Self::initialize_postgres(settings).await,
            DatabaseBackend::Sqlite => Self::initialize_sqlite(settings).await,
            DatabaseBackend::Auto => unreachable!("validated backend cannot remain Auto"),
        }
    }

    async fn initialize_postgres(settings: &Settings) -> Result<Self> {
        let pool_options = PgPoolOptions::new()
            .max_connections(settings.database.max_connections)
            .min_connections(settings.database.min_connections)
            .acquire_timeout(Duration::from_secs(settings.database.connect_timeout_secs))
            .idle_timeout(Some(Duration::from_secs(300)))
            .max_lifetime(Some(Duration::from_secs(1800)));
        let pool = connect_postgres_pool(
            pool_options,
            settings.pg_options()?,
            settings.database.connect_retries,
            "primary",
        )
        .await
        .context("primary database connection failed")?;

        let replica_options = PgPoolOptions::new()
            .max_connections(settings.database.read_replica_max_connections)
            .min_connections(settings.database.read_replica_min_connections)
            .acquire_timeout(Duration::from_secs(settings.database.connect_timeout_secs))
            .idle_timeout(Some(Duration::from_secs(300)))
            .max_lifetime(Some(Duration::from_secs(1800)));
        let mut read_pools = Vec::with_capacity(settings.database.read_replica_urls.len());
        for (index, url) in settings.database.read_replica_urls.iter().enumerate() {
            match connect_postgres_pool(
                replica_options.clone(),
                settings.pg_options_for_url(url)?,
                settings.database.connect_retries,
                "read-replica",
            )
            .await
            {
                Ok(replica) => read_pools.push(replica),
                Err(error) if settings.database.require_all_read_replicas => {
                    for replica in read_pools {
                        replica.close().await;
                    }
                    pool.close().await;
                    return Err(error).with_context(|| {
                        format!("required read replica {index} failed to initialize")
                    });
                }
                Err(error) => {
                    warn!(replica_index = index, error = %error, "read replica unavailable; primary fallback remains active")
                }
            }
        }

        // Existing SQL-file migrations are PostgreSQL-specific and remain available unchanged.
        if settings.database.run_migrations {
            let root = settings
                .database
                .migrations_dir
                .to_str()
                .context("migrations directory is not valid UTF-8")?;
            migration::handle_migrate_tree(&pool, root)
                .await
                .context("database migrations failed")?;
        }
        sqlx::query("SELECT 1")
            .execute(&pool)
            .await
            .context("database health check failed")?;
        info!(
            backend = "postgres",
            max_connections = settings.database.max_connections,
            read_replica_count = read_pools.len(),
            "database initialized"
        );
        Ok(Self {
            pool: DatabasePool::Postgres(pool),
            read_pools,
            next_replica: AtomicUsize::new(0),
        })
    }

    async fn initialize_sqlite(settings: &Settings) -> Result<Self> {
        // A SQLite in-memory URL creates a database per connection. Force a one-connection
        // pool so every ORM operation in the process observes the same database. File-backed
        // SQLite can safely use a bounded pool.
        let in_memory = settings
            .db_url()
            .trim()
            .to_ascii_lowercase()
            .starts_with("sqlite::memory:");
        let max_connections = if in_memory {
            1
        } else {
            settings.database.max_connections.min(16)
        };
        let min_connections = if in_memory {
            1
        } else {
            settings.database.min_connections.min(max_connections)
        };
        let options = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .min_connections(min_connections)
            .acquire_timeout(Duration::from_secs(settings.database.connect_timeout_secs));
        let pool = connect_sqlite_pool(
            options,
            settings.sqlite_options()?,
            settings.database.connect_retries,
        )
        .await
        .context("SQLite database connection failed")?;
        // SqliteConnectOptions enables foreign keys on every connection, including
        // replacements. A one-off PRAGMA on a pool cannot establish that invariant.
        if settings.database.run_migrations {
            if !settings.database.migrations_dir.is_dir() {
                pool.close().await;
                return Err(anyhow::anyhow!(
                    "migrations directory does not exist: {}",
                    settings.database.migrations_dir.display()
                ));
            }
            let root = settings
                .database
                .migrations_dir
                .to_str()
                .context("migrations directory is not valid UTF-8")?;
            if let Err(error) = migration::handle_migrate_tree_sqlite(&pool, root).await {
                pool.close().await;
                return Err(error).context("SQLite database migrations failed");
            }
        }
        sqlx::query("SELECT 1")
            .execute(&pool)
            .await
            .context("SQLite health check failed")?;
        info!(backend = "sqlite", max_connections, "database initialized");
        Ok(Self {
            pool: DatabasePool::Sqlite(pool),
            read_pools: Vec::new(),
            next_replica: AtomicUsize::new(0),
        })
    }

    pub fn backend(&self) -> DatabaseBackend {
        self.pool.backend()
    }
    pub fn pool(&self) -> &DatabasePool {
        &self.pool
    }
    pub fn postgres_pool(&self) -> Option<&PgPool> {
        self.pool.postgres()
    }
    pub fn sqlite_pool(&self) -> Option<&SqlitePool> {
        self.pool.sqlite()
    }

    pub fn postgres_write_pool(&self) -> Option<&PgPool> {
        self.postgres_pool()
    }

    pub fn postgres_read_pool(&self) -> Option<&PgPool> {
        let primary = self.postgres_pool()?;
        if self.read_pools.is_empty() {
            return Some(primary);
        }
        let index = self.next_replica.fetch_add(1, Ordering::Relaxed) % self.read_pools.len();
        Some(&self.read_pools[index])
    }

    pub fn read_replica_count(&self) -> usize {
        self.read_pools.len()
    }

    pub async fn health_check(&self) -> Result<()> {
        match &self.pool {
            DatabasePool::Postgres(pool) => {
                sqlx::query("SELECT 1").execute(pool).await?;
            }
            DatabasePool::Sqlite(pool) => {
                sqlx::query("SELECT 1").execute(pool).await?;
            }
        }
        Ok(())
    }

    pub async fn health_check_all(&self) -> Result<()> {
        self.health_check().await?;
        for (index, replica) in self.read_pools.iter().enumerate() {
            sqlx::query("SELECT 1")
                .execute(replica)
                .await
                .with_context(|| format!("read replica {index} health check failed"))?;
        }
        Ok(())
    }

    pub async fn close(self) {
        for replica in self.read_pools {
            replica.close().await;
        }
        match self.pool {
            DatabasePool::Postgres(pool) => pool.close().await,
            DatabasePool::Sqlite(pool) => pool.close().await,
        }
    }
}

async fn connect_postgres_pool(
    pool_options: PgPoolOptions,
    connect_options: PgConnectOptions,
    retries: u32,
    role: &'static str,
) -> Result<PgPool> {
    let mut delay = Duration::from_millis(250);
    for attempt in 1..=retries {
        match pool_options
            .clone()
            .connect_with(connect_options.clone())
            .await
        {
            Ok(pool) => match sqlx::query("SELECT 1").execute(&pool).await {
                Ok(_) => return Ok(pool),
                Err(error) => {
                    pool.close().await;
                    if attempt == retries {
                        return Err(error.into());
                    }
                    warn!(attempt, role, error = %error, "database health check failed; retrying");
                }
            },
            Err(error) if attempt == retries => return Err(error.into()),
            Err(error) => {
                warn!(attempt, role, error = %error, "database connection failed; retrying")
            }
        }
        sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(5));
    }
    Err(anyhow::anyhow!(
        "{role} database connection loop ended unexpectedly"
    ))
}

async fn connect_sqlite_pool(
    pool_options: SqlitePoolOptions,
    connect_options: SqliteConnectOptions,
    retries: u32,
) -> Result<SqlitePool> {
    let mut delay = Duration::from_millis(100);
    for attempt in 1..=retries {
        match pool_options
            .clone()
            .connect_with(connect_options.clone())
            .await
        {
            Ok(pool) => match sqlx::query("SELECT 1").execute(&pool).await {
                Ok(_) => return Ok(pool),
                Err(error) => {
                    pool.close().await;
                    if attempt == retries {
                        return Err(error.into());
                    }
                    warn!(attempt, error = %error, "SQLite health check failed; retrying");
                }
            },
            Err(error) if attempt == retries => return Err(error.into()),
            Err(error) => warn!(attempt, error = %error, "SQLite connection failed; retrying"),
        }
        sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(2));
    }
    Err(anyhow::anyhow!(
        "SQLite database connection loop ended unexpectedly"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_sqlite_initializes_without_external_services() {
        let mut settings = Settings::default();
        settings.database.backend = DatabaseBackend::Sqlite;
        settings.database.url = "sqlite::memory:".to_owned();
        settings.database.require_ssl = false;
        settings.database.max_connections = 1;
        settings.database.min_connections = 1;
        let database = Database::initialize_with(&settings)
            .await
            .expect("SQLite should initialize");
        assert_eq!(database.backend(), DatabaseBackend::Sqlite);
        assert!(database.sqlite_pool().is_some());
        assert!(database.postgres_pool().is_none());
        database.health_check().await.expect("SQLite health check");
        database.close().await;
    }
}
