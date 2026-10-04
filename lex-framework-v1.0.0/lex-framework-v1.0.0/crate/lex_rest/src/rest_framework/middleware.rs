use async_trait::async_trait;
use http::{HeaderName, HeaderValue, Method, StatusCode};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;

use crate::rest_framework::{
    audit::{AuditEvent, AuditOutcome, AuditSink, TracingAuditSink},
    authentication::AuthenticationChain,
    exceptions::{ApiError, ApiResult},
    idempotency::{IdempotencyReservation, IdempotencyStore, prepare_idempotency},
    permissions::PermissionBackend,
    request::{ApiRequest, Principal, RequestContext, TransportSecurity},
    response::ApiResponse,
    routing::{RouteEndpoint, RouteTable},
    throttling::{ThrottleEngine, ThrottlePolicy},
    views::{APIView, ViewServices, is_read_only_method},
    websocket::WebSocketView,
};

#[async_trait]
pub trait TenantResolver: Send + Sync {
    /// Resolve tenant only from authenticated/validated request state. Implementations that honor
    /// client headers must validate them against the authenticated identity; the framework will
    /// still enforce membership/permissions afterwards.
    async fn resolve_tenant(
        &self,
        request: &ApiRequest,
        principal: &Principal,
    ) -> ApiResult<Option<Uuid>>;
}

#[derive(Debug, Default)]
pub struct NoTenantResolver;

#[async_trait]
impl TenantResolver for NoTenantResolver {
    async fn resolve_tenant(
        &self,
        _request: &ApiRequest,
        _principal: &Principal,
    ) -> ApiResult<Option<Uuid>> {
        Ok(None)
    }
}

#[async_trait]
pub trait RequestMiddleware: Send + Sync {
    async fn before(&self, _request: &ApiRequest) -> ApiResult<()> {
        Ok(())
    }

    /// Response hook for failures that occur before an authenticated RequestContext exists.
    /// Identity-dependent middleware should keep the default no-op implementation.
    async fn after_pre_context(
        &self,
        _request: &ApiRequest,
        _response: &mut ApiResponse,
    ) -> ApiResult<()> {
        Ok(())
    }

