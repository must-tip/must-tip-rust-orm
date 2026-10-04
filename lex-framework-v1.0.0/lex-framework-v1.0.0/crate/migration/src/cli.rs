use anyhow::{Context, Result, anyhow};
use clap::{Parser, Subcommand};
use sanitize_filename::sanitize;
use sqlx::{postgres::PgPoolOptions, sqlite::SqlitePoolOptions};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tracing_subscriber::EnvFilter;

use crate::{
    apply::load_current_db_schema,
    command_handlers::{
        handle_make_migration, handle_migrate, handle_migrate_tree, handle_reapply, handle_reset,
        handle_rollback, handle_show_migrations, pending_migrations, pending_migrations_tree,
    },
    diff::{DynamicEntity, diff_schema},
    sqlite_command_handlers::{
        handle_migrate_sqlite, handle_migrate_tree_sqlite, handle_reapply_sqlite,
        handle_reset_sqlite, handle_rollback_sqlite, handle_show_migrations_sqlite,
        pending_migrations_sqlite, pending_migrations_tree_sqlite,
    },
};

#[derive(Parser)]
#[command(
    name = "migration-cli",
    version = env!("CARGO_PKG_VERSION"),
    about = "Transactional PostgreSQL/SQLite migration CLI"
)]
pub struct Cli {
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
    #[arg(short = 'u', long, env = "DATABASE_URL")]
    db_url: Option<String>,
    #[arg(short, long, default_value = "./migrations", value_name = "DIR")]
    dir: PathBuf,
    #[command(subcommand)]
    cmd: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Make {
        name: String,
        #[arg(long)]
        up: PathBuf,
        #[arg(long)]
        down: PathBuf,
    },
    Migrate {
        #[arg(short, long, default_value = "all")]
        target: String,
        #[arg(long)]
        dry_run: bool,
    },
    Rollback,
    Reset {
        #[arg(long)]
        yes: bool,
    },
    Reapply,
    Status,
    Pending {
        #[arg(short, long, default_value = "all")]
        target: String,
    },
    Diff,
}

fn is_sqlite_url(url: &str) -> bool {
    url.trim().to_ascii_lowercase().starts_with("sqlite:")
}

pub async fn run_migration_cli() -> Result<()> {
    let cli = Cli::parse();
    let filter = match cli.verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .try_init();

    if let Commands::Make { name, up, down } = &cli.cmd {
        return make_from_files(&cli.dir, name, up, down).await;
    }

    let root = fs::canonicalize(&cli.dir).with_context(|| {
        format!(
            "failed to access migrations directory {}",
            cli.dir.display()
        )
    })?;
    if !root.is_dir() {
        return Err(anyhow!(
            "migrations path is not a directory: {}",
            root.display()
        ));
    }
    let db_url = cli
        .db_url
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("DATABASE_URL is required for this command"))?;

    if is_sqlite_url(&db_url) {
        run_sqlite_command(cli.cmd, &db_url, &root).await
    } else {
        run_postgres_command(cli.cmd, &db_url, &root).await
    }
}

async fn run_sqlite_command(cmd: Commands, db_url: &str, root: &Path) -> Result<()> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(db_url)
        .await
        .context("failed to connect to SQLite database")?;
    let result = match cmd {
        Commands::Make { .. } => unreachable!(),
        Commands::Migrate { target, dry_run } => {
            let migrations = if target == "all" {
                if dry_run {
                    Some(pending_migrations_tree_sqlite(&pool, path_str(root)?).await?)
                } else {
                    handle_migrate_tree_sqlite(&pool, path_str(root)?).await?;
                    None
                }
            } else {
                let target_dir = resolve_target(root, &target)?;
                if dry_run {
                    Some(pending_migrations_sqlite(&pool, path_str(&target_dir)?).await?)
                } else {
                    handle_migrate_sqlite(&pool, path_str(&target_dir)?).await?;
                    None
                }
            };
            if let Some(migrations) = migrations {
                for migration in migrations {
                    println!(
                        "-- {} {}\n{}",
                        migration.name,
                        &migration.hash[..12],
                        migration.up_sql
                    );
                }
            }
            Ok(())
        }
        Commands::Rollback => handle_rollback_sqlite(&pool, path_str(root)?).await,
        Commands::Reset { yes } => {
            if !yes {
                Err(anyhow!("reset is destructive; rerun with --yes"))
            } else {
                handle_reset_sqlite(&pool, path_str(root)?).await
            }
        }
        Commands::Reapply => handle_reapply_sqlite(&pool, path_str(root)?).await,
        Commands::Status => handle_show_migrations_sqlite(&pool).await,
        Commands::Pending { target } => {
            let migrations = if target == "all" {
                pending_migrations_tree_sqlite(&pool, path_str(root)?).await?
            } else {
                let target_dir = resolve_target(root, &target)?;
                pending_migrations_sqlite(&pool, path_str(&target_dir)?).await?
            };
            for migration in migrations {
                println!("{}", migration.name);
            }
            Ok(())
        }
        Commands::Diff => Err(anyhow!(
            "database schema introspection/diff is currently PostgreSQL-specific; SQLite migrations and status are supported, but Diff must be run against PostgreSQL"
        )),
    };
    pool.close().await;
    result
}

