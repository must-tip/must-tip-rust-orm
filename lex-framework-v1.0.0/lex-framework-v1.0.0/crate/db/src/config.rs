use clap::ArgMatches;
use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;
use sqlx::{
    postgres::{PgConnectOptions, PgSslMode},
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
};
use std::{
    env,
    net::{AddrParseError, SocketAddr},
    path::PathBuf,
    str::FromStr,
};
use thiserror::Error;

pub const DEFAULT_SQLITE_URL: &str = "sqlite://db.sqlite3";

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("configuration error: {0}")]
    Config(#[from] ConfigError),
    #[error("DATABASE_URL is required in production")]
    MissingDatabaseUrl,
    #[error("invalid DATABASE_URL: {0}")]
    InvalidDatabaseUrl(String),
    #[error("database backend/url mismatch: {0}")]
    BackendMismatch(String),
    #[error("invalid server address: {0}")]
    InvalidServerAddress(#[from] AddrParseError),
    #[error("invalid configuration: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum DatabaseBackend {
    #[default]
    Auto,
    Postgres,
    Sqlite,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct Settings {
    pub debug: bool,
    pub database: DatabaseSettings,
    pub server: ServerSettings,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct DatabaseSettings {
    pub backend: DatabaseBackend,
    #[serde(skip)]
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub connect_timeout_secs: u64,
    pub connect_retries: u32,
    pub require_ssl: bool,
    pub run_migrations: bool,
    pub migrations_dir: PathBuf,
    #[serde(skip)]
    pub read_replica_urls: Vec<String>,
    pub read_replica_max_connections: u32,
    pub read_replica_min_connections: u32,
    pub require_all_read_replicas: bool,
    pub sqlite_create_if_missing: bool,
    pub sqlite_foreign_keys: bool,
    pub sqlite_wal: bool,
    pub sqlite_busy_timeout_ms: u64,
}

impl std::fmt::Debug for DatabaseSettings {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DatabaseSettings")
            .field("backend", &self.backend)
            .field("url", &"[REDACTED]")
            .field("read_replica_count", &self.read_replica_urls.len())
            .field("max_connections", &self.max_connections)
            .finish_non_exhaustive()
    }
}

/// Production profile detection shared by database and HTTP configuration.
pub fn is_production() -> bool {
    production_profile(&env::var("APP_PROFILE").unwrap_or_default())
}

fn production_profile(profile: &str) -> bool {
    matches!(
        profile.trim().to_ascii_lowercase().as_str(),
        "prod" | "production"
    )
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ServerSettings {
    pub host: String,
    pub port: u16,
}

impl Default for DatabaseSettings {
    fn default() -> Self {
        Self {
            backend: DatabaseBackend::Auto,
            url: String::new(),
            max_connections: 20,
            min_connections: 1,
            connect_timeout_secs: 10,
            connect_retries: 5,
            require_ssl: true,
            run_migrations: false,
            migrations_dir: PathBuf::from("./migrations"),
            read_replica_urls: Vec::new(),
            read_replica_max_connections: 10,
            read_replica_min_connections: 0,
            require_all_read_replicas: false,
            sqlite_create_if_missing: true,
            sqlite_foreign_keys: true,
            sqlite_wal: true,
            sqlite_busy_timeout_ms: 5_000,
        }
    }
}

impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_owned(),
            port: 8080,
        }
    }
}

