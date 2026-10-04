use async_trait::async_trait;
use http::{
    Method,
    header::{AUTHORIZATION, COOKIE},
};
use std::{collections::BTreeSet, sync::Arc};

use crate::rest_framework::{
    exceptions::{ApiError, ApiResult},
    request::{ApiRequest, Principal, TransportSecurity},
};

const MAX_CREDENTIAL_BYTES: usize = 4096;
const MAX_COOKIE_HEADER_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone)]
pub enum AuthenticationOutcome {
    /// The backend does not recognize credentials for its mechanism.
    Abstain,
    /// Authentication succeeded and produced an immutable principal.
    Authenticated(Principal),
    /// Credentials for this mechanism were present but invalid. The chain stops immediately.
    Rejected {
        /// Static client-safe reason only. Token/parser/provider details belong in backend logs.
        safe_detail: &'static str,
    },
}

#[async_trait]
pub trait AuthenticationBackend: Send + Sync {
    fn name(&self) -> &'static str;

    async fn authenticate(&self, request: &ApiRequest) -> ApiResult<AuthenticationOutcome>;

    fn challenge(&self) -> Option<String> {
        None
    }
}

#[derive(Clone, Default)]
pub struct AuthenticationChain {
    backends: Arc<Vec<Arc<dyn AuthenticationBackend>>>,
}

impl AuthenticationChain {
    pub fn new(backends: Vec<Arc<dyn AuthenticationBackend>>) -> ApiResult<Self> {
        if backends.len() > 16 {
            return Err(ApiError::configuration(
                "at most 16 authentication backends may be configured",
            ));
        }
        let mut names = BTreeSet::new();
        for backend in &backends {
            let name = backend.name();
            if name.is_empty() || !names.insert(name) {
                return Err(ApiError::configuration(
                    "authentication backend names must be unique and non-empty",
                ));
            }
        }
        Ok(Self {
            backends: Arc::new(backends),
        })
    }

    pub async fn authenticate(&self, request: &ApiRequest) -> ApiResult<Principal> {
        let mut authenticated = None;
        for backend in self.backends.iter() {
            match backend.authenticate(request).await? {
                AuthenticationOutcome::Abstain => {}
                AuthenticationOutcome::Authenticated(principal) => {
                    // Validate every presented mechanism; backend ordering must not let a valid
                    // cookie hide an invalid bearer token or select between competing identities.
                    if authenticated.replace(principal).is_some() {
                        return Err(ApiError::authentication(
                            "multiple authentication credentials are not allowed",
                        ));
                    }
                }
                AuthenticationOutcome::Rejected { safe_detail } => {
                    tracing::warn!(backend = backend.name(), "authentication rejected");
                    return Err(ApiError::authentication(safe_detail));
                }
            }
        }
        Ok(authenticated.unwrap_or_else(Principal::anonymous))
    }

    pub fn challenges(&self) -> Vec<String> {
        self.backends
            .iter()
            .filter_map(|backend| backend.challenge())
            .collect()
    }
}

/// Adapter for services that have already authenticated a request at a trusted boundary (for
/// example an mTLS gateway). Implementations must validate that boundary before returning a
/// principal; `lex_rest` never trusts client-provided identity headers by itself.
#[async_trait]
pub trait TrustedPrincipalResolver: Send + Sync {
    async fn resolve(&self, request: &ApiRequest) -> ApiResult<Option<Principal>>;
}

pub struct TrustedBoundaryAuthentication<R> {
    resolver: Arc<R>,
}

impl<R> TrustedBoundaryAuthentication<R> {
    pub fn new(resolver: Arc<R>) -> Self {
        Self { resolver }
    }
}

#[async_trait]
impl<R> AuthenticationBackend for TrustedBoundaryAuthentication<R>
where
    R: TrustedPrincipalResolver + 'static,
{
    fn name(&self) -> &'static str {
        "trusted_boundary"
    }

    async fn authenticate(&self, request: &ApiRequest) -> ApiResult<AuthenticationOutcome> {
        self.resolver.resolve(request).await.map(|principal| {
            principal.map_or(
                AuthenticationOutcome::Abstain,
                AuthenticationOutcome::Authenticated,
            )
        })
    }
}

/// A resolved server-side session. Session identifiers remain opaque to the framework and must be
/// generated with sufficient entropy by the session store implementation.
#[derive(Clone)]
pub struct SessionIdentity {
    pub principal: Principal,
    /// Optional server-side CSRF secret. When present and CSRF protection is enabled, unsafe
    /// methods must submit the same value through the configured CSRF header.
    pub csrf_secret: Option<String>,
}

