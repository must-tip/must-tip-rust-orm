use http::{HeaderMap, Method, Uri};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt,
    net::IpAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;

use crate::rest_framework::exceptions::{ApiError, ApiResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    Anonymous,
    User,
    Service,
    ApiKey,
}

/// Immutable authenticated identity produced by an `AuthenticationBackend`.
///
/// Construction deliberately requires explicit fields and keeps authorization scopes private.
/// Callers cannot mutate a principal after request authentication has completed.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct Principal {
    kind: PrincipalKind,
    subject: String,
    user_id: Option<Uuid>,
    service_id: Option<String>,
    authentication_method: String,
    scopes: Arc<BTreeSet<String>>,
}

impl Principal {
    pub fn anonymous() -> Self {
        Self {
            kind: PrincipalKind::Anonymous,
            subject: "anonymous".to_owned(),
            user_id: None,
            service_id: None,
            authentication_method: "none".to_owned(),
            scopes: Arc::new(BTreeSet::new()),
        }
    }

    pub fn user(
        user_id: Uuid,
        authentication_method: impl Into<String>,
        scopes: impl IntoIterator<Item = String>,
    ) -> ApiResult<Self> {
        Self::new(
            PrincipalKind::User,
            format!("user:{user_id}"),
            Some(user_id),
            None,
            authentication_method,
            scopes,
        )
    }

    pub fn service(
        service_id: impl Into<String>,
        authentication_method: impl Into<String>,
        scopes: impl IntoIterator<Item = String>,
    ) -> ApiResult<Self> {
        let service_id = service_id.into();
        if service_id.trim().is_empty() || service_id.len() > 255 {
            return Err(ApiError::authentication(
                "invalid service principal identifier",
            ));
        }
        Self::new(
            PrincipalKind::Service,
            format!("service:{service_id}"),
            None,
            Some(service_id),
            authentication_method,
            scopes,
        )
    }

    pub fn api_key(
        owner_user_id: Uuid,
        key_id: Uuid,
        authentication_method: impl Into<String>,
        scopes: impl IntoIterator<Item = String>,
    ) -> ApiResult<Self> {
        Self::new(
            PrincipalKind::ApiKey,
            format!("api_key:{key_id}"),
            Some(owner_user_id),
            None,
            authentication_method,
            scopes,
        )
    }

    fn new(
        kind: PrincipalKind,
        subject: String,
        user_id: Option<Uuid>,
        service_id: Option<String>,
        authentication_method: impl Into<String>,
        scopes: impl IntoIterator<Item = String>,
    ) -> ApiResult<Self> {
        let authentication_method = authentication_method.into();
        if authentication_method.trim().is_empty() || authentication_method.len() > 128 {
            return Err(ApiError::authentication(
                "invalid authentication method identifier",
            ));
        }
        let mut normalized = BTreeSet::new();
        for scope in scopes {
            let scope = scope.trim();
            if scope.is_empty() || scope.len() > 255 || scope.chars().any(char::is_whitespace) {
                return Err(ApiError::authentication("invalid authentication scope"));
            }
            if normalized.len() >= 512 {
                return Err(ApiError::authentication("too many authentication scopes"));
            }
            normalized.insert(scope.to_owned());
        }
        Ok(Self {
            kind,
            subject,
            user_id,
            service_id,
            authentication_method,
            scopes: Arc::new(normalized),
        })
    }

    pub const fn kind(&self) -> PrincipalKind {
        self.kind
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }

    pub const fn user_id(&self) -> Option<Uuid> {
        self.user_id
    }

    pub fn service_id(&self) -> Option<&str> {
        self.service_id.as_deref()
    }

    pub fn authentication_method(&self) -> &str {
        &self.authentication_method
    }

    pub fn is_authenticated(&self) -> bool {
        !matches!(self.kind, PrincipalKind::Anonymous)
    }

    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.contains(scope)
    }

    pub fn scopes(&self) -> impl Iterator<Item = &str> {
        self.scopes.iter().map(String::as_str)
    }
}

impl fmt::Debug for Principal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Principal")
            .field("kind", &self.kind)
            .field("subject", &self.subject)
            .field("user_id", &self.user_id)
            .field("service_id", &self.service_id)
            .field("authentication_method", &self.authentication_method)
            .field("scope_count", &self.scopes.len())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportSecurity {
    Insecure,
    Secure,
}

impl TransportSecurity {
    pub const fn is_secure(self) -> bool {
        matches!(self, Self::Secure)
    }
}

/// Immutable per-request security and tracing state.
#[derive(Clone)]
pub struct RequestContext {
    request_id: Uuid,
    trace_id: Option<String>,
    principal: Principal,
    tenant_id: Option<Uuid>,
    peer_ip: Option<IpAddr>,
    transport_security: TransportSecurity,
    started_at: Instant,
    deadline: Option<Instant>,
}