async fn run_postgres_command(cmd: Commands, db_url: &str, root: &Path) -> Result<()> {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(db_url)
        .await
        .context("failed to connect to PostgreSQL database")?;
    let result = match cmd {
        Commands::Make { .. } => unreachable!(),
        Commands::Migrate { target, dry_run } => {
            if target == "all" {
                if dry_run {
                    for migration in pending_migrations_tree(&pool, path_str(root)?).await? {
                        println!(
                            "-- {} {}\n{}",
                            migration.name,
                            &migration.hash[..12],
                            migration.up_sql
                        );
                    }
                    Ok(())
                } else {
                    handle_migrate_tree(&pool, path_str(root)?).await
                }
            } else {
                let target_dir = resolve_target(root, &target)?;
                if dry_run {
                    for migration in pending_migrations(&pool, path_str(&target_dir)?).await? {
                        println!(
                            "-- {} {}\n{}",
                            migration.name,
                            &migration.hash[..12],
                            migration.up_sql
                        );
                    }
                    Ok(())
                } else {
                    handle_migrate(&pool, path_str(&target_dir)?).await
                }
            }
        }
        Commands::Rollback => handle_rollback(&pool, path_str(root)?).await,
        Commands::Reset { yes } => {
            if !yes {
                Err(anyhow!("reset is destructive; rerun with --yes"))
            } else {
                handle_reset(&pool, path_str(root)?).await
            }
        }
        Commands::Reapply => handle_reapply(&pool, path_str(root)?).await,
        Commands::Status => handle_show_migrations(&pool).await,
        Commands::Pending { target } => {
            if target == "all" {
                for migration in pending_migrations_tree(&pool, path_str(root)?).await? {
                    println!("{}", migration.name);
                }
            } else {
                let target_dir = resolve_target(root, &target)?;
                for migration in pending_migrations(&pool, path_str(&target_dir)?).await? {
                    println!("{}", migration.name);
                }
            }
            Ok(())
        }
        Commands::Diff => {
            models::register_all_models();
            let current = load_current_db_schema(db_url)
                .await
                .map_err(|error| anyhow!(error))?;
            let desired = DynamicEntity::discover().map_err(|error| anyhow!(error))?;
            for statement in diff_schema(&current, &desired).map_err(|error| anyhow!(error))? {
                println!("{statement}");
            }
            Ok(())
        }
    };
    pool.close().await;
    result
}

async fn make_from_files(dir: &Path, name: &str, up: &Path, down: &Path) -> Result<()> {
    let clean = sanitize(name);
    if clean.is_empty() {
        return Err(anyhow!("invalid migration name"));
    }
    let up_sql = fs::read_to_string(up)
        .with_context(|| format!("failed reading UP SQL {}", up.display()))?;
    let down_sql = fs::read_to_string(down)
        .with_context(|| format!("failed reading DOWN SQL {}", down.display()))?;
    handle_make_migration(path_str(dir)?, &clean, &up_sql, &down_sql).await
}

fn path_str(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| anyhow!("path is not valid UTF-8: {}", path.display()))
}

fn resolve_target(root: &Path, target: &str) -> Result<PathBuf> {
    if target.is_empty() || target == "." || target == ".." {
        return Err(anyhow!("invalid migration target"));
    }
    let candidate = fs::canonicalize(root.join(target))
        .with_context(|| format!("unknown migration target: {target}"))?;
    if !candidate.starts_with(root) || !candidate.is_dir() {
        return Err(anyhow!("migration target escapes migration root: {target}"));
    }
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_detection_recognizes_sqlite() {
        assert!(is_sqlite_url("sqlite://db.sqlite3"));
        assert!(is_sqlite_url("sqlite::memory:"));
        assert!(!is_sqlite_url("postgresql://localhost/app"));
    }

    #[test]
    fn target_resolution_rejects_parent_traversal() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
        let child = dir.path().join("tenant");
        fs::create_dir(&child).unwrap_or_else(|error| panic!("mkdir: {error}"));
        let root = fs::canonicalize(dir.path()).unwrap_or_else(|error| panic!("root: {error}"));
        assert!(resolve_target(&root, "tenant").is_ok());
        assert!(resolve_target(&root, "../").is_err());
    }
}