#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn resolve_session(
        &self,
        session_id: &str,
        request: &ApiRequest,
    ) -> ApiResult<Option<SessionIdentity>>;
}

#[derive(Debug, Clone)]
pub struct SessionAuthenticationConfig {
    pub cookie_name: String,
    pub require_secure_transport: bool,
    pub csrf_protection: bool,
    pub csrf_header: String,
}

impl Default for SessionAuthenticationConfig {
    fn default() -> Self {
        Self {
            cookie_name: "lex_session".to_owned(),
            require_secure_transport: false,
            csrf_protection: true,
            csrf_header: "x-csrf-token".to_owned(),
        }
    }
}

pub struct SessionAuthentication {
    store: Arc<dyn SessionStore>,
    config: SessionAuthenticationConfig,
}

impl SessionAuthentication {
    pub fn new(
        store: Arc<dyn SessionStore>,
        config: SessionAuthenticationConfig,
    ) -> ApiResult<Self> {
        validate_cookie_name(&config.cookie_name)?;
        validate_header_name(&config.csrf_header)?;
        Ok(Self { store, config })
    }
}

#[async_trait]
impl AuthenticationBackend for SessionAuthentication {
    fn name(&self) -> &'static str {
        "session"
    }

    async fn authenticate(&self, request: &ApiRequest) -> ApiResult<AuthenticationOutcome> {
        let Some(session_id) = cookie_value(request, &self.config.cookie_name)? else {
            return Ok(AuthenticationOutcome::Abstain);
        };
        if self.config.require_secure_transport
            && !matches!(request.transport_security(), TransportSecurity::Secure)
        {
            return Ok(AuthenticationOutcome::Rejected {
                safe_detail: "session authentication requires a secure transport",
            });
        }
        validate_credential(&session_id)?;
        let Some(identity) = self.store.resolve_session(&session_id, request).await? else {
            return Ok(AuthenticationOutcome::Rejected {
                safe_detail: "invalid or expired session",
            });
        };
        enforce_csrf(
            request,
            self.config.csrf_protection,
            &self.config.csrf_header,
            identity.csrf_secret.as_deref(),
        )?;
        Ok(AuthenticationOutcome::Authenticated(identity.principal))
    }
}

/// Identity resolved from an application-defined cookie format. Use this when an application needs
/// a signed/encrypted stateless cookie rather than a server-side session. The resolver is
/// responsible for signature/encryption verification, expiry, key rotation, and revocation policy.
#[derive(Clone)]
pub struct CookieIdentity {
    pub principal: Principal,
    pub csrf_secret: Option<String>,
}

#[async_trait]
pub trait CookiePrincipalResolver: Send + Sync {
    async fn resolve_cookie(
        &self,
        cookie_value: &str,
        request: &ApiRequest,
    ) -> ApiResult<Option<CookieIdentity>>;
}

#[derive(Debug, Clone)]
pub struct CookieAuthenticationConfig {
    pub cookie_name: String,
    pub require_secure_transport: bool,
    pub csrf_protection: bool,
    pub csrf_header: String,
}

impl Default for CookieAuthenticationConfig {
    fn default() -> Self {
        Self {
            cookie_name: "lex_auth".to_owned(),
            require_secure_transport: false,
            csrf_protection: true,
            csrf_header: "x-csrf-token".to_owned(),
        }
    }
}

pub struct CookieAuthentication {
    resolver: Arc<dyn CookiePrincipalResolver>,
    config: CookieAuthenticationConfig,
}

impl CookieAuthentication {
    pub fn new(
        resolver: Arc<dyn CookiePrincipalResolver>,
        config: CookieAuthenticationConfig,
    ) -> ApiResult<Self> {
        validate_cookie_name(&config.cookie_name)?;
        validate_header_name(&config.csrf_header)?;
        Ok(Self { resolver, config })
    }
}

#[async_trait]
impl AuthenticationBackend for CookieAuthentication {
    fn name(&self) -> &'static str {
        "cookie"
    }

    async fn authenticate(&self, request: &ApiRequest) -> ApiResult<AuthenticationOutcome> {
        let Some(value) = cookie_value(request, &self.config.cookie_name)? else {
            return Ok(AuthenticationOutcome::Abstain);
        };
        if self.config.require_secure_transport
            && !matches!(request.transport_security(), TransportSecurity::Secure)
        {
            return Ok(AuthenticationOutcome::Rejected {
                safe_detail: "cookie authentication requires a secure transport",
            });
        }
        validate_credential(&value)?;
        let Some(identity) = self.resolver.resolve_cookie(&value, request).await? else {
            return Ok(AuthenticationOutcome::Rejected {
                safe_detail: "invalid or expired authentication cookie",
            });
        };
        enforce_csrf(
            request,
            self.config.csrf_protection,
            &self.config.csrf_header,
            identity.csrf_secret.as_deref(),
        )?;
        Ok(AuthenticationOutcome::Authenticated(identity.principal))
    }
}

