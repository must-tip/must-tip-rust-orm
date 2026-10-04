use async_trait::async_trait;
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use std::time::Duration;
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    sync::RwLock,
    time::timeout,
};

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("invalid media object key")]
    InvalidKey,
    #[error("media object not found")]
    NotFound,
    #[error("media I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("media provider error: {0}")]
    Provider(String),
}

fn validate_key(key: &str) -> Result<(), MediaError> {
    if key.is_empty()
        || key.starts_with('/')
        || key.contains("..")
        || key.contains('\\')
        || key.chars().any(char::is_control)
        || key
            .split('/')
            .any(|component| component.is_empty() || component == ".")
    {
        return Err(MediaError::InvalidKey);
    }
    Ok(())
}

#[async_trait]
pub trait MediaStorage: Send + Sync {
    async fn put(&self, key: &str, bytes: &[u8]) -> Result<(), MediaError>;
    async fn get(&self, key: &str) -> Result<Vec<u8>, MediaError>;
    async fn delete(&self, key: &str) -> Result<(), MediaError>;
}

#[derive(Default, Clone)]
pub struct MemoryStorage {
    objects: Arc<RwLock<BTreeMap<String, Vec<u8>>>>,
}

#[async_trait]
impl MediaStorage for MemoryStorage {
    async fn put(&self, key: &str, bytes: &[u8]) -> Result<(), MediaError> {
        validate_key(key)?;
        self.objects
            .write()
            .await
            .insert(key.to_owned(), bytes.to_vec());
        Ok(())
    }
    async fn get(&self, key: &str) -> Result<Vec<u8>, MediaError> {
        validate_key(key)?;
        self.objects
            .read()
            .await
            .get(key)
            .cloned()
            .ok_or(MediaError::NotFound)
    }
    async fn delete(&self, key: &str) -> Result<(), MediaError> {
        validate_key(key)?;
        self.objects.write().await.remove(key);
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct FileSystemStorage {
    root: Arc<Dir>,
}
impl FileSystemStorage {
    /// Open a storage directory as a filesystem capability. Object operations cannot follow
    /// symlinks outside this directory, including during concurrent filesystem changes.
    pub async fn new(root: impl Into<PathBuf>) -> Result<Self, MediaError> {
        let root = root.into();
        tokio::fs::create_dir_all(&root).await?;
        let root =
            tokio::task::spawn_blocking(move || Dir::open_ambient_dir(root, ambient_authority()))
                .await
                .map_err(|_| MediaError::Provider("storage worker failed".into()))??;
        Ok(Self {
            root: Arc::new(root),
        })
    }
}
#[async_trait]
impl MediaStorage for FileSystemStorage {
    async fn put(&self, key: &str, bytes: &[u8]) -> Result<(), MediaError> {
        validate_key(key)?;
        let root = Arc::clone(&self.root);
        let key = key.to_owned();
        let bytes = bytes.to_vec();
        tokio::task::spawn_blocking(move || -> Result<(), MediaError> {
            if let Some(parent) = Path::new(&key)
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
            {
                root.create_dir_all(parent)?;
            }
            // Exclusive creation prevents collisions with another upload or existing object.
            let temporary = format!(".lex-upload-{}", uuid::Uuid::new_v4().simple());
            let mut file =
                root.open_with(&temporary, OpenOptions::new().write(true).create_new(true))?;
            let result = (|| -> std::io::Result<()> {
                file.write_all(&bytes)?;
                file.sync_all()?;
                drop(file);
                root.rename(&temporary, &root, &key)
            })();
            if result.is_err() {
                let _ = root.remove_file(&temporary);
            }
            result.map_err(MediaError::from)
        })
        .await
        .map_err(|_| MediaError::Provider("storage worker failed".into()))?
    }
    async fn get(&self, key: &str) -> Result<Vec<u8>, MediaError> {
        validate_key(key)?;
        let root = Arc::clone(&self.root);
        let key = key.to_owned();
        match tokio::task::spawn_blocking(move || root.read(key))
            .await
            .map_err(|_| MediaError::Provider("storage worker failed".into()))?
        {
            Ok(bytes) => Ok(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(MediaError::NotFound),
            Err(error) => Err(error.into()),
        }
    }
    async fn delete(&self, key: &str) -> Result<(), MediaError> {
        validate_key(key)?;
        let root = Arc::clone(&self.root);
        let key = key.to_owned();
        match tokio::task::spawn_blocking(move || root.remove_file(key))
            .await
            .map_err(|_| MediaError::Provider("storage worker failed".into()))?
        {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

/// Extension point for S3-compatible providers. Implement this trait with your chosen client
/// (AWS SDK, Backblaze B2 S3 API, MinIO SDK, etc.) and inject it as `Arc<dyn MediaStorage>`.
/// This keeps provider SDK choice out of the framework core while preserving one storage API.
pub type SharedMediaStorage = Arc<dyn MediaStorage>;

#[async_trait]
pub trait MalwareScanner: Send + Sync {
    async fn scan(&self, name: &str, bytes: &[u8]) -> Result<ScanResult, MediaError>;
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanResult {
    Clean,
    Infected { signature: String },
}
#[derive(Debug, Default)]
pub struct NoopScanner;
#[async_trait]
impl MalwareScanner for NoopScanner {
    async fn scan(&self, _: &str, _: &[u8]) -> Result<ScanResult, MediaError> {
        Ok(ScanResult::Clean)
    }
}

/// ClamAV integration contract. The framework settings expose host/port/timeouts; a concrete
/// clamd transport can implement this trait without coupling application code to that transport.
#[derive(Debug, Clone)]
pub struct ClamAvScanner {
    host: String,
    port: u16,
    timeout: Duration,
}
impl ClamAvScanner {
    pub fn new(host: impl Into<String>, port: u16, timeout_secs: u64) -> Result<Self, MediaError> {
        if port == 0 || timeout_secs == 0 {
            return Err(MediaError::Provider(
                "invalid ClamAV connection settings".into(),
            ));
        }
        Ok(Self {
            host: host.into(),
            port,
            timeout: Duration::from_secs(timeout_secs),
        })
    }
}
#[async_trait]
impl MalwareScanner for ClamAvScanner {
    async fn scan(&self, _name: &str, bytes: &[u8]) -> Result<ScanResult, MediaError> {
        let fut = async {
            let mut stream = TcpStream::connect((self.host.as_str(), self.port))
                .await
                .map_err(|e| MediaError::Provider(format!("cannot connect to clamd: {e}")))?;
            stream
                .write_all(b"zINSTREAM\0")
                .await
                .map_err(|e| MediaError::Provider(format!("clamd command failed: {e}")))?;
            for chunk in bytes.chunks(64 * 1024) {
                let n = (chunk.len() as u32).to_be_bytes();
                stream
                    .write_all(&n)
                    .await
                    .map_err(|e| MediaError::Provider(e.to_string()))?;
                stream
                    .write_all(chunk)
                    .await
                    .map_err(|e| MediaError::Provider(e.to_string()))?;
            }
            stream
                .write_all(&0u32.to_be_bytes())
                .await
                .map_err(|e| MediaError::Provider(e.to_string()))?;
            stream
                .flush()
                .await
                .map_err(|e| MediaError::Provider(e.to_string()))?;
            // clamd terminates responses with NUL, not connection closure. Bound the read so
            // a broken provider cannot make an upload allocate an unbounded response buffer.
            let mut response = Vec::with_capacity(256);
            let mut reader = BufReader::new(stream).take(4097);
            reader
                .read_until(0, &mut response)
                .await
                .map_err(|e| MediaError::Provider(e.to_string()))?;
            if response.len() > 4096 || response.last() != Some(&0) {
                return Err(MediaError::Provider(
                    "invalid or oversized clamd response".into(),
                ));
            }
            let text = std::str::from_utf8(&response[..response.len() - 1])
                .map_err(|_| MediaError::Provider("invalid clamd response encoding".into()))?;
            if text == "stream: OK" {
                return Ok(ScanResult::Clean);
            }
            if let Some(signature) = text
                .strip_prefix("stream: ")
                .and_then(|value| value.strip_suffix(" FOUND"))
                && !signature.is_empty()
            {
                return Ok(ScanResult::Infected {
                    signature: signature.into(),
                });
            }
            Err(MediaError::Provider(format!(
                "unexpected clamd response: {text}"
            )))
        };
        timeout(self.timeout, fut)
            .await
            .map_err(|_| MediaError::Provider("ClamAV scan timed out".into()))?
    }
}

pub trait ClamAvTransport: MalwareScanner {}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn memory_round_trip() {
        let s = MemoryStorage::default();
        s.put("users/1/avatar.png", b"abc").await.unwrap();
        assert_eq!(s.get("users/1/avatar.png").await.unwrap(), b"abc");
    }
    #[tokio::test]
    async fn filesystem_round_trip() {
        let d = std::env::temp_dir().join(format!("lex-media-{}", std::process::id()));
        let s = FileSystemStorage::new(&d).await.unwrap();
        s.put("x/a.txt", b"hello").await.unwrap();
        assert_eq!(s.get("x/a.txt").await.unwrap(), b"hello");
        let _ = tokio::fs::remove_dir_all(d).await;
    }
    #[test]
    fn rejects_traversal() {
        assert!(validate_key("../secret").is_err());
        assert!(validate_key("/absolute").is_err());
    }
}
