use anyhow::{Context, Result, anyhow};
use db_controller::{ColumnSpec, TABLE_REGISTRY};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationFile {
    pub name: String,
    pub up_path: PathBuf,
    pub down_path: PathBuf,
    pub up_sql: String,
    pub down_sql: String,
    pub hash: String,
}

#[derive(Debug, FromRow)]
struct MigrationRow {
    name: String,
    hash: String,
}

fn migration_hash(up: &str, down: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hasher = Sha256::new();
    hasher.update(b"up\0");
    hasher.update(up.as_bytes());
    hasher.update(b"\0down\0");
    hasher.update(down.as_bytes());
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

fn read_utf8(path: &Path) -> Result<String> {
    fs::read_to_string(path)
        .with_context(|| format!("failed reading migration file {}", path.display()))
}

pub fn read_migrations(dir: &str) -> Result<Vec<MigrationFile>> {
    let dir = Path::new(dir);
    if !dir.is_dir() {
        return Err(anyhow!(
            "migration directory does not exist: {}",
            dir.display()
        ));
    }

    let mut entries = fs::read_dir(dir)
        .with_context(|| format!("failed reading migration directory {}", dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.path());

    let down_names = entries
        .iter()
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            path.file_name()
                .and_then(|value| value.to_str())
                .and_then(|name| name.strip_suffix("_down.sql"))
                .map(str::to_owned)
        })
        .collect::<HashSet<_>>();

    let mut migrations = Vec::new();
    let mut up_names = HashSet::new();
    for entry in entries {
        let up_path = entry.path();
        if !up_path.is_file() {
            continue;
        }
        let Some(file) = up_path.file_name().and_then(|value| value.to_str()) else {
            return Err(anyhow!(
                "migration filename is not UTF-8: {}",
                up_path.display()
            ));
        };
        let Some(name) = file.strip_suffix("_up.sql") else {
            continue;
        };
        let name = name.to_owned();
        if !up_names.insert(name.clone()) {
            return Err(anyhow!("duplicate migration name: {name}"));
        }

        let down_path = dir.join(format!("{name}_down.sql"));
        if !down_path.is_file() {
            return Err(anyhow!(
                "missing down migration for {name}: {}",
                down_path.display()
            ));
        }
        let up_sql = read_utf8(&up_path)?;
        let down_sql = read_utf8(&down_path)?;
        if up_sql.trim().is_empty() || down_sql.trim().is_empty() {
            return Err(anyhow!(
                "migration {name} contains an empty UP or DOWN script"
            ));
        }
        let hash = migration_hash(&up_sql, &down_sql);
        migrations.push(MigrationFile {
            name,
            up_path,
            down_path,
            up_sql,
            down_sql,
            hash,
        });
    }

    let mut orphan_down = down_names
        .difference(&up_names)
        .cloned()
        .collect::<Vec<_>>();
    orphan_down.sort();
    if let Some(name) = orphan_down.first() {
        return Err(anyhow!("missing up migration for {name}"));
    }

    migrations.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(migrations)
}

/// Read migration pairs from `root` and all descendant directories.
/// Migration names are globally unique because `_migrations.name` is the durable key.
pub fn read_migrations_tree(root: &str) -> Result<Vec<MigrationFile>> {
    let root = Path::new(root);
    if !root.is_dir() {
        return Err(anyhow!(
            "migration directory does not exist: {}",
            root.display()
        ));
    }

    fn walk(dir: &Path, output: &mut Vec<MigrationFile>) -> Result<()> {
        let dir_string = dir
            .to_str()
            .ok_or_else(|| anyhow!("migration path is not valid UTF-8: {}", dir.display()))?;
        output.extend(read_migrations(dir_string)?);

        let mut children = fs::read_dir(dir)
            .with_context(|| format!("failed reading migration directory {}", dir.display()))?
            .collect::<std::io::Result<Vec<_>>>()?;
        children.sort_by_key(|entry| entry.path());
        for entry in children {
            if entry.file_type()?.is_dir() {
                walk(&entry.path(), output)?;
            }
        }
        Ok(())
    }

    let mut migrations = Vec::new();
    walk(root, &mut migrations)?;
    migrations.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.up_path.cmp(&right.up_path))
    });

    let mut seen = HashSet::new();
    for migration in &migrations {
        if !seen.insert(migration.name.clone()) {
            return Err(anyhow!(
                "duplicate migration name across migration tree: {}",
                migration.name
            ));
        }
    }
    Ok(migrations)
}