/// Resolves an opaque bearer token into a principal. Implementations should store only a one-way
/// digest of long-lived opaque tokens, enforce expiry/revocation, and perform constant-time digest
/// comparison. JWT/PASETO implementations may validate signed tokens here instead.
#[async_trait]
pub trait BearerTokenResolver: Send + Sync {
    async fn resolve_bearer_token(
        &self,
        token: &str,
        request: &ApiRequest,
    ) -> ApiResult<Option<Principal>>;
}

#[derive(Debug, Clone)]
pub struct BearerTokenAuthenticationConfig {
    pub scheme: String,
}

impl Default for BearerTokenAuthenticationConfig {
    fn default() -> Self {
        Self {
            scheme: "Bearer".to_owned(),
        }
    }
}

pub struct BearerTokenAuthentication {
    resolver: Arc<dyn BearerTokenResolver>,
    config: BearerTokenAuthenticationConfig,
}

impl BearerTokenAuthentication {
    pub fn new(
        resolver: Arc<dyn BearerTokenResolver>,
        config: BearerTokenAuthenticationConfig,
    ) -> ApiResult<Self> {
        if config.scheme.trim().is_empty()
            || config.scheme.len() > 32
            || !config
                .scheme
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        {
            return Err(ApiError::configuration(
                "invalid bearer authentication scheme",
            ));
        }
        Ok(Self { resolver, config })
    }
}

#[async_trait]
impl AuthenticationBackend for BearerTokenAuthentication {
    fn name(&self) -> &'static str {
        "bearer"
    }

    async fn authenticate(&self, request: &ApiRequest) -> ApiResult<AuthenticationOutcome> {
        let Some(value) = request.headers().get(AUTHORIZATION) else {
            return Ok(AuthenticationOutcome::Abstain);
        };
        let raw = value
            .to_str()
            .map_err(|_| ApiError::authentication("invalid authorization header"))?;
        let Some((scheme, token)) = raw.split_once(' ') else {
            return Ok(AuthenticationOutcome::Rejected {
                safe_detail: "invalid authorization header",
            });
        };
        if !scheme.eq_ignore_ascii_case(&self.config.scheme) {
            return Ok(AuthenticationOutcome::Abstain);
        }
        let token = token.trim();
        validate_credential(token)?;
        let Some(principal) = self.resolver.resolve_bearer_token(token, request).await? else {
            return Ok(AuthenticationOutcome::Rejected {
                safe_detail: "invalid or expired token",
            });
        };
        Ok(AuthenticationOutcome::Authenticated(principal))
    }

    fn challenge(&self) -> Option<String> {
        Some(self.config.scheme.clone())
    }
}

fn cookie_value(request: &ApiRequest, name: &str) -> ApiResult<Option<String>> {
    let Some(header) = request.headers().get(COOKIE) else {
        return Ok(None);
    };
    let raw = header
        .to_str()
        .map_err(|_| ApiError::authentication("invalid cookie header"))?;
    if raw.len() > MAX_COOKIE_HEADER_BYTES {
        return Err(ApiError::authentication("cookie header is too large"));
    }
    let mut found: Option<String> = None;
    for pair in raw.split(';') {
        let Some((key, value)) = pair.trim().split_once('=') else {
            continue;
        };
        if key.trim() == name {
            if found.is_some() {
                return Err(ApiError::authentication("duplicate authentication cookie"));
            }
            found = Some(value.trim().to_owned());
        }
    }
    Ok(found)
}

fn validate_cookie_name(name: &str) -> ApiResult<()> {
    if name.is_empty()
        || name.len() > 128
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        return Err(ApiError::configuration(
            "invalid authentication cookie name",
        ));
    }
    Ok(())
}

fn validate_header_name(name: &str) -> ApiResult<()> {
    http::header::HeaderName::from_bytes(name.as_bytes())
        .map(|_| ())
        .map_err(|_| ApiError::configuration("invalid authentication header name"))
}