impl RequestContext {
    pub fn new(
        principal: Principal,
        tenant_id: Option<Uuid>,
        peer_ip: Option<IpAddr>,
        transport_security: TransportSecurity,
        trace_id: Option<String>,
        timeout: Option<Duration>,
    ) -> ApiResult<Self> {
        if let Some(trace_id) = trace_id.as_deref()
            && (trace_id.is_empty() || trace_id.len() > 256 || !trace_id.is_ascii())
        {
            return Err(ApiError::bad_request(
                "invalid_trace_id",
                "invalid trace identifier",
            ));
        }
        if let Some(timeout) = timeout
            && (timeout.is_zero() || timeout > Duration::from_secs(300))
        {
            return Err(ApiError::bad_request(
                "invalid_timeout",
                "request timeout must be within 1ns..=300s",
            ));
        }
        let started_at = Instant::now();
        Ok(Self {
            request_id: Uuid::now_v7(),
            trace_id,
            principal,
            tenant_id,
            peer_ip,
            transport_security,
            started_at,
            deadline: timeout.and_then(|value| started_at.checked_add(value)),
        })
    }

    pub const fn request_id(&self) -> Uuid {
        self.request_id
    }

    pub fn trace_id(&self) -> Option<&str> {
        self.trace_id.as_deref()
    }

    pub const fn principal(&self) -> &Principal {
        &self.principal
    }

    /// Convenience accessor for the authenticated user id, when the principal
    /// represents a user or user-owned credential.
    pub const fn user_id(&self) -> Option<Uuid> {
        self.principal.user_id()
    }

    /// True when authentication resolved any non-anonymous principal.
    pub fn is_authenticated(&self) -> bool {
        self.principal.is_authenticated()
    }

    /// Convenience scope check for token/session-backed APIs.
    pub fn has_scope(&self, scope: &str) -> bool {
        self.principal.has_scope(scope)
    }

    pub const fn tenant_id(&self) -> Option<Uuid> {
        self.tenant_id
    }

    pub const fn peer_ip(&self) -> Option<IpAddr> {
        self.peer_ip
    }

    pub const fn transport_security(&self) -> TransportSecurity {
        self.transport_security
    }

    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    pub fn remaining(&self) -> Option<Duration> {
        self.deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
    }

    pub fn ensure_not_expired(&self) -> ApiResult<()> {
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(ApiError::timeout("request deadline exceeded"));
        }
        Ok(())
    }
}

impl fmt::Debug for RequestContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestContext")
            .field("request_id", &self.request_id)
            .field("trace_id", &self.trace_id)
            .field("principal", &self.principal)
            .field("tenant_id", &self.tenant_id)
            .field("peer_ip", &self.peer_ip)
            .field("transport_security", &self.transport_security)
            .finish()
    }
}

#[derive(Clone)]
pub struct ApiRequest {
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Arc<[u8]>,
    query: Arc<BTreeMap<String, Vec<String>>>,
    path_params: Arc<BTreeMap<String, String>>,
    peer_ip: Option<IpAddr>,
    transport_security: TransportSecurity,
}

impl ApiRequest {
    pub const DEFAULT_MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
    pub const MAX_QUERY_PAIRS: usize = 256;
    pub const MAX_QUERY_VALUE_BYTES: usize = 16 * 1024;

    pub fn new(
        method: Method,
        uri: Uri,
        headers: HeaderMap,
        body: Vec<u8>,
        peer_ip: Option<IpAddr>,
        transport_security: TransportSecurity,
        max_body_bytes: usize,
    ) -> ApiResult<Self> {
        if max_body_bytes == 0 || max_body_bytes > 64 * 1024 * 1024 {
            return Err(ApiError::configuration("invalid request body limit"));
        }
        if body.len() > max_body_bytes {
            return Err(ApiError::payload_too_large(max_body_bytes));
        }
        let query = parse_query(uri.query().unwrap_or_default())?;
        Ok(Self {
            method,
            uri,
            headers,
            body: Arc::from(body),
            query: Arc::new(query),
            path_params: Arc::new(BTreeMap::new()),
            peer_ip,
            transport_security,
        })
    }

    pub fn with_path_params(mut self, params: BTreeMap<String, String>) -> Self {
        self.path_params = Arc::new(params);
        self
    }

    pub const fn method(&self) -> &Method {
        &self.method
    }

    pub const fn uri(&self) -> &Uri {
        &self.uri
    }

    pub fn path(&self) -> &str {
        self.uri.path()
    }

    pub const fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    pub const fn peer_ip(&self) -> Option<IpAddr> {
        self.peer_ip
    }

