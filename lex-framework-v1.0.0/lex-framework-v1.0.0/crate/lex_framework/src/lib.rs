mod auth_runtime;

use anyhow::{Context, Result, bail};
use config::{Config, Environment, File};
use lex_rest::rest_framework::{
    authentication::{
        AuthenticationBackend, AuthenticationChain, BearerTokenAuthentication,
        BearerTokenAuthenticationConfig, CookieAuthentication, CookieAuthenticationConfig,
        SessionAuthentication, SessionAuthenticationConfig,
    },
    exceptions::ApiResult,
    middleware::{
        CorsMiddleware, RequestLimitsMiddleware, RequestMiddleware, SecurityHeadersMiddleware,
    },
    permissions::{
        DatabasePermissionBackend, PermissionBackend, ScopePermissionBackend, ScopePermissionConfig,
    },
};
use media::MediaSettings;
use serde::Deserialize;
use std::{collections::BTreeSet, sync::Arc};

pub use auth_runtime::{FrameworkAuthService, IssuedBearerToken, IssuedCookie, IssuedSession};
pub use db;
pub use db_controller;
pub use lex_rest;
pub use media;
pub use migration;
/// Built-in identities and normalized tenant RBAC entities.
pub use models;
pub use orm;
pub use secure_input;

pub mod views {
    pub use lex_rest::rest_framework::views::{APIView, ViewAccess, ViewPermissions, ViewServices};
    pub use lex_rest::rest_framework::{
        exceptions::{ApiError, ApiResult},
        request::{ApiRequest, RequestContext},
        response::ApiResponse,
    };
}

pub mod authentication {
    pub use crate::{FrameworkAuthService, IssuedBearerToken, IssuedCookie, IssuedSession};
    pub use lex_rest::rest_framework::request::{Principal, PrincipalKind};
}

pub mod permissions {
    pub use lex_rest::rest_framework::permissions::{
        DatabasePermissionBackend, DatabasePermissionConfig, PermissionBackend, PermissionDecision,
        PermissionExpr, PermissionResource, Permissions, ScopePermissionBackend,
        ScopePermissionConfig,
    };
    pub use lex_rest::rest_framework::views::ViewPermissions;
}

pub mod middleware {
    pub use lex_rest::rest_framework::middleware::{
        CorsMiddleware, RequestLimitsMiddleware, RequestMiddleware,
        RequestMiddleware as Middleware, SecurityHeadersMiddleware,
    };
}

pub mod prelude {
    pub use db_controller::{Entity, QuerySet};
    pub use lex_rest::prelude::*;
    pub use media::{
        ClamAvScanner, FileSystemStorage, MalwareScanner, MediaSettings, MediaStorage,
        MemoryStorage,
    };
    pub use orm::Model;