impl Settings {
    pub fn load(cli_args: Option<&ArgMatches>) -> Result<Self, SettingsError> {
        let production = is_production();
        if !production {
            let _ = dotenvy::dotenv();
        }

        let config = Config::builder()
            .add_source(File::with_name("Config").required(false))
            .add_source(Environment::with_prefix("APP").separator("__"))
            .build()?;
        let mut settings: Self = config.try_deserialize()?;
        settings.database.url = cli_args
            .and_then(|args| args.get_one::<String>("db_url").cloned())
            .or_else(|| env::var("DATABASE_URL").ok())
            .filter(|url| !url.trim().is_empty())
            .unwrap_or_else(|| {
                if production {
                    String::new()
                } else {
                    DEFAULT_SQLITE_URL.to_owned()
                }
            });
        if production && settings.database.url.is_empty() {
            return Err(SettingsError::MissingDatabaseUrl);
        }
        settings.database.read_replica_urls = load_read_replica_urls()?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn db_url(&self) -> &str {
        &self.database.url
    }

    pub fn database_backend(&self) -> Result<DatabaseBackend, SettingsError> {
        let detected = backend_from_url(self.db_url())?;
        match self.database.backend {
            DatabaseBackend::Auto => Ok(detected),
            configured if configured == detected => Ok(configured),
            configured => Err(SettingsError::BackendMismatch(format!(
                "configured backend {configured:?} does not match detected backend {detected:?}"
            ))),
        }
    }

    pub fn validate(&self) -> Result<(), SettingsError> {
        if self.database.max_connections == 0 {
            return Err(SettingsError::Invalid(
                "database.max_connections must be > 0".to_owned(),
            ));
        }
        if self.database.min_connections > self.database.max_connections {
            return Err(SettingsError::Invalid(
                "database.min_connections cannot exceed max_connections".to_owned(),
            ));
        }
        if self.database.connect_timeout_secs == 0 || self.database.connect_retries == 0 {
            return Err(SettingsError::Invalid(
                "database connect timeout/retries must be > 0".to_owned(),
            ));
        }
        if self.database.sqlite_busy_timeout_ms == 0 {
            return Err(SettingsError::Invalid(
                "database.sqlite_busy_timeout_ms must be > 0".to_owned(),
            ));
        }

        match self.database_backend()? {
            DatabaseBackend::Postgres => {
                if self.database.read_replica_max_connections == 0 {
                    return Err(SettingsError::Invalid(
                        "database.read_replica_max_connections must be > 0".to_owned(),
                    ));
                }
                if self.database.read_replica_min_connections
                    > self.database.read_replica_max_connections
                {
                    return Err(SettingsError::Invalid("database.read_replica_min_connections cannot exceed read_replica_max_connections".to_owned()));
                }
                if self.database.read_replica_urls.len() > 32 {
                    return Err(SettingsError::Invalid(
                        "at most 32 read replicas may be configured".to_owned(),
                    ));
                }
                self.pg_options()?;
                for url in &self.database.read_replica_urls {
                    self.pg_options_for_url(url)?;
                }
            }
            DatabaseBackend::Sqlite => {
                if !self.database.read_replica_urls.is_empty() {
                    return Err(SettingsError::Invalid(
                        "SQLite does not support DATABASE_READ_REPLICA_URLS_JSON".to_owned(),
                    ));
                }
                self.sqlite_options()?;
            }
            DatabaseBackend::Auto => unreachable!("database_backend resolves Auto"),
        }
        self.server_addr()?;
        Ok(())
    }

    pub fn pg_options(&self) -> Result<PgConnectOptions, SettingsError> {
        self.pg_options_for_url(self.db_url())
    }

    pub fn pg_options_for_url(&self, url: &str) -> Result<PgConnectOptions, SettingsError> {
        let mut options = PgConnectOptions::from_str(url)
            .map_err(|error| SettingsError::InvalidDatabaseUrl(error.to_string()))?;
        if self.database.require_ssl {
            options = options.ssl_mode(PgSslMode::VerifyFull);
        }
        Ok(options)
    }

    pub fn sqlite_options(&self) -> Result<SqliteConnectOptions, SettingsError> {
        if !self.database.sqlite_foreign_keys {
            return Err(SettingsError::Invalid(
                "database.sqlite_foreign_keys must be true for Lex-managed pools".into(),
            ));
        }
        let mut options = SqliteConnectOptions::from_str(self.db_url())
            .map_err(|error| SettingsError::InvalidDatabaseUrl(error.to_string()))?
            .create_if_missing(self.database.sqlite_create_if_missing)
            .foreign_keys(self.database.sqlite_foreign_keys)
            .busy_timeout(std::time::Duration::from_millis(
                self.database.sqlite_busy_timeout_ms,
            ))
            .synchronous(SqliteSynchronous::Normal);
        if self.database.sqlite_wal
            && !self
                .db_url()
                .trim()
                .to_ascii_lowercase()
                .starts_with("sqlite::memory:")
        {
            options = options.journal_mode(SqliteJournalMode::Wal);
        }
        Ok(options)
    }

    pub fn server_addr(&self) -> Result<SocketAddr, SettingsError> {
        Ok(format!("{}:{}", self.server.host, self.server.port).parse()?)
    }
}

pub fn backend_from_url(url: &str) -> Result<DatabaseBackend, SettingsError> {
    let lower = url.trim().to_ascii_lowercase();
    if lower.starts_with("postgres://") || lower.starts_with("postgresql://") {
        Ok(DatabaseBackend::Postgres)
    } else if lower.starts_with("sqlite:") {
        Ok(DatabaseBackend::Sqlite)
    } else {
        Err(SettingsError::InvalidDatabaseUrl(
            "supported schemes are postgres://, postgresql://, sqlite://, and sqlite::memory:"
                .to_owned(),
        ))
    }
}

fn load_read_replica_urls() -> Result<Vec<String>, SettingsError> {
    let Some(raw) = env::var("DATABASE_READ_REPLICA_URLS_JSON").ok() else {
        return Ok(Vec::new());
    };
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let values: Vec<String> = serde_json::from_str(&raw).map_err(|error| {
        SettingsError::Invalid(format!(
            "DATABASE_READ_REPLICA_URLS_JSON must be a JSON array of PostgreSQL URLs: {error}"
        ))
    })?;
    if values.iter().any(|value| value.trim().is_empty()) {
        return Err(SettingsError::Invalid(
            "read replica URLs cannot be empty".to_owned(),
        ));
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_configuration_cannot_disable_referential_integrity() {
        let mut settings = Settings::default();
        settings.database.url = "sqlite::memory:".into();
        settings.database.sqlite_foreign_keys = false;
        assert!(settings.sqlite_options().is_err());
        assert!(settings.validate().is_err());
    }

    #[test]
    fn backend_mismatch_does_not_disclose_credentials_in_url_parameters() {
        let mut settings = Settings::default();
        settings.database.backend = DatabaseBackend::Sqlite;
        settings.database.url =
            "postgres://alice:hidden-userinfo@private-host/db?password=hidden-query".into();
        let error = settings.database_backend().unwrap_err().to_string();
        assert!(!error.contains("hidden"));
        assert!(!error.contains("private-host"));
    }

    #[test]
    fn production_profile_is_consistent_for_case_and_whitespace() {
        for profile in ["prod", "production", "PRODUCTION", " Prod "] {
            assert!(production_profile(profile));
        }
        assert!(!production_profile("development"));
    }

    #[test]
    fn debugging_settings_does_not_disclose_database_credentials() {
        let mut settings = Settings::default();
        settings.database.url = "postgres://user:secret@internal/db".into();
        settings
            .database
            .read_replica_urls
            .push("postgres://replica:other-secret@internal/db".into());
        let rendered = format!("{settings:?}");
        assert!(!rendered.contains("secret"));
        assert!(!rendered.contains("internal"));
    }

    #[test]
    fn auto_backend_detects_sqlite_and_postgres() {
        assert_eq!(
            backend_from_url("sqlite://db.sqlite3").unwrap(),
            DatabaseBackend::Sqlite
        );
        assert_eq!(
            backend_from_url("sqlite::memory:").unwrap(),
            DatabaseBackend::Sqlite
        );
        assert_eq!(
            backend_from_url("postgresql://localhost/test").unwrap(),
            DatabaseBackend::Postgres
        );
    }

    #[test]
    fn sqlite_settings_are_valid_without_ssl_or_replicas() {
        let mut settings = Settings::default();
        settings.database.url = "sqlite::memory:".to_owned();
        settings.database.require_ssl = false;
        assert_eq!(
            settings.database_backend().unwrap(),
            DatabaseBackend::Sqlite
        );
        assert!(settings.validate().is_ok());
        assert!(settings.sqlite_options().is_ok());
    }

    #[test]
    fn rejects_backend_mismatch() {
        let mut settings = Settings::default();
        settings.database.backend = DatabaseBackend::Postgres;
        settings.database.url = "sqlite::memory:".to_owned();
        assert!(matches!(
            settings.database_backend(),
            Err(SettingsError::BackendMismatch(_))
        ));
    }

    #[test]
    fn rejects_sqlite_read_replicas() {
        let mut settings = Settings::default();
        settings.database.url = "sqlite::memory:".to_owned();
        settings.database.read_replica_urls = vec!["postgresql://localhost/replica".into()];
        assert!(settings.validate().is_err());
    }
}