fn validate_credential(value: &str) -> ApiResult<()> {
    if value.is_empty()
        || value.len() > MAX_CREDENTIAL_BYTES
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(ApiError::authentication(
            "invalid authentication credential",
        ));
    }
    Ok(())
}

fn enforce_csrf(
    request: &ApiRequest,
    enabled: bool,
    header_name: &str,
    expected: Option<&str>,
) -> ApiResult<()> {
    if !enabled || is_safe_method(request.method()) {
        return Ok(());
    }
    let Some(expected) = expected else {
        return Err(ApiError::permission_denied("CSRF validation failed"));
    };
    let header_name = http::header::HeaderName::from_bytes(header_name.as_bytes())
        .map_err(|_| ApiError::configuration("invalid CSRF header name"))?;
    let Some(actual) = request.headers().get(&header_name) else {
        return Err(ApiError::permission_denied("CSRF validation failed"));
    };
    let actual = actual
        .to_str()
        .map_err(|_| ApiError::permission_denied("CSRF validation failed"))?;
    if !constant_time_eq(expected.as_bytes(), actual.as_bytes()) {
        return Err(ApiError::permission_denied("CSRF validation failed"));
    }
    Ok(())
}

fn is_safe_method(method: &Method) -> bool {
    method == Method::GET
        || method == Method::HEAD
        || method == Method::OPTIONS
        || method == Method::TRACE
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut diff = left.len() ^ right.len();
    let max = left.len().max(right.len());
    for index in 0..max {
        let a = left.get(index).copied().unwrap_or(0);
        let b = right.get(index).copied().unwrap_or(0);
        diff |= usize::from(a ^ b);
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{HeaderMap, HeaderValue};
    use uuid::Uuid;

    struct TestSessionStore;

    #[async_trait]
    impl SessionStore for TestSessionStore {
        async fn resolve_session(
            &self,
            session_id: &str,
            _request: &ApiRequest,
        ) -> ApiResult<Option<SessionIdentity>> {
            if session_id != "good-session" {
                return Ok(None);
            }
            Ok(Some(SessionIdentity {
                principal: Principal::user(Uuid::nil(), "session", Vec::<String>::new())?,
                csrf_secret: Some("csrf-secret".to_owned()),
            }))
        }
    }

    struct TestTokenResolver;

    #[async_trait]
    impl BearerTokenResolver for TestTokenResolver {
        async fn resolve_bearer_token(
            &self,
            token: &str,
            _request: &ApiRequest,
        ) -> ApiResult<Option<Principal>> {
            if token != "valid-token" {
                return Ok(None);
            }
            Ok(Some(Principal::service(
                "worker",
                "bearer",
                ["jobs:run".to_owned()],
            )?))
        }
    }

    fn request(method: Method, headers: HeaderMap) -> ApiRequest {
        ApiRequest::new(
            method,
            "/".parse().expect("uri"),
            headers,
            Vec::new(),
            None,
            TransportSecurity::Secure,
            1024,
        )
        .expect("request")
    }

    #[tokio::test]
    async fn session_requires_csrf_on_unsafe_methods() {
        let backend = SessionAuthentication::new(
            Arc::new(TestSessionStore),
            SessionAuthenticationConfig::default(),
        )
        .expect("backend");
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, HeaderValue::from_static("lex_session=good-session"));
        let result = backend.authenticate(&request(Method::POST, headers)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn session_accepts_matching_csrf_header() {
        let backend = SessionAuthentication::new(
            Arc::new(TestSessionStore),
            SessionAuthenticationConfig::default(),
        )
        .expect("backend");
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, HeaderValue::from_static("lex_session=good-session"));
        headers.insert("x-csrf-token", HeaderValue::from_static("csrf-secret"));
        let result = backend
            .authenticate(&request(Method::POST, headers))
            .await
            .expect("auth");
        assert!(matches!(result, AuthenticationOutcome::Authenticated(_)));
    }

    #[tokio::test]
    async fn bearer_authentication_rejects_invalid_token() {
        let backend = BearerTokenAuthentication::new(
            Arc::new(TestTokenResolver),
            BearerTokenAuthenticationConfig::default(),
        )
        .expect("backend");
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer invalid"));
        let result = backend
            .authenticate(&request(Method::GET, headers))
            .await
            .expect("auth");
        assert!(matches!(result, AuthenticationOutcome::Rejected { .. }));
    }

    #[test]
    fn duplicate_auth_cookie_is_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("lex_session=a; lex_session=b"),
        );
        assert!(cookie_value(&request(Method::GET, headers), "lex_session").is_err());
    }
}
