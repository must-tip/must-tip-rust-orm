use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use db::{Database, DatabasePool};
use hmac::{Hmac, KeyInit, Mac};
use lex_rest::rest_framework::{
    authentication::{
        BearerTokenResolver, CookieIdentity, CookiePrincipalResolver, SessionIdentity, SessionStore,
    },
    exceptions::{ApiError, ApiResult},
    request::{ApiRequest, Principal},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::{
    env,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

use crate::{AuthenticationSettings, CookieAuthSettings};

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub(crate) struct DatabaseAuthenticationStore {
    pool: DatabasePool,
}

impl DatabaseAuthenticationStore {
    pub async fn initialize(database: &Database) -> ApiResult<Arc<Self>> {
        let store = Arc::new(Self {
            pool: database.pool().clone(),
        });
        store.ensure_schema().await?;
        Ok(store)
    }

    async fn ensure_schema(&self) -> ApiResult<()> {
        let sessions = r#"
            CREATE TABLE IF NOT EXISTS _lex_auth_sessions (
                token_hash TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                scopes_json TEXT NOT NULL,
                csrf_secret TEXT,
                expires_at_epoch BIGINT NOT NULL,
                revoked INTEGER NOT NULL DEFAULT 0,
                created_at_epoch BIGINT NOT NULL
            )
        "#;
        let tokens = r#"
            CREATE TABLE IF NOT EXISTS _lex_auth_tokens (
                token_hash TEXT PRIMARY KEY,
                token_id TEXT NOT NULL UNIQUE,
                owner_user_id TEXT NOT NULL,
                scopes_json TEXT NOT NULL,
                expires_at_epoch BIGINT NOT NULL,
                revoked INTEGER NOT NULL DEFAULT 0,
                created_at_epoch BIGINT NOT NULL
            )
        "#;
        match &self.pool {
            DatabasePool::Postgres(pool) => {
                sqlx::query(sessions)
                    .execute(pool)
                    .await
                    .map_err(db_error)?;
                sqlx::query(tokens).execute(pool).await.map_err(db_error)?;
            }
            DatabasePool::Sqlite(pool) => {
                sqlx::query(sessions)
                    .execute(pool)
                    .await
                    .map_err(db_error)?;
                sqlx::query(tokens).execute(pool).await.map_err(db_error)?;
            }
        }
        Ok(())
    }

    pub async fn create_session(
        &self,
        user_id: Uuid,
        scopes: impl IntoIterator<Item = String>,
        ttl_secs: u64,
    ) -> ApiResult<IssuedSession> {
        let token = random_credential("sess");
        let csrf_token = random_credential("csrf");
        let hash = sha256_hex(&token);
        let scopes = normalize_scopes(scopes)?;
        let scopes_json =
            serde_json::to_string(&scopes).map_err(|e| ApiError::internal(e.to_string()))?;
        let now = now_epoch()?;
        let expires = expiry(now, ttl_secs)?;
        match &self.pool {
            DatabasePool::Postgres(pool) => {
                sqlx::query("INSERT INTO _lex_auth_sessions (token_hash,user_id,scopes_json,csrf_secret,expires_at_epoch,revoked,created_at_epoch) VALUES ($1,$2,$3,$4,$5,0,$6)")
                    .bind(&hash).bind(user_id.to_string()).bind(&scopes_json).bind(&csrf_token).bind(expires).bind(now)
                    .execute(pool).await.map_err(db_error)?;
            }
            DatabasePool::Sqlite(pool) => {
                sqlx::query("INSERT INTO _lex_auth_sessions (token_hash,user_id,scopes_json,csrf_secret,expires_at_epoch,revoked,created_at_epoch) VALUES (?1,?2,?3,?4,?5,0,?6)")
                    .bind(&hash).bind(user_id.to_string()).bind(&scopes_json).bind(&csrf_token).bind(expires).bind(now)
                    .execute(pool).await.map_err(db_error)?;
            }
        }
        Ok(IssuedSession {
            token,
            csrf_token,
            expires_at_epoch: expires,
        })
    }

    pub async fn revoke_session(&self, token: &str) -> ApiResult<()> {
        let hash = sha256_hex(token);
        match &self.pool {
            DatabasePool::Postgres(pool) => {
                sqlx::query("UPDATE _lex_auth_sessions SET revoked=1 WHERE token_hash=$1")
                    .bind(hash)
                    .execute(pool)
                    .await
                    .map_err(db_error)?;
            }
            DatabasePool::Sqlite(pool) => {
                sqlx::query("UPDATE _lex_auth_sessions SET revoked=1 WHERE token_hash=?1")
                    .bind(hash)
                    .execute(pool)
                    .await
                    .map_err(db_error)?;
            }
        }
        Ok(())
    }

    pub async fn create_bearer_token(
        &self,
        user_id: Uuid,
        scopes: impl IntoIterator<Item = String>,
        ttl_secs: u64,
    ) -> ApiResult<IssuedBearerToken> {
        let token = random_credential("mtk");
        let token_id = Uuid::now_v7();
        let hash = sha256_hex(&token);
        let scopes = normalize_scopes(scopes)?;
        let scopes_json =
            serde_json::to_string(&scopes).map_err(|e| ApiError::internal(e.to_string()))?;
        let now = now_epoch()?;
        let expires = expiry(now, ttl_secs)?;
        match &self.pool {
            DatabasePool::Postgres(pool) => {
                sqlx::query("INSERT INTO _lex_auth_tokens (token_hash,token_id,owner_user_id,scopes_json,expires_at_epoch,revoked,created_at_epoch) VALUES ($1,$2,$3,$4,$5,0,$6)")
                    .bind(&hash).bind(token_id.to_string()).bind(user_id.to_string()).bind(&scopes_json).bind(expires).bind(now)
                    .execute(pool).await.map_err(db_error)?;
            }
            DatabasePool::Sqlite(pool) => {
                sqlx::query("INSERT INTO _lex_auth_tokens (token_hash,token_id,owner_user_id,scopes_json,expires_at_epoch,revoked,created_at_epoch) VALUES (?1,?2,?3,?4,?5,0,?6)")
                    .bind(&hash).bind(token_id.to_string()).bind(user_id.to_string()).bind(&scopes_json).bind(expires).bind(now)
                    .execute(pool).await.map_err(db_error)?;
            }
        }
        Ok(IssuedBearerToken {
            token,
            token_id,
            expires_at_epoch: expires,
        })
    }

    pub async fn revoke_bearer_token(&self, token: &str) -> ApiResult<()> {
        let hash = sha256_hex(token);
        match &self.pool {
            DatabasePool::Postgres(pool) => {
                sqlx::query("UPDATE _lex_auth_tokens SET revoked=1 WHERE token_hash=$1")
                    .bind(hash)
                    .execute(pool)
                    .await
                    .map_err(db_error)?;
            }
            DatabasePool::Sqlite(pool) => {
                sqlx::query("UPDATE _lex_auth_tokens SET revoked=1 WHERE token_hash=?1")
                    .bind(hash)
                    .execute(pool)
                    .await
                    .map_err(db_error)?;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl SessionStore for DatabaseAuthenticationStore {
    async fn resolve_session(
        &self,
        session_id: &str,
        _request: &ApiRequest,
    ) -> ApiResult<Option<SessionIdentity>> {
        let hash = sha256_hex(session_id);
        let now = now_epoch()?;
        let record: Option<(String, String, Option<String>, i64, i32)> = match &self.pool {
            DatabasePool::Postgres(pool) => {
                let row = sqlx::query("SELECT user_id,scopes_json,csrf_secret,expires_at_epoch,revoked FROM _lex_auth_sessions WHERE token_hash=$1").bind(hash).fetch_optional(pool).await.map_err(db_error)?;
                row.map(|row| -> ApiResult<_> {
                    Ok((
                        row.try_get("user_id").map_err(db_error)?,
                        row.try_get("scopes_json").map_err(db_error)?,
                        row.try_get("csrf_secret").map_err(db_error)?,
                        row.try_get("expires_at_epoch").map_err(db_error)?,
                        row.try_get("revoked").map_err(db_error)?,
                    ))
                })
                .transpose()?
            }
            DatabasePool::Sqlite(pool) => {
                let row = sqlx::query("SELECT user_id,scopes_json,csrf_secret,expires_at_epoch,revoked FROM _lex_auth_sessions WHERE token_hash=?1").bind(hash).fetch_optional(pool).await.map_err(db_error)?;
                row.map(|row| -> ApiResult<_> {
                    Ok((
                        row.try_get("user_id").map_err(db_error)?,
                        row.try_get("scopes_json").map_err(db_error)?,
                        row.try_get("csrf_secret").map_err(db_error)?,
                        row.try_get("expires_at_epoch").map_err(db_error)?,
                        row.try_get("revoked").map_err(db_error)?,
                    ))
                })
                .transpose()?
            }
        };
        let Some((user_id, scopes_json, csrf_secret, expires, revoked)) = record else {
            return Ok(None);
        };
        if revoked != 0 || expires <= now {
            return Ok(None);
        }
        let user_id = parse_uuid(user_id)?;
        let scopes = parse_scopes(scopes_json)?;
        Ok(Some(SessionIdentity {
            principal: Principal::user(user_id, "session", scopes)?,
            csrf_secret,
        }))
    }
}

#[async_trait]
impl BearerTokenResolver for DatabaseAuthenticationStore {
    async fn resolve_bearer_token(
        &self,
        token: &str,
        _request: &ApiRequest,
    ) -> ApiResult<Option<Principal>> {
        let hash = sha256_hex(token);
        let now = now_epoch()?;
        let record: Option<(String, String, String, i64, i32)> = match &self.pool {
            DatabasePool::Postgres(pool) => {
                let row = sqlx::query("SELECT token_id,owner_user_id,scopes_json,expires_at_epoch,revoked FROM _lex_auth_tokens WHERE token_hash=$1").bind(hash).fetch_optional(pool).await.map_err(db_error)?;
                row.map(|row| -> ApiResult<_> {
                    Ok((
                        row.try_get("token_id").map_err(db_error)?,
                        row.try_get("owner_user_id").map_err(db_error)?,
                        row.try_get("scopes_json").map_err(db_error)?,
                        row.try_get("expires_at_epoch").map_err(db_error)?,
                        row.try_get("revoked").map_err(db_error)?,
                    ))
                })
                .transpose()?
            }
            DatabasePool::Sqlite(pool) => {
                let row = sqlx::query("SELECT token_id,owner_user_id,scopes_json,expires_at_epoch,revoked FROM _lex_auth_tokens WHERE token_hash=?1").bind(hash).fetch_optional(pool).await.map_err(db_error)?;
                row.map(|row| -> ApiResult<_> {
                    Ok((
                        row.try_get("token_id").map_err(db_error)?,
                        row.try_get("owner_user_id").map_err(db_error)?,
                        row.try_get("scopes_json").map_err(db_error)?,
                        row.try_get("expires_at_epoch").map_err(db_error)?,
                        row.try_get("revoked").map_err(db_error)?,
                    ))
                })
                .transpose()?
            }
        };
        let Some((token_id, owner_user_id, scopes_json, expires, revoked)) = record else {
            return Ok(None);
        };
        if revoked != 0 || expires <= now {
            return Ok(None);
        }
        let token_id = parse_uuid(token_id)?;
        let owner = parse_uuid(owner_user_id)?;
        let scopes = parse_scopes(scopes_json)?;
        Ok(Some(Principal::api_key(owner, token_id, "bearer", scopes)?))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SignedCookiePayload {
    sub: String,
    exp: i64,
    scopes: Vec<String>,
    csrf: String,
}

#[derive(Clone)]
pub(crate) struct SignedCookieResolver {
    key: Arc<Vec<u8>>,
}

impl SignedCookieResolver {
    pub fn from_settings(settings: &CookieAuthSettings) -> ApiResult<Self> {
        let key = env::var(&settings.secret_env).map_err(|_| ApiError::configuration("cookie authentication is enabled but its signing secret environment variable is missing"))?;
        if key.len() < 32 {
            return Err(ApiError::configuration(
                "cookie authentication signing secret must be at least 32 bytes",
            ));
        }
        Ok(Self {
            key: Arc::new(key.into_bytes()),
        })
    }

    pub fn issue(
        &self,
        user_id: Uuid,
        scopes: impl IntoIterator<Item = String>,
        ttl_secs: u64,
    ) -> ApiResult<IssuedCookie> {
        let scopes = normalize_scopes(scopes)?;
        let now = now_epoch()?;
        let exp = expiry(now, ttl_secs)?;
        let csrf_token = random_credential("csrf");
        let payload = SignedCookiePayload {
            sub: user_id.to_string(),
            exp,
            scopes,
            csrf: csrf_token.clone(),
        };
        let raw = serde_json::to_vec(&payload).map_err(|e| ApiError::internal(e.to_string()))?;
        let encoded = URL_SAFE_NO_PAD.encode(raw);
        let mut mac = HmacSha256::new_from_slice(self.key.as_slice())
            .map_err(|_| ApiError::configuration("invalid cookie signing key"))?;
        mac.update(encoded.as_bytes());
        let sig = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        Ok(IssuedCookie {
            value: format!("{encoded}.{sig}"),
            csrf_token,
            expires_at_epoch: exp,
        })
    }
}

#[async_trait]
impl CookiePrincipalResolver for SignedCookieResolver {
    async fn resolve_cookie(
        &self,
        cookie_value: &str,
        _request: &ApiRequest,
    ) -> ApiResult<Option<CookieIdentity>> {
        let Some((payload, signature)) = cookie_value.split_once('.') else {
            return Ok(None);
        };
        let sig = URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| ApiError::authentication("invalid authentication cookie"))?;
        let mut mac = HmacSha256::new_from_slice(self.key.as_slice())
            .map_err(|_| ApiError::configuration("invalid cookie signing key"))?;
        mac.update(payload.as_bytes());
        if mac.verify_slice(&sig).is_err() {
            return Ok(None);
        }
        let raw = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| ApiError::authentication("invalid authentication cookie"))?;
        let data: SignedCookiePayload = serde_json::from_slice(&raw)
            .map_err(|_| ApiError::authentication("invalid authentication cookie"))?;
        if data.exp <= now_epoch()? {
            return Ok(None);
        }
        let user_id = parse_uuid(data.sub)?;
        Ok(Some(CookieIdentity {
            principal: Principal::user(user_id, "cookie", data.scopes)?,
            csrf_secret: Some(data.csrf),
        }))
    }
}

#[derive(Clone)]
pub struct FrameworkAuthService {
    database: Option<Arc<DatabaseAuthenticationStore>>,
    cookie: Option<Arc<SignedCookieResolver>>,
    settings: AuthenticationSettings,
}

impl FrameworkAuthService {
    pub(crate) fn new(
        database: Option<Arc<DatabaseAuthenticationStore>>,
        cookie: Option<Arc<SignedCookieResolver>>,
        settings: AuthenticationSettings,
    ) -> Self {
        Self {
            database,
            cookie,
            settings,
        }
    }

    pub async fn create_session(
        &self,
        user_id: Uuid,
        scopes: impl IntoIterator<Item = String>,
    ) -> ApiResult<IssuedSession> {
        if !self
            .settings
            .backends
            .iter()
            .any(|backend| backend.trim().eq_ignore_ascii_case("session"))
        {
            return Err(ApiError::configuration(
                "session authentication is not enabled",
            ));
        }
        let store = self
            .database
            .as_ref()
            .ok_or_else(|| ApiError::configuration("session authentication is not enabled"))?;
        store
            .create_session(user_id, scopes, self.settings.session.ttl_secs)
            .await
    }
    pub async fn revoke_session(&self, token: &str) -> ApiResult<()> {
        if !self
            .settings
            .backends
            .iter()
            .any(|backend| backend.trim().eq_ignore_ascii_case("session"))
        {
            return Err(ApiError::configuration(
                "session authentication is not enabled",
            ));
        }
        let store = self
            .database
            .as_ref()
            .ok_or_else(|| ApiError::configuration("session authentication is not enabled"))?;
        store.revoke_session(token).await
    }
    pub async fn create_bearer_token(
        &self,
        user_id: Uuid,
        scopes: impl IntoIterator<Item = String>,
    ) -> ApiResult<IssuedBearerToken> {
        if !self
            .settings
            .backends
            .iter()
            .any(|backend| backend.trim().eq_ignore_ascii_case("bearer"))
        {
            return Err(ApiError::configuration(
                "bearer authentication is not enabled",
            ));
        }
        let store = self
            .database
            .as_ref()
            .ok_or_else(|| ApiError::configuration("bearer authentication is not enabled"))?;
        store
            .create_bearer_token(user_id, scopes, self.settings.bearer.ttl_secs)
            .await
    }
    pub async fn revoke_bearer_token(&self, token: &str) -> ApiResult<()> {
        if !self
            .settings
            .backends
            .iter()
            .any(|backend| backend.trim().eq_ignore_ascii_case("bearer"))
        {
            return Err(ApiError::configuration(
                "bearer authentication is not enabled",
            ));
        }
        let store = self
            .database
            .as_ref()
            .ok_or_else(|| ApiError::configuration("bearer authentication is not enabled"))?;
        store.revoke_bearer_token(token).await
    }
    pub fn create_cookie(
        &self,
        user_id: Uuid,
        scopes: impl IntoIterator<Item = String>,
    ) -> ApiResult<IssuedCookie> {
        let resolver = self
            .cookie
            .as_ref()
            .ok_or_else(|| ApiError::configuration("cookie authentication is not enabled"))?;
        resolver.issue(user_id, scopes, self.settings.cookie.ttl_secs)
    }
    pub fn session_cookie(&self, issued: &IssuedSession, secure: bool) -> String {
        build_cookie(
            &self.settings.session.cookie_name,
            &issued.token,
            issued.expires_at_epoch,
            secure,
            true,
        )
    }
    pub fn auth_cookie(&self, issued: &IssuedCookie, secure: bool) -> String {
        build_cookie(
            &self.settings.cookie.cookie_name,
            &issued.value,
            issued.expires_at_epoch,
            secure,
            true,
        )
    }
}

#[derive(Clone)]
pub struct IssuedSession {
    pub token: String,
    pub csrf_token: String,
    pub expires_at_epoch: i64,
}
#[derive(Clone)]
pub struct IssuedBearerToken {
    pub token: String,
    pub token_id: Uuid,
    pub expires_at_epoch: i64,
}
#[derive(Clone)]
pub struct IssuedCookie {
    pub value: String,
    pub csrf_token: String,
    pub expires_at_epoch: i64,
}

macro_rules! redacted_debug {
    ($type:ty) => {
        impl std::fmt::Debug for $type {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter
                    .debug_struct(stringify!($type))
                    .field("credentials", &"[REDACTED]")
                    .field("expires_at_epoch", &self.expires_at_epoch)
                    .finish()
            }
        }
    };
}
redacted_debug!(IssuedSession);
redacted_debug!(IssuedBearerToken);
redacted_debug!(IssuedCookie);

fn expiry(now: i64, ttl_secs: u64) -> ApiResult<i64> {
    let ttl = i64::try_from(ttl_secs)
        .map_err(|_| ApiError::configuration("authentication TTL overflow"))?;
    if ttl == 0 {
        return Err(ApiError::configuration(
            "authentication TTL must be positive",
        ));
    }
    now.checked_add(ttl)
        .ok_or_else(|| ApiError::configuration("authentication TTL overflow"))
}

fn build_cookie(name: &str, value: &str, expires: i64, secure: bool, http_only: bool) -> String {
    let mut out = format!(
        "{name}={value}; Path=/; SameSite=Lax; Max-Age={}",
        (expires - now_epoch().unwrap_or(expires)).max(0)
    );
    if secure {
        out.push_str("; Secure");
    }
    if http_only {
        out.push_str("; HttpOnly");
    }
    out
}

fn normalize_scopes(scopes: impl IntoIterator<Item = String>) -> ApiResult<Vec<String>> {
    let mut out = Vec::new();
    for value in scopes {
        let value = value.trim();
        if value.is_empty() || value.len() > 255 || value.chars().any(char::is_whitespace) {
            return Err(ApiError::authentication("invalid authentication scope"));
        }
        if out.len() >= 512 {
            return Err(ApiError::authentication("too many authentication scopes"));
        }
        if !out.iter().any(|v| v == value) {
            out.push(value.to_owned());
        }
    }
    Ok(out)
}
fn random_credential(prefix: &str) -> String {
    format!(
        "{prefix}_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}
fn sha256_hex(value: &str) -> String {
    let bytes = Sha256::digest(value.as_bytes());
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn now_epoch() -> ApiResult<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs() as i64)
        .map_err(|_| ApiError::internal("system clock is before UNIX epoch"))
}
fn parse_uuid(value: String) -> ApiResult<Uuid> {
    Uuid::parse_str(&value)
        .map_err(|_| ApiError::internal("stored authentication identifier is invalid"))
}
fn parse_scopes(value: String) -> ApiResult<Vec<String>> {
    serde_json::from_str(&value)
        .map_err(|_| ApiError::internal("stored authentication scopes are invalid"))
}
fn db_error<E: std::fmt::Display>(error: E) -> ApiError {
    ApiError::internal(format!("authentication storage error: {error}"))
}

#[cfg(test)]
mod security_tests {
    use super::*;
    use http::{HeaderMap, Method, Uri};
    use lex_rest::rest_framework::request::TransportSecurity;

    fn request() -> ApiRequest {
        ApiRequest::new(
            Method::GET,
            Uri::from_static("/private"),
            HeaderMap::new(),
            vec![],
            None,
            TransportSecurity::Secure,
            1024,
        )
        .unwrap()
    }
    #[test]
    fn credential_lifetimes_reject_zero_and_overflow() {
        assert!(expiry(1, 0).is_err());
        assert!(expiry(1, u64::MAX).is_err());
        assert!(expiry(i64::MAX, 1).is_err());
        assert_eq!(expiry(100, 60).unwrap(), 160);
    }
    #[tokio::test]
    async fn tampered_signed_cookies_and_wrong_secrets_never_produce_an_identity() {
        let resolver = SignedCookieResolver {
            key: Arc::new(vec![17; 32]),
        };
        let wrong = SignedCookieResolver {
            key: Arc::new(vec![18; 32]),
        };
        let user = Uuid::new_v4();
        let cookie = resolver.issue(user, ["projects.read".into()], 60).unwrap();
        assert_eq!(
            resolver
                .resolve_cookie(&cookie.value, &request())
                .await
                .unwrap()
                .unwrap()
                .principal
                .user_id(),
            Some(user)
        );
        assert!(
            wrong
                .resolve_cookie(&cookie.value, &request())
                .await
                .unwrap()
                .is_none()
        );
        let (payload, signature) = cookie.value.split_once('.').unwrap();
        let mut data: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
        for field in ["sub", "scopes", "exp"] {
            let original = data[field].clone();
            data[field] = match field {
                "sub" => serde_json::json!(Uuid::new_v4()),
                "scopes" => serde_json::json!(["global:admin"]),
                _ => serde_json::json!(i64::MAX),
            };
            let tampered = format!(
                "{}.{}",
                URL_SAFE_NO_PAD.encode(serde_json::to_vec(&data).unwrap()),
                signature
            );
            assert!(
                resolver
                    .resolve_cookie(&tampered, &request())
                    .await
                    .unwrap()
                    .is_none()
            );
            data[field] = original;
        }
        for value in ["", ".", "invalid.invalid", "%%%%.%%%%"] {
            assert!(!matches!(
                resolver.resolve_cookie(value, &request()).await,
                Ok(Some(_))
            ));
        }
        let mut expired: SignedCookiePayload =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
        expired.exp = 0;
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&expired).unwrap());
        let mut mac = HmacSha256::new_from_slice(&resolver.key).unwrap();
        mac.update(payload.as_bytes());
        let value = format!(
            "{payload}.{}",
            URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
        );
        assert!(
            resolver
                .resolve_cookie(&value, &request())
                .await
                .unwrap()
                .is_none()
        );
        assert!(!format!("{cookie:?}").contains(&cookie.value));
        assert!(!format!("{cookie:?}").contains(&cookie.csrf_token));
    }
}