    pub use crate::{
        AuthenticationSettings, FrameworkAuthService, FrameworkSettings, IssuedBearerToken,
        IssuedCookie, IssuedSession,
    };
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CorsSettings {
    pub enabled: bool,
    pub allowed_origins: Vec<String>,
    pub allowed_methods: Vec<String>,
    pub allowed_headers: Vec<String>,
    pub expose_headers: Vec<String>,
    pub allow_credentials: bool,
    pub max_age_secs: u64,
}

impl Default for CorsSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            allowed_origins: vec!["http://localhost:3000".into()],
            allowed_methods: vec![
                "GET".into(),
                "POST".into(),
                "PUT".into(),
                "PATCH".into(),
                "DELETE".into(),
                "OPTIONS".into(),
            ],
            allowed_headers: vec![
                "Authorization".into(),
                "Content-Type".into(),
                "Idempotency-Key".into(),
                "X-Request-ID".into(),
                "X-CSRF-Token".into(),
            ],
            expose_headers: vec!["X-Request-ID".into()],
            allow_credentials: false,
            max_age_secs: 600,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct HeaderSecuritySettings {
    pub enabled: bool,
    pub csp: String,
    pub permissions_policy: String,
    pub hsts: Option<String>,
}

impl Default for HeaderSecuritySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            csp: "default-src 'none'; frame-ancestors 'none'; base-uri 'none'".into(),
            permissions_policy: "camera=(), microphone=(), geolocation=()".into(),
            hsts: Some("max-age=31536000; includeSubDomains".into()),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RequestLimitSettings {
    pub max_uri_bytes: usize,
    pub max_header_count: usize,
    pub max_header_bytes: usize,
    pub max_single_header_bytes: usize,
}

impl Default for RequestLimitSettings {
    fn default() -> Self {
        Self {
            max_uri_bytes: 8192,
            max_header_count: 100,
            max_header_bytes: 65536,
            max_single_header_bytes: 16384,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SessionAuthSettings {
    pub cookie_name: String,
    pub require_secure_transport: bool,
    pub csrf_protection: bool,
    pub csrf_header: String,
    pub ttl_secs: u64,
}

impl Default for SessionAuthSettings {
    fn default() -> Self {
        Self {
            cookie_name: "lex_session".into(),
            require_secure_transport: false,
            csrf_protection: true,
            csrf_header: "x-csrf-token".into(),
            ttl_secs: 86400,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CookieAuthSettings {
    pub cookie_name: String,
    pub require_secure_transport: bool,
    pub csrf_protection: bool,
    pub csrf_header: String,
    pub ttl_secs: u64,
    pub secret_env: String,
}

impl Default for CookieAuthSettings {
    fn default() -> Self {
        Self {
            cookie_name: "lex_auth".into(),
            require_secure_transport: false,
            csrf_protection: true,
            csrf_header: "x-csrf-token".into(),
            ttl_secs: 86400,
            secret_env: "LEX_AUTH_COOKIE_SECRET".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct BearerAuthSettings {
    pub scheme: String,
    pub ttl_secs: u64,
}

impl Default for BearerAuthSettings {
    fn default() -> Self {
        Self {
            scheme: "Bearer".into(),
            ttl_secs: 2592000,
        }
    }
}

/// Framework-owned authentication configuration. The settings select mechanisms and ordering;
/// Lex constructs the corresponding session, signed-cookie, and bearer-token backends
/// automatically from the active database and runtime secrets.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct AuthenticationSettings {
    /// Ordered backend names. Supported values: `session`, `cookie`, `bearer`.
    /// An empty list means requests remain anonymous until a view's permission policy decides.
    pub backends: Vec<String>,
    pub session: SessionAuthSettings,
    pub cookie: CookieAuthSettings,
    pub bearer: BearerAuthSettings,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PermissionSettings {
    /// `scopes` is zero-setup and maps named permissions to session/token scopes.
    /// `database` uses normalized tenant RBAC on PostgreSQL or SQLite.
    /// Install the matching `migration::rbac` schema before enabling it.
    pub backend: String,
    pub global_admin_scope: String,
    pub tenant_member_scope: String,
    pub tenant_admin_scope: String,
}

impl Default for PermissionSettings {
    fn default() -> Self {
        Self {
            backend: "scopes".into(),
            global_admin_scope: "admin:global".into(),
            tenant_member_scope: "tenant:member".into(),
            tenant_admin_scope: "tenant:admin".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct SecuritySettings {
    pub cors: CorsSettings,
    pub headers: HeaderSecuritySettings,
    pub request_limits: RequestLimitSettings,
    pub authentication: AuthenticationSettings,
    pub permissions: PermissionSettings,
    pub trusted_proxy_hops: usize,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct FeatureSettings {
    pub media: MediaSettings,
    pub security: SecuritySettings,
}

#[derive(Debug, Clone, Default)]
pub struct FrameworkSettings {
    pub core: db::Settings,
    pub media: MediaSettings,
    pub security: SecuritySettings,
}

impl FrameworkSettings {
    pub fn load() -> Result<Self> {
        let core = db::Settings::load(None)?;
        let features: FeatureSettings = Config::builder()
            .add_source(File::with_name("Config").required(false))
            .add_source(Environment::with_prefix("APP").separator("__"))
            .build()?
            .try_deserialize()?;
        let production = is_production();
        features
            .media
            .validate(production)
            .context("invalid media settings")?;
        validate_security(&features.security, production)?;
        Ok(Self {
            core,
            media: features.media,
            security: features.security,
        })
    }

    /// Build the permission backend selected in `Config.toml`. Most applications
    /// can use `scopes` and never wire permission infrastructure manually.
    pub fn permission_backend(
        &self,
        database: &db::Database,
    ) -> Result<Arc<dyn PermissionBackend>> {
        match self
            .security
            .permissions
            .backend
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "scopes" => Ok(Arc::new(ScopePermissionBackend::new(
                ScopePermissionConfig {
                    global_admin_scope: self.security.permissions.global_admin_scope.clone(),
                    tenant_member_scope: self.security.permissions.tenant_member_scope.clone(),
                    tenant_admin_scope: self.security.permissions.tenant_admin_scope.clone(),
                },
            ))),
            "database" => {
                use lex_rest::rest_framework::permissions::{
                    DatabasePermissionConfig, RbacPermissionRepository,
                };
                let repository = match database.pool() {
                    db::DatabasePool::Postgres(pool) => {
                        RbacPermissionRepository::postgres(pool.clone())
                    }
                    db::DatabasePool::Sqlite(pool) => {
                        RbacPermissionRepository::sqlite(pool.clone())
                    }
                };
                Ok(Arc::new(DatabasePermissionBackend::new(
                    Arc::new(repository),
                    DatabasePermissionConfig::default(),
                )))
            }
            other => {
                bail!("unsupported permission backend '{other}'; expected 'scopes' or 'database'")
            }
        }
    }

    pub fn rest_middleware(&self) -> ApiResult<Vec<Arc<dyn RequestMiddleware>>> {
        let mut out: Vec<Arc<dyn RequestMiddleware>> = Vec::new();
        let limits = &self.security.request_limits;
        out.push(Arc::new(RequestLimitsMiddleware::new(
            limits.max_uri_bytes,
            limits.max_header_count,
            limits.max_header_bytes,
            limits.max_single_header_bytes,
        )?));
        if self.security.headers.enabled {
            let mut headers = SecurityHeadersMiddleware::default()
                .content_security_policy(&self.security.headers.csp)?
                .permissions_policy(&self.security.headers.permissions_policy)?;
            if let Some(value) = self.security.headers.hsts.as_deref() {
                headers = headers.hsts(value)?;
            } else {
                headers = headers.disable_hsts();
            }
            out.push(Arc::new(headers));
        }
        if self.security.cors.enabled {
            let cors = &self.security.cors;
            out.push(Arc::new(CorsMiddleware::new(
                cors.allowed_origins.clone(),
                cors.allowed_methods.clone(),
                cors.allowed_headers.clone(),
                cors.expose_headers.clone(),
                cors.allow_credentials,
                cors.max_age_secs,
            )?));
        }
        Ok(out)
    }

    pub async fn authentication(
        &self,
        database: &db::Database,
    ) -> ApiResult<(AuthenticationChain, FrameworkAuthService)> {
        use auth_runtime::{DatabaseAuthenticationStore, SignedCookieResolver};

        let configured = &self.security.authentication.backends;
        let needs_database = configured
            .iter()
            .any(|value| value == "session" || value == "bearer");
        let database_store = if needs_database {
            Some(DatabaseAuthenticationStore::initialize(database).await?)
        } else {
            None
        };
        let cookie_resolver = if configured.iter().any(|value| value == "cookie") {
            Some(Arc::new(SignedCookieResolver::from_settings(
                &self.security.authentication.cookie,
            )?))
        } else {
            None
        };

        let mut backends: Vec<Arc<dyn AuthenticationBackend>> = Vec::new();
        for configured in configured {
            match configured.as_str() {
                "session" => {
                    let store = database_store.clone().ok_or_else(|| {
                        lex_rest::rest_framework::exceptions::ApiError::configuration(
                            "session authentication store failed to initialize",
                        )
                    })?;
                    let source = &self.security.authentication.session;
                    backends.push(Arc::new(SessionAuthentication::new(
                        store,
                        SessionAuthenticationConfig {
                            cookie_name: source.cookie_name.clone(),
                            require_secure_transport: source.require_secure_transport,
                            csrf_protection: source.csrf_protection,
                            csrf_header: source.csrf_header.clone(),
                        },
                    )?));
                }
                "cookie" => {
                    let resolver = cookie_resolver.clone().ok_or_else(|| {
                        lex_rest::rest_framework::exceptions::ApiError::configuration(
                            "cookie authentication resolver failed to initialize",
                        )
                    })?;
                    let source = &self.security.authentication.cookie;
                    backends.push(Arc::new(CookieAuthentication::new(
                        resolver,
                        CookieAuthenticationConfig {
                            cookie_name: source.cookie_name.clone(),
                            require_secure_transport: source.require_secure_transport,
                            csrf_protection: source.csrf_protection,
                            csrf_header: source.csrf_header.clone(),
                        },
                    )?));
                }
                "bearer" => {
                    let resolver = database_store.clone().ok_or_else(|| {
                        lex_rest::rest_framework::exceptions::ApiError::configuration(
                            "bearer authentication store failed to initialize",
                        )
                    })?;
                    let source = &self.security.authentication.bearer;
                    backends.push(Arc::new(BearerTokenAuthentication::new(
                        resolver,
                        BearerTokenAuthenticationConfig {
                            scheme: source.scheme.clone(),
                        },
                    )?));
                }
                _ => {
                    return Err(
                        lex_rest::rest_framework::exceptions::ApiError::configuration(
                            "unsupported authentication backend",
                        ),
                    );
                }
            }
        }
        let chain = AuthenticationChain::new(backends)?;
        let service = FrameworkAuthService::new(
            database_store,
            cookie_resolver,
            self.security.authentication.clone(),
        );
        Ok((chain, service))
    }
}

pub fn is_production() -> bool {
    db::config::is_production()
}

fn validate_security(settings: &SecuritySettings, production: bool) -> Result<()> {
    if settings.cors.enabled {
        CorsMiddleware::new(
            settings.cors.allowed_origins.clone(),
            settings.cors.allowed_methods.clone(),
            settings.cors.allowed_headers.clone(),
            settings.cors.expose_headers.clone(),
            settings.cors.allow_credentials,
            settings.cors.max_age_secs,
        )
        .map_err(|error| {
            anyhow::anyhow!(
                "security.cors: {}",
                error.log_detail().unwrap_or(error.safe_detail())
            )
        })?;
    }
    if settings.trusted_proxy_hops > 16 {
        bail!("security.trusted_proxy_hops must be <= 16");
    }
    if settings.cors.allow_credentials && settings.cors.allowed_origins.iter().any(|v| v == "*") {
        bail!("credentialed CORS cannot use wildcard origins");
    }
    if production && settings.headers.csp.trim().is_empty() {
        bail!("production CSP cannot be empty");
    }

    let permission_backend = settings.permissions.backend.trim().to_ascii_lowercase();
    if !matches!(permission_backend.as_str(), "scopes" | "database") {
        bail!("security.permissions.backend must be 'scopes' or 'database'");
    }
    for (name, value) in [
        (
            "global_admin_scope",
            &settings.permissions.global_admin_scope,
        ),
        (
            "tenant_member_scope",
            &settings.permissions.tenant_member_scope,
        ),
        (
            "tenant_admin_scope",
            &settings.permissions.tenant_admin_scope,
        ),
    ] {
        if value.trim().is_empty() || value.chars().any(char::is_whitespace) {
            bail!("security.permissions.{name} must be a non-empty scope without whitespace");
        }
    }

    let auth = &settings.authentication;
    let mut configured = BTreeSet::new();
    for backend in &auth.backends {
        if !matches!(backend.as_str(), "session" | "cookie" | "bearer") {
            bail!("unsupported security.authentication backend: {backend}");
        }
        if !configured.insert(backend.as_str()) {
            bail!("authentication backends must not be duplicated");
        }
    }
    if auth.session.ttl_secs == 0 || auth.cookie.ttl_secs == 0 || auth.bearer.ttl_secs == 0 {
        bail!("authentication TTL values must be greater than zero");
    }
    if auth.cookie.secret_env.trim().is_empty() {
        bail!("security.authentication.cookie.secret_env cannot be empty");
    }
    if production {
        if auth.backends.iter().any(|value| value == "session") {
            if !auth.session.require_secure_transport {
                bail!("production session authentication must require secure transport");
            }
            if !auth.session.csrf_protection {
                bail!("production session authentication must keep CSRF protection enabled");
            }
        }
        if auth.backends.iter().any(|value| value == "cookie") {
            if !auth.cookie.require_secure_transport {
                bail!("production cookie authentication must require secure transport");
            }
            if !auth.cookie.csrf_protection {
                bail!("production cookie authentication must keep CSRF protection enabled");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_secure_and_developer_friendly() {
        let security = SecuritySettings::default();
        assert!(security.cors.enabled);
        assert!(!security.cors.allowed_origins.is_empty());
        assert!(security.headers.csp.contains("default-src"));
        assert!(security.authentication.backends.is_empty());
        assert_eq!(security.permissions.backend, "scopes");
    }

    #[test]
    fn credentialed_wildcard_cors_is_rejected() {
        let mut security = SecuritySettings::default();
        security.cors.allow_credentials = true;
        security.cors.allowed_origins = vec!["*".into()];
        assert!(validate_security(&security, false).is_err());
    }

    #[test]
    fn production_cookie_auth_requires_tls_and_csrf() {
        let mut security = SecuritySettings::default();
        security.authentication.backends = vec!["cookie".into()];
        assert!(validate_security(&security, true).is_err());
        security.authentication.cookie.require_secure_transport = true;
        assert!(validate_security(&security, true).is_ok());
        security.authentication.cookie.csrf_protection = false;
        assert!(validate_security(&security, true).is_err());
    }

    #[test]
    fn duplicate_auth_backends_are_rejected() {
        let mut security = SecuritySettings::default();
        security.authentication.backends = vec!["bearer".into(), "bearer".into()];
        assert!(validate_security(&security, false).is_err());
    }

    #[tokio::test]
    async fn framework_owns_session_and_bearer_authentication_on_sqlite() {
        use http::{
            HeaderMap, HeaderValue, Method, Uri,
            header::{AUTHORIZATION, COOKIE},
        };
        use lex_rest::rest_framework::request::{ApiRequest, TransportSecurity};
        use uuid::Uuid;

        let mut core = db::Settings::default();
        core.database.backend = db::DatabaseBackend::Sqlite;
        core.database.url = "sqlite::memory:".to_owned();
        core.database.require_ssl = false;
        core.database.max_connections = 1;
        core.database.min_connections = 1;
        let database = db::Database::initialize_with(&core)
            .await
            .expect("database");

        let mut settings = FrameworkSettings {
            core,
            media: MediaSettings::default(),
            security: SecuritySettings::default(),
        };
        settings.security.authentication.backends = vec!["session".into(), "bearer".into()];
        let (chain, auth) = settings
            .authentication(&database)
            .await
            .expect("authentication");
        let user_id = Uuid::now_v7();

        let session = auth
            .create_session(user_id, ["profile:read".to_owned()])
            .await
            .expect("session");
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_str(&format!("lex_session={}", session.token)).expect("cookie"),
        );
        let request = ApiRequest::new(
            Method::GET,
            Uri::from_static("/private"),
            headers,
            vec![],
            None,
            TransportSecurity::Insecure,
            1024,
        )
        .expect("request");
        let principal = chain.authenticate(&request).await.expect("principal");
        assert_eq!(principal.user_id(), Some(user_id));
        assert_eq!(principal.authentication_method(), "session");

        let bearer = auth
            .create_bearer_token(user_id, ["api:read".to_owned()])
            .await
            .expect("bearer");
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", bearer.token)).expect("authorization"),
        );
        let request = ApiRequest::new(
            Method::GET,
            Uri::from_static("/private"),
            headers,
            vec![],
            None,
            TransportSecurity::Insecure,
            1024,
        )
        .expect("request");
        let principal = chain.authenticate(&request).await.expect("principal");
        assert_eq!(principal.user_id(), Some(user_id));
        assert_eq!(principal.authentication_method(), "bearer");

        drop(chain);
        drop(auth);
        database.close().await;
    }
}