pub async fn ensure_table(pool: &PgPool) -> Result<()> {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS _migrations (
        name TEXT PRIMARY KEY,
        hash TEXT NOT NULL,
        applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
    )"#,
    )
    .execute(pool)
    .await
    .context("failed creating _migrations table")?;
    Ok(())
}

pub async fn get_applied(pool: &PgPool) -> Result<HashMap<String, String>> {
    let rows = sqlx::query_as::<_, MigrationRow>("SELECT name, hash FROM _migrations")
        .fetch_all(pool)
        .await
        .context("failed reading applied migrations")?;
    Ok(rows.into_iter().map(|row| (row.name, row.hash)).collect())
}

pub fn collect_schemas() -> HashMap<String, HashMap<String, ColumnSpec>> {
    TABLE_REGISTRY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .map(|(name, migration)| (name.clone(), migration.schema.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sorted_pairs_and_hashes_both_directions() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
        fs::write(dir.path().join("002_b_up.sql"), "CREATE TABLE b(id INT);")
            .unwrap_or_else(|error| panic!("write: {error}"));
        fs::write(dir.path().join("002_b_down.sql"), "DROP TABLE b;")
            .unwrap_or_else(|error| panic!("write: {error}"));
        fs::write(dir.path().join("001_a_up.sql"), "CREATE TABLE a(id INT);")
            .unwrap_or_else(|error| panic!("write: {error}"));
        fs::write(dir.path().join("001_a_down.sql"), "DROP TABLE a;")
            .unwrap_or_else(|error| panic!("write: {error}"));

        let path = dir.path().to_str().unwrap_or_else(|| panic!("utf8"));
        let values = read_migrations(path).unwrap_or_else(|error| panic!("read: {error}"));
        assert_eq!(
            values
                .iter()
                .map(|migration| migration.name.as_str())
                .collect::<Vec<_>>(),
            vec!["001_a", "002_b"]
        );

        let old_hash = values[0].hash.clone();
        fs::write(dir.path().join("001_a_down.sql"), "DROP TABLE a CASCADE;")
            .unwrap_or_else(|error| panic!("write: {error}"));
        let changed = read_migrations(path).unwrap_or_else(|error| panic!("read: {error}"));
        assert_ne!(old_hash, changed[0].hash);
    }

    #[test]
    fn tree_reader_rejects_duplicate_names_across_targets() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
        for target in ["a", "b"] {
            let target_dir = dir.path().join(target);
            fs::create_dir(&target_dir).unwrap_or_else(|error| panic!("mkdir: {error}"));
            fs::write(target_dir.join("001_same_up.sql"), "SELECT 1;")
                .unwrap_or_else(|error| panic!("write: {error}"));
            fs::write(target_dir.join("001_same_down.sql"), "SELECT 1;")
                .unwrap_or_else(|error| panic!("write: {error}"));
        }
        let root = dir.path().to_str().unwrap_or_else(|| panic!("utf8"));
        assert!(read_migrations_tree(root).is_err());
    }

    #[test]
    fn rejects_missing_direction_files() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
        let path = dir.path().to_str().unwrap_or_else(|| panic!("utf8"));

        fs::write(dir.path().join("001_a_up.sql"), "SELECT 1;")
            .unwrap_or_else(|error| panic!("write: {error}"));
        assert!(read_migrations(path).is_err());

        fs::remove_file(dir.path().join("001_a_up.sql"))
            .unwrap_or_else(|error| panic!("remove: {error}"));
        fs::write(dir.path().join("001_a_down.sql"), "SELECT 1;")
            .unwrap_or_else(|error| panic!("write: {error}"));
        assert!(read_migrations(path).is_err());
    }
}