    pub const fn transport_security(&self) -> TransportSecurity {
        self.transport_security
    }

    pub fn json(&self) -> ApiResult<Value> {
        let media_type = self
            .headers
            .get(http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(str::trim)
            .unwrap_or("");
        if !media_type.eq_ignore_ascii_case("application/json")
            && !(media_type.to_ascii_lowercase().starts_with("application/")
                && media_type.to_ascii_lowercase().ends_with("+json"))
        {
            return Err(ApiError::unsupported_media_type());
        }
        if self.body.is_empty() {
            return Err(ApiError::bad_request(
                "empty_body",
                "request body is required",
            ));
        }
        serde_json::from_slice(&self.body)
            .map_err(|_| ApiError::bad_request("invalid_json", "request body is not valid JSON"))
    }

    pub fn query_values(&self, key: &str) -> Option<&[String]> {
        self.query.get(key).map(Vec::as_slice)
    }

    /// Adapter for the existing ORM `RestQueryPolicy`, which intentionally accepts one value per
    /// key. Duplicate query keys are rejected rather than silently dropping values.
    pub fn query_single_map(&self) -> ApiResult<HashMap<String, String>> {
        let mut out = HashMap::with_capacity(self.query.len());
        for (key, values) in self.query.iter() {
            if values.len() != 1 {
                return Err(ApiError::bad_request(
                    "duplicate_query_parameter",
                    format!("query parameter {key} must appear exactly once"),
                ));
            }
            let value = values.first().cloned().ok_or_else(|| {
                ApiError::bad_request("invalid_query_parameter", "query parameter has no value")
            })?;
            out.insert(key.clone(), value);
        }
        Ok(out)
    }

    pub fn path_param(&self, name: &str) -> Option<&str> {
        self.path_params.get(name).map(String::as_str)
    }

    pub fn path_params(&self) -> &BTreeMap<String, String> {
        &self.path_params
    }
}

impl fmt::Debug for ApiRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sensitive_headers = [
            "authorization",
            "cookie",
            "proxy-authorization",
            "x-api-key",
        ];
        let visible_headers = self
            .headers
            .keys()
            .filter(|name| !sensitive_headers.contains(&name.as_str()))
            .map(|name| name.as_str())
            .collect::<Vec<_>>();
        f.debug_struct("ApiRequest")
            .field("method", &self.method)
            .field("path", &self.uri.path())
            .field("query_parameter_keys", &self.query.len())
            .field("visible_header_names", &visible_headers)
            .field("body_bytes", &self.body.len())
            .field("peer_ip", &self.peer_ip)
            .field("transport_security", &self.transport_security)
            .finish()
    }
}

fn parse_query(raw: &str) -> ApiResult<BTreeMap<String, Vec<String>>> {
    let mut output: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if raw.is_empty() {
        return Ok(output);
    }
    let mut count = 0usize;
    for (key, value) in url::form_urlencoded::parse(raw.as_bytes()) {
        count += 1;
        if count > ApiRequest::MAX_QUERY_PAIRS {
            return Err(ApiError::bad_request(
                "query_too_complex",
                "too many query parameters",
            ));
        }
        if key.is_empty()
            || key.len() > 256
            || value.len() > ApiRequest::MAX_QUERY_VALUE_BYTES
            || key.chars().any(char::is_control)
            || value.chars().any(char::is_control)
        {
            return Err(ApiError::bad_request(
                "invalid_query_parameter",
                "invalid query parameter",
            ));
        }
        output
            .entry(key.into_owned())
            .or_default()
            .push(value.into_owned());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::header::{AUTHORIZATION, CONTENT_TYPE};

    #[test]
    fn debug_redacts_sensitive_request_material() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, "Bearer top-secret".parse().expect("header"));
        headers.insert(CONTENT_TYPE, "application/json".parse().expect("header"));
        let request = ApiRequest::new(
            Method::POST,
            "/users?token=query-secret".parse().expect("uri"),
            headers,
            br#"{"password":"secret"}"#.to_vec(),
            None,
            TransportSecurity::Secure,
            1024,
        )
        .expect("request");
        let rendered = format!("{request:?}");
        assert!(!rendered.contains("top-secret"));
        assert!(!rendered.contains("password"));
        assert!(!rendered.contains("query-secret"));
        assert!(!rendered.contains("token="));
        assert!(rendered.contains("content-type"));
    }

    #[test]
    fn duplicate_query_keys_are_not_collapsed() {
        let request = ApiRequest::new(
            Method::GET,
            "/users?role=admin&role=user".parse().expect("uri"),
            HeaderMap::new(),
            Vec::new(),
            None,
            TransportSecurity::Secure,
            1024,
        )
        .expect("request");
        assert!(request.query_single_map().is_err());
    }
}
