use serde::Deserialize;
use std::{env, path::PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MediaConfigError {
    #[error("invalid media configuration: {0}")]
    Invalid(String),
    #[error("required media credential environment variable {0} is missing")]
    MissingCredential(String),
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MediaStorageBackend {
    /// Process-local storage, useful for tests only. Data disappears on restart.
    Memory,
    /// Local filesystem storage. This is the default development backend.
    #[default]
    Filesystem,
    /// S3-compatible object storage (AWS S3, Backblaze B2 S3 API, MinIO, etc.).
    S3,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MalwareScannerBackend {
    #[default]
    Disabled,
    Clamav,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct S3Settings {
    /// Example Backblaze endpoint: <https://s3.us-west-004.backblazeb2.com>
    pub endpoint: Option<String>,
    pub region: Option<String>,
    pub bucket: Option<String>,
    pub public_base_url: Option<String>,
    pub force_path_style: bool,
    /// Names of environment variables containing credentials. Values are never stored in Config.toml.
    pub access_key_env: String,
    pub secret_key_env: String,
}

impl Default for S3Settings {
    fn default() -> Self {
        Self {
            endpoint: None,
            region: None,
            bucket: None,
            public_base_url: None,
            force_path_style: false,
            access_key_env: "LEX_MEDIA_S3_ACCESS_KEY_ID".to_owned(),
            secret_key_env: "LEX_MEDIA_S3_SECRET_ACCESS_KEY".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ClamAvSettings {
    pub host: String,
    pub port: u16,
    pub timeout_secs: u64,
    pub fail_closed: bool,
}

impl Default for ClamAvSettings {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 3310,
            timeout_secs: 15,
            fail_closed: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MediaSettings {
    pub enabled: bool,
    pub storage: MediaStorageBackend,
    pub local_root: PathBuf,
    pub temp_root: PathBuf,
    pub max_upload_bytes: u64,
    pub allowed_content_types: Vec<String>,
    pub scanner: MalwareScannerBackend,
    pub s3: S3Settings,
    pub clamav: ClamAvSettings,
}

impl Default for MediaSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            storage: MediaStorageBackend::Filesystem,
            local_root: PathBuf::from("./media"),
            temp_root: PathBuf::from("./.lex/uploads"),
            max_upload_bytes: 25 * 1024 * 1024,
            allowed_content_types: vec![
                "image/jpeg".into(),
                "image/png".into(),
                "image/webp".into(),
                "application/pdf".into(),
            ],
            scanner: MalwareScannerBackend::Disabled,
            s3: S3Settings::default(),
            clamav: ClamAvSettings::default(),
        }
    }
}

impl MediaSettings {
    pub fn validate(&self, production: bool) -> Result<(), MediaConfigError> {
        if self.max_upload_bytes == 0 || self.max_upload_bytes > 5 * 1024 * 1024 * 1024 {
            return Err(MediaConfigError::Invalid(
                "media.max_upload_bytes must be between 1 byte and 5 GiB".into(),
            ));
        }
        if self.allowed_content_types.len() > 128
            || self
                .allowed_content_types
                .iter()
                .any(|v| v.trim().is_empty())
        {
            return Err(MediaConfigError::Invalid(
                "media.allowed_content_types is invalid".into(),
            ));
        }
        if production && self.storage == MediaStorageBackend::Memory {
            return Err(MediaConfigError::Invalid(
                "memory media storage is test-only and forbidden in production".into(),
            ));
        }
        if self.storage == MediaStorageBackend::S3 {
            if self.s3.bucket.as_deref().unwrap_or("").trim().is_empty()
                || self.s3.region.as_deref().unwrap_or("").trim().is_empty()
            {
                return Err(MediaConfigError::Invalid(
                    "S3 media storage requires media.s3.bucket and media.s3.region".into(),
                ));
            }
            if production {
                self.s3_credentials()?;
            }
        }
        if self.scanner == MalwareScannerBackend::Clamav
            && (self.clamav.port == 0 || self.clamav.timeout_secs == 0)
        {
            return Err(MediaConfigError::Invalid(
                "ClamAV host/port/timeout configuration is invalid".into(),
            ));
        }
        Ok(())
    }

    pub fn s3_credentials(&self) -> Result<(String, String), MediaConfigError> {
        let access = env::var(&self.s3.access_key_env)
            .map_err(|_| MediaConfigError::MissingCredential(self.s3.access_key_env.clone()))?;
        let secret = env::var(&self.s3.secret_key_env)
            .map_err(|_| MediaConfigError::MissingCredential(self.s3.secret_key_env.clone()))?;
        if access.trim().is_empty() || secret.trim().is_empty() {
            return Err(MediaConfigError::Invalid(
                "S3 credentials cannot be empty".into(),
            ));
        }
        Ok((access, secret))
    }
}