    async fn after(
        &self,
        _request: &ApiRequest,
        _context: &RequestContext,
        _response: &mut ApiResponse,
    ) -> ApiResult<()> {
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct RequestLimitsMiddleware {
    max_uri_bytes: usize,
    max_header_count: usize,
    max_header_bytes: usize,
    max_single_header_bytes: usize,
}

impl Default for RequestLimitsMiddleware {
    fn default() -> Self {
        Self {
            max_uri_bytes: 8 * 1024,
            max_header_count: 100,
            max_header_bytes: 64 * 1024,
            max_single_header_bytes: 16 * 1024,
        }
    }
}

impl RequestLimitsMiddleware {
    pub fn new(
        max_uri_bytes: usize,
        max_header_count: usize,
        max_header_bytes: usize,
        max_single_header_bytes: usize,
    ) -> ApiResult<Self> {
        if !(256..=64 * 1024).contains(&max_uri_bytes)
            || !(1..=512).contains(&max_header_count)
            || !(1024..=1024 * 1024).contains(&max_header_bytes)
            || !(256..=256 * 1024).contains(&max_single_header_bytes)
            || max_single_header_bytes > max_header_bytes
        {
            return Err(ApiError::configuration("invalid request header/URI limits"));
        }
        Ok(Self {
            max_uri_bytes,
            max_header_count,
            max_header_bytes,
            max_single_header_bytes,
        })
    }
}

#[async_trait]
impl RequestMiddleware for RequestLimitsMiddleware {
    async fn before(&self, request: &ApiRequest) -> ApiResult<()> {
        if request.uri().to_string().len() > self.max_uri_bytes {
            return Err(ApiError::new(
                StatusCode::URI_TOO_LONG,
                "uri_too_long",
                "request URI exceeds the configured limit",
            ));
        }
        if request.headers().len() > self.max_header_count {
            return Err(ApiError::new(
                StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
                "too_many_headers",
                "request contains too many headers",
            ));
        }
        let mut total = 0usize;
        for (name, value) in request.headers() {
            let current = name.as_str().len().saturating_add(value.as_bytes().len());
            if current > self.max_single_header_bytes {
                return Err(ApiError::new(
                    StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
                    "header_too_large",
                    "a request header exceeds the configured limit",
                ));
            }
            total = total.checked_add(current).ok_or_else(|| {
                ApiError::new(
                    StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
                    "headers_too_large",
                    "request headers exceed the configured limit",
                )
            })?;
            if total > self.max_header_bytes {
                return Err(ApiError::new(
                    StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
                    "headers_too_large",
                    "request headers exceed the configured limit",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct SecurityHeadersMiddleware {
    hsts: Option<HeaderValue>,
    content_security_policy: HeaderValue,
    permissions_policy: HeaderValue,
}

impl Default for SecurityHeadersMiddleware {
    fn default() -> Self {
        Self {
            hsts: Some(HeaderValue::from_static("max-age=31536000")),
            content_security_policy: HeaderValue::from_static(
                "default-src 'none'; frame-ancestors 'none'; base-uri 'none'",
            ),
            permissions_policy: HeaderValue::from_static(
                "camera=(), microphone=(), geolocation=()",
            ),
        }
    }
}

impl SecurityHeadersMiddleware {
    pub fn disable_hsts(mut self) -> Self {
        self.hsts = None;
        self
    }

    pub fn hsts(mut self, value: &str) -> ApiResult<Self> {
        self.hsts = Some(
            HeaderValue::from_str(value)
                .map_err(|_| ApiError::configuration("invalid HSTS header value"))?,
        );
        Ok(self)
    }

    pub fn content_security_policy(mut self, value: &str) -> ApiResult<Self> {
        self.content_security_policy = HeaderValue::from_str(value)
            .map_err(|_| ApiError::configuration("invalid Content-Security-Policy value"))?;
        Ok(self)
    }

    pub fn permissions_policy(mut self, value: &str) -> ApiResult<Self> {
        self.permissions_policy = HeaderValue::from_str(value)
            .map_err(|_| ApiError::configuration("invalid Permissions-Policy value"))?;
        Ok(self)
    }

    fn apply_headers(&self, secure: bool, response: &mut ApiResponse) {
        response.headers_mut().insert(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        );
        response.headers_mut().insert(
            HeaderName::from_static("x-frame-options"),
            HeaderValue::from_static("DENY"),
        );
        response.headers_mut().insert(
            HeaderName::from_static("referrer-policy"),
            HeaderValue::from_static("no-referrer"),
        );
        response.headers_mut().insert(
            HeaderName::from_static("content-security-policy"),
            self.content_security_policy.clone(),
        );
        response.headers_mut().insert(
            HeaderName::from_static("permissions-policy"),
            self.permissions_policy.clone(),
        );
        if secure && let Some(hsts) = self.hsts.as_ref() {
            response.headers_mut().insert(
                HeaderName::from_static("strict-transport-security"),
                hsts.clone(),
            );
        }
    }
}

#[async_trait]
impl RequestMiddleware for SecurityHeadersMiddleware {
    async fn after_pre_context(
        &self,
        request: &ApiRequest,
        response: &mut ApiResponse,
    ) -> ApiResult<()> {
        self.apply_headers(request.transport_security().is_secure(), response);
        Ok(())
    }
    async fn after(
        &self,
        _request: &ApiRequest,
        context: &RequestContext,
        response: &mut ApiResponse,
    ) -> ApiResult<()> {
        self.apply_headers(context.transport_security().is_secure(), response);
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct CorsMiddleware {
    allowed_origins: Vec<String>,
    allowed_methods: String,
    allowed_headers: String,
    expose_headers: String,
    allow_credentials: bool,
    max_age_secs: u64,
}

impl CorsMiddleware {
    pub fn new(
        allowed_origins: Vec<String>,
        allowed_methods: Vec<String>,
        allowed_headers: Vec<String>,
        expose_headers: Vec<String>,
        allow_credentials: bool,
        max_age_secs: u64,
    ) -> ApiResult<Self> {
        if allowed_origins.is_empty() || max_age_secs > 86400 {
            return Err(ApiError::configuration("invalid CORS configuration"));
        }
        if allow_credentials && allowed_origins.iter().any(|v| v == "*") {
            return Err(ApiError::configuration(
                "CORS wildcard origin cannot be combined with credentials",
            ));
        }
        for origin in &allowed_origins {
            if origin == "*" {
                continue;
            }
            let valid = url::Url::parse(origin).is_ok_and(|value| {
                matches!(value.scheme(), "http" | "https")
                    && value.origin().ascii_serialization() == *origin
            });
            if !valid {
                return Err(ApiError::configuration(
                    "CORS origins must be canonical HTTP(S) origins without paths or credentials",
                ));
            }
        }
        if allowed_methods.is_empty()
            || allowed_methods
                .iter()
                .any(|method| Method::from_bytes(method.as_bytes()).is_err())
        {
            return Err(ApiError::configuration(
                "CORS allowed methods must be valid HTTP method names",
            ));
        }
        if allowed_headers
            .iter()
            .chain(&expose_headers)
            .any(|name| HeaderName::from_bytes(name.as_bytes()).is_err())
        {
            return Err(ApiError::configuration(
                "CORS header names must be valid HTTP header names",
            ));
        }
        Ok(Self {
            allowed_origins,
            allowed_methods: allowed_methods.join(", "),
            allowed_headers: allowed_headers.join(", "),
            expose_headers: expose_headers.join(", "),
            allow_credentials,
            max_age_secs,
        })
    }
    fn request_origin<'a>(&self, request: &'a ApiRequest) -> Option<&'a str> {
        request
            .headers()
            .get(http::header::ORIGIN)
            .and_then(|v| v.to_str().ok())
    }
    fn origin_allowed(&self, origin: &str) -> bool {
        self.allowed_origins.iter().any(|v| v == "*" || v == origin)
    }
    fn apply(&self, request: &ApiRequest, response: &mut ApiResponse) -> ApiResult<()> {
        let Some(origin) = self.request_origin(request) else {
            return Ok(());
        };
        if !self.origin_allowed(origin) {
            return Ok(());
        }
        let value = if self.allowed_origins.iter().any(|v| v == "*") {
            "*"
        } else {
            origin
        };
        response.headers_mut().insert(
            http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_str(value)
                .map_err(|_| ApiError::configuration("invalid CORS origin"))?,
        );
        response
            .headers_mut()
            .append(http::header::VARY, HeaderValue::from_static("Origin"));
        response.headers_mut().insert(
            http::header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_str(&self.allowed_methods)
                .map_err(|_| ApiError::configuration("invalid CORS methods"))?,
        );
        response.headers_mut().insert(
            http::header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_str(&self.allowed_headers)
                .map_err(|_| ApiError::configuration("invalid CORS headers"))?,
        );
        if !self.expose_headers.is_empty() {
            response.headers_mut().insert(
                http::header::ACCESS_CONTROL_EXPOSE_HEADERS,
                HeaderValue::from_str(&self.expose_headers)
                    .map_err(|_| ApiError::configuration("invalid CORS expose headers"))?,
            );
        }
        if self.allow_credentials {
            response.headers_mut().insert(
                http::header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
                HeaderValue::from_static("true"),
            );
        }
        response.headers_mut().insert(
            http::header::ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from_str(&self.max_age_secs.to_string())
                .map_err(|_| ApiError::configuration("invalid CORS max age"))?,
        );
        Ok(())
    }
}
#[async_trait]
impl RequestMiddleware for CorsMiddleware {
    async fn before(&self, request: &ApiRequest) -> ApiResult<()> {
        if request
            .headers()
            .get_all(http::header::ORIGIN)
            .iter()
            .count()
            > 1
            || request
                .headers()
                .get(http::header::ORIGIN)
                .is_some_and(|value| value.to_str().is_err())
        {
            return Err(ApiError::bad_request(
                "invalid_origin",
                "request must contain one valid Origin header",
            ));
        }
        if let Some(origin) = self.request_origin(request)
            && !self.origin_allowed(origin)
        {
            return Err(ApiError::permission_denied(
                "request Origin is not allowed by CORS policy",
            ));
        }
        if request.method() == Method::OPTIONS && self.request_origin(request).is_some() {
            if let Some(method) = request
                .headers()
                .get(http::header::ACCESS_CONTROL_REQUEST_METHOD)
            {
                let method = Method::from_bytes(method.as_bytes()).map_err(|_| {
                    ApiError::bad_request("invalid_preflight", "invalid preflight method")
                })?;
                if !self
                    .allowed_methods
                    .split(',')
                    .any(|allowed| allowed.trim() == method.as_str())
                {
                    return Err(ApiError::permission_denied(
                        "preflight method is not allowed by CORS policy",
                    ));
                }
            }
            if let Some(headers) = request
                .headers()
                .get(http::header::ACCESS_CONTROL_REQUEST_HEADERS)
            {
                let headers = headers.to_str().map_err(|_| {
                    ApiError::bad_request("invalid_preflight", "invalid preflight headers")
                })?;
                for name in headers
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                {
                    let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
                        ApiError::bad_request("invalid_preflight", "invalid preflight header name")
                    })?;
                    let allowed = self
                        .allowed_headers
                        .split(',')
                        .map(str::trim)
                        .any(|allowed| {
                            allowed.eq_ignore_ascii_case(name.as_str())
                                || (allowed == "*"
                                    && !self.allow_credentials
                                    && name != http::header::AUTHORIZATION)
                        });
                    if !allowed {
                        return Err(ApiError::permission_denied(
                            "preflight header is not allowed by CORS policy",
                        ));
                    }
                }
            }
        }
        Ok(())
    }
    async fn after_pre_context(
        &self,
        request: &ApiRequest,
        response: &mut ApiResponse,
    ) -> ApiResult<()> {
        self.apply(request, response)
    }
    async fn after(
        &self,
        request: &ApiRequest,
        _context: &RequestContext,
        response: &mut ApiResponse,
    ) -> ApiResult<()> {
        self.apply(request, response)
    }
}

#[derive(Clone)]
pub struct RestRuntime {
    authentication: AuthenticationChain,
    permissions: Arc<dyn PermissionBackend>,
    tenant_resolver: Arc<dyn TenantResolver>,
    throttle_engine: Option<ThrottleEngine>,
    global_throttles: Arc<Vec<ThrottlePolicy>>,
    idempotency_store: Option<Arc<dyn IdempotencyStore>>,
    audit_sink: Arc<dyn AuditSink>,
    middleware: Arc<Vec<Arc<dyn RequestMiddleware>>>,
    request_timeout: Duration,
    require_secure_transport: bool,
}

pub struct RestRuntimeBuilder {
    authentication: AuthenticationChain,
    permissions: Arc<dyn PermissionBackend>,
    tenant_resolver: Arc<dyn TenantResolver>,
    throttle_engine: Option<ThrottleEngine>,
    global_throttles: Vec<ThrottlePolicy>,
    idempotency_store: Option<Arc<dyn IdempotencyStore>>,
    audit_sink: Arc<dyn AuditSink>,
    middleware: Vec<Arc<dyn RequestMiddleware>>,
    request_timeout: Duration,
    require_secure_transport: bool,
}

impl RestRuntimeBuilder {
    pub fn new(permissions: Arc<dyn PermissionBackend>) -> Self {
        Self {
            authentication: AuthenticationChain::default(),
            permissions,
            tenant_resolver: Arc::new(NoTenantResolver),
            throttle_engine: None,
            global_throttles: Vec::new(),
            idempotency_store: None,
            audit_sink: Arc::new(TracingAuditSink),
            middleware: Vec::new(),
            request_timeout: Duration::from_secs(30),
            require_secure_transport: true,
        }
    }

    pub fn authentication(mut self, authentication: AuthenticationChain) -> Self {
        self.authentication = authentication;
        self
    }

    pub fn tenant_resolver(mut self, resolver: Arc<dyn TenantResolver>) -> Self {
        self.tenant_resolver = resolver;
        self
    }

    pub fn throttling(
        mut self,
        engine: ThrottleEngine,
        global_policies: Vec<ThrottlePolicy>,
    ) -> Self {
        self.throttle_engine = Some(engine);
        self.global_throttles = global_policies;
        self
    }

    pub fn idempotency_store(mut self, store: Arc<dyn IdempotencyStore>) -> Self {
        self.idempotency_store = Some(store);
        self
    }

    pub fn audit_sink(mut self, sink: Arc<dyn AuditSink>) -> Self {
        self.audit_sink = sink;
        self
    }

    pub fn middleware(mut self, middleware: Vec<Arc<dyn RequestMiddleware>>) -> Self {
        self.middleware = middleware;
        self
    }

    pub fn request_timeout(mut self, timeout: Duration) -> ApiResult<Self> {
        if timeout.is_zero() || timeout > Duration::from_secs(300) {
            return Err(ApiError::configuration(
                "REST request timeout must be within 1ns..=300s",
            ));
        }
        self.request_timeout = timeout;
        Ok(self)
    }

    pub fn require_secure_transport(mut self, require: bool) -> Self {
        self.require_secure_transport = require;
        self
    }

    pub fn build(self) -> ApiResult<RestRuntime> {
        if self.global_throttles.len() > 16 {
            return Err(ApiError::configuration("too many global throttle policies"));
        }
        for policy in &self.global_throttles {
            policy.validate()?;
        }
        if self.middleware.len() > 32 {
            return Err(ApiError::configuration(
                "too many REST middleware components",
            ));
        }
        Ok(RestRuntime {
            authentication: self.authentication,
            permissions: self.permissions,
            tenant_resolver: self.tenant_resolver,
            throttle_engine: self.throttle_engine,
            global_throttles: Arc::new(self.global_throttles),
            idempotency_store: self.idempotency_store,
            audit_sink: self.audit_sink,
            middleware: Arc::new(self.middleware),
            request_timeout: self.request_timeout,
            require_secure_transport: self.require_secure_transport,
        })
    }
}

impl RestRuntime {
    async fn build_context(&self, request: &ApiRequest) -> ApiResult<RequestContext> {
        if self.require_secure_transport
            && request.transport_security() != TransportSecurity::Secure
        {
            return Err(ApiError::permission_denied("HTTPS transport is required"));
        }
        for middleware in self.middleware.iter() {
            middleware.before(request).await?;
        }
        let principal = self.authentication.authenticate(request).await?;
        let tenant_id = self
            .tenant_resolver
            .resolve_tenant(request, &principal)
            .await?;
        let trace_id = trace_id_from_headers(request);
        RequestContext::new(
            principal,
            tenant_id,
            request.peer_ip(),
            request.transport_security(),
            trace_id,
            Some(self.request_timeout),
        )
    }

    async fn enforce_throttles(
        &self,
        context: &RequestContext,
        view_policies: Vec<ThrottlePolicy>,
    ) -> ApiResult<()> {
        if self.global_throttles.is_empty() && view_policies.is_empty() {
            return Ok(());
        }
        let engine = self.throttle_engine.as_ref().ok_or_else(|| {
            ApiError::configuration(
                "throttle policies configured without a distributed throttle store",
            )
        })?;
        for policy in self.global_throttles.iter().chain(view_policies.iter()) {
            engine.check(policy, context).await?;
        }
        Ok(())
    }

    pub async fn dispatch_http(
        &self,
        route_name: &str,
        view: &dyn APIView,
        request: &ApiRequest,
    ) -> ApiResponse {
        let method = request.method().clone();
        let started = Instant::now();
        let context = match self.build_context(request).await {
            Ok(context) => context,
            Err(error) => {
                let request_id = Uuid::now_v7();
                let mut response = error.to_response(request_id);
                response.set_request_id(request_id);
                self.apply_authentication_challenges(&mut response);
                for middleware in self.middleware.iter().rev() {
                    if let Err(middleware_error) =
                        middleware.after_pre_context(request, &mut response).await
                    {
                        tracing::error!(
                            request_id = %request_id,
                            error_code = middleware_error.code(),
                            "pre-context response middleware failed"
                        );
                        response = middleware_error.to_response(request_id);
                        response.set_request_id(request_id);
                        self.apply_authentication_challenges(&mut response);
                    }
                }
                let outcome = match response.status() {
                    StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => AuditOutcome::Denied,
                    StatusCode::TOO_MANY_REQUESTS => AuditOutcome::Throttled,
                    status if status.is_client_error() => AuditOutcome::ClientError,
                    _ => AuditOutcome::ServerError,
                };
                let event = AuditEvent::before_context(
                    request_id,
                    trace_id_from_headers(request),
                    route_name,
                    method.as_str(),
                    response.status().as_u16(),
                    started.elapsed(),
                    outcome,
                );
                if let Err(audit_error) = self.audit_sink.record(event).await {
                    tracing::error!(
                        request_id = %request_id,
                        error_code = audit_error.code(),
                        "pre-authentication audit sink failed"
                    );
                }
                return response;
            }
        };

        let result = self
            .dispatch_http_inner(route_name, view, request, &context, &method)
            .await;
        let mut response = match result {
            Ok(response) => response,
            Err(error) => error.to_response(context.request_id()),
        };
        response.set_request_id(context.request_id());
        self.apply_authentication_challenges(&mut response);

        // Middleware runs for both successful and error responses once a RequestContext exists.
        // Continue the reverse chain after a failure so security/header middleware still executes.
        for middleware in self.middleware.iter().rev() {
            if let Err(error) = middleware.after(request, &context, &mut response).await {
                tracing::error!(
                    request_id = %context.request_id(),
                    error_code = error.code(),
                    "response middleware failed"
                );
                response = error.to_response(context.request_id());
                response.set_request_id(context.request_id());
                self.apply_authentication_challenges(&mut response);
            }
        }
        self.record_audit(route_name, &method, &context, &response)
            .await;
        response
    }

    async fn dispatch_http_inner(
        &self,
        route_name: &str,
        view: &dyn APIView,
        request: &ApiRequest,
        context: &RequestContext,
        method: &Method,
    ) -> ApiResult<ApiResponse> {
        let services = ViewServices::new(Arc::clone(&self.permissions));
        let permission = view.permission_policy(method);
        permission
            .enforce(
                self.permissions.as_ref(),
                context,
                None,
                is_read_only_method(method),
            )
            .await?;
        self.enforce_throttles(context, view.throttle_policies(method))
            .await?;

        let idempotency_policy = view.idempotency_policy(method);
        let idempotency = prepare_idempotency(request, context, route_name, &idempotency_policy)?;
        let mut reservation: Option<(String, String, String)> = None;
        if let Some(input) = idempotency {
            let store = self.idempotency_store.as_ref().ok_or_else(|| {
                ApiError::configuration(
                    "idempotency policy configured without an idempotency store",
                )
            })?;
            match store
                .reserve(
                    &input.namespace,
                    &input.key_hash,
                    &input.fingerprint,
                    idempotency_policy.ttl,
                )
                .await?
            {
                IdempotencyReservation::Started { token } => {
                    reservation = Some((input.namespace, input.key_hash, token));
                }
                IdempotencyReservation::Replay(mut response) => {
                    response.headers_mut().insert(
                        HeaderName::from_static("idempotency-replayed"),
                        HeaderValue::from_static("true"),
                    );
                    return Ok(response);
                }
                IdempotencyReservation::Conflict => {
                    return Err(ApiError::conflict(
                        "idempotency_conflict",
                        "the same Idempotency-Key was used with a different request",
                    ));
                }
                IdempotencyReservation::InProgress { retry_after } => {
                    return Err(ApiError::conflict(
                        "idempotency_in_progress",
                        format!(
                            "a request with this Idempotency-Key is still executing; retry after {} seconds",
                            retry_after.as_secs().max(1)
                        ),
                    ).with_retry_after(retry_after.as_secs()));
                }
            }
        }

        let execution = view.dispatch(request, context, &services).await;
        let response = match execution {
            Ok(response) => response,
            Err(error) => {
                if let Some((namespace, key_hash, token)) = reservation.as_ref()
                    && let Some(store) = self.idempotency_store.as_ref()
                    && let Err(abandon_error) = store.abandon(namespace, key_hash, token).await
                {
                    tracing::error!(
                        request_id = %context.request_id(),
                        error_code = abandon_error.code(),
                        "failed to abandon idempotency reservation"
                    );
                }
                return Err(error);
            }
        };

        if let Some((namespace, key_hash, token)) = reservation.as_ref() {
            let store = self.idempotency_store.as_ref().ok_or_else(|| {
                ApiError::configuration("idempotency store disappeared during request")
            })?;
            store
                .complete(
                    namespace,
                    key_hash,
                    token,
                    &response,
                    idempotency_policy.ttl,
                )
                .await
                .map_err(|error| {
                    ApiError::internal(format!(
                        "mutation completed but idempotency result persistence failed: {error}"
                    ))
                })?;
        }

        Ok(response)
    }

    fn apply_authentication_challenges(&self, response: &mut ApiResponse) {
        if response.status() != StatusCode::UNAUTHORIZED {
            return;
        }
        for challenge in self.authentication.challenges() {
            if let Ok(value) = HeaderValue::from_str(&challenge) {
                response
                    .headers_mut()
                    .append(http::header::WWW_AUTHENTICATE, value);
            } else {
                tracing::error!(
                    "authentication backend emitted an invalid WWW-Authenticate challenge"
                );
            }
        }
    }

    pub async fn authorize_websocket(
        &self,
        view: &dyn WebSocketView,
        request: &ApiRequest,
    ) -> ApiResult<(RequestContext, ViewServices)> {
        let context = self.build_context(request).await?;
        let security = view.security_policy();
        security.enforce_handshake(request)?;
        view.permission_policy()
            .enforce(self.permissions.as_ref(), &context, None, true)
            .await?;
        self.enforce_throttles(&context, view.throttle_policies())
            .await?;
        Ok((context, ViewServices::new(Arc::clone(&self.permissions))))
    }

    async fn render_unrouted_error(
        &self,
        route_name: &str,
        request: &ApiRequest,
        error: ApiError,
    ) -> ApiResponse {
        let context = match RequestContext::new(
            Principal::anonymous(),
            None,
            request.peer_ip(),
            request.transport_security(),
            trace_id_from_headers(request),
            Some(self.request_timeout),
        ) {
            Ok(context) => context,
            Err(_) => return error.to_response(Uuid::now_v7()),
        };
        let mut response = error.to_response(context.request_id());
        response.set_request_id(context.request_id());
        self.apply_authentication_challenges(&mut response);
        for middleware in self.middleware.iter().rev() {
            if let Err(middleware_error) = middleware.after(request, &context, &mut response).await
            {
                tracing::error!(
                    request_id = %context.request_id(),
                    error_code = middleware_error.code(),
                    "response middleware failed for unrouted request"
                );
                response = middleware_error.to_response(context.request_id());
                response.set_request_id(context.request_id());
                self.apply_authentication_challenges(&mut response);
            }
        }
        self.record_audit(route_name, request.method(), &context, &response)
            .await;
        response
    }

    async fn record_audit(
        &self,
        route_name: &str,
        method: &Method,
        context: &RequestContext,
        response: &ApiResponse,
    ) {
        let outcome = match response.status() {
            status if status.is_success() => AuditOutcome::Success,
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => AuditOutcome::Denied,
            StatusCode::TOO_MANY_REQUESTS => AuditOutcome::Throttled,
            status if status.is_client_error() => AuditOutcome::ClientError,
            _ => AuditOutcome::ServerError,
        };
        let event = AuditEvent::from_request(
            context,
            route_name,
            method.as_str(),
            response.status().as_u16(),
            outcome,
        );
        if let Err(error) = self.audit_sink.record(event).await {
            tracing::error!(
                request_id = %context.request_id(),
                error_code = error.code(),
                "audit sink failed"
            );
        }
    }
}

#[derive(Clone)]
pub struct RestApplication {
    routes: Arc<RouteTable>,
    runtime: Arc<RestRuntime>,
}

impl RestApplication {
    pub fn new(routes: RouteTable, runtime: RestRuntime) -> Self {
        Self {
            routes: Arc::new(routes),
            runtime: Arc::new(runtime),
        }
    }

    pub fn routes(&self) -> &RouteTable {
        self.routes.as_ref()
    }

    pub fn runtime(&self) -> &RestRuntime {
        self.runtime.as_ref()
    }

    pub async fn handle(&self, request: ApiRequest) -> ApiResponse {
        match self.routes.resolve(request.method(), request.path()) {
            Ok(resolved) => {
                let request = request.with_path_params(resolved.path_params);
                match resolved.spec.endpoint {
                    RouteEndpoint::Http(view) => {
                        self.runtime
                            .dispatch_http(&resolved.spec.name, view.as_ref(), &request)
                            .await
                    }
                    RouteEndpoint::WebSocket(_) => {
                        self.runtime
                            .render_unrouted_error(
                                &resolved.spec.name,
                                &request,
                                ApiError::new(
                                    StatusCode::UPGRADE_REQUIRED,
                                    "websocket_upgrade_required",
                                    "this endpoint requires a WebSocket upgrade",
                                ),
                            )
                            .await
                    }
                }
            }
            Err(error) => {
                self.runtime
                    .render_unrouted_error("unmatched_route", &request, error)
                    .await
            }
        }
    }
}

fn trace_id_from_headers(request: &ApiRequest) -> Option<String> {
    let value = request.headers().get("traceparent")?.to_str().ok()?;
    parse_traceparent(value)
}

fn parse_traceparent(value: &str) -> Option<String> {
    if value.len() > 256 || !value.is_ascii() {
        return None;
    }
    let parts = value.split('-').collect::<Vec<_>>();
    if parts.len() < 4
        || parts[0].len() != 2
        || parts[1].len() != 32
        || parts[2].len() != 16
        || parts[3].len() != 2
        || parts[0].eq_ignore_ascii_case("ff")
        || !parts
            .iter()
            .take(4)
            .all(|part| part.bytes().all(|byte| byte.is_ascii_hexdigit()))
        || parts[1].bytes().all(|byte| byte == b'0')
        || parts[2].bytes().all(|byte| byte == b'0')
    {
        return None;
    }
    // Trace Context version 00 has exactly four fields; later versions may append fields.
    if parts[0] == "00" && parts.len() != 4 {
        return None;
    }
    Some(parts[1].to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::parse_traceparent;

    #[test]
    fn traceparent_extracts_only_valid_nonzero_trace_id() {
        assert_eq!(
            parse_traceparent("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01").as_deref(),
            Some("4bf92f3577b34da6a3ce929d0e0e4736")
        );
        assert!(
            parse_traceparent("00-00000000000000000000000000000000-00f067aa0ba902b7-01").is_none()
        );
        assert!(
            parse_traceparent("ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01").is_none()
        );
    }
}
