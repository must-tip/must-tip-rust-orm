use async_trait::async_trait;
use http::{HeaderMap, HeaderValue, Method, StatusCode};
use lex_rest::rest_framework::{
    authentication::{AuthenticationBackend, AuthenticationChain, AuthenticationOutcome},
    exceptions::ApiResult,
    middleware::{RestApplication, RestRuntimeBuilder, SecurityHeadersMiddleware},
    permissions::{PermissionBackend, PermissionDecision, PermissionResource},
    request::{ApiRequest, Principal, RequestContext, TransportSecurity},
    response::ApiResponse,
    routing::{RouteSpec, RouteTable},
    views::{APIView, ViewServices},
};
use std::sync::Arc;
use uuid::Uuid;

struct HeaderAuthentication;

#[async_trait]
impl AuthenticationBackend for HeaderAuthentication {
    fn name(&self) -> &'static str {
        "test_bearer"
    }

    async fn authenticate(&self, request: &ApiRequest) -> ApiResult<AuthenticationOutcome> {
        let Some(value) = request.headers().get(http::header::AUTHORIZATION) else {
            return Ok(AuthenticationOutcome::Abstain);
        };
        if value.as_bytes() != b"Bearer good" {
            return Ok(AuthenticationOutcome::Rejected {
                safe_detail: "invalid credentials",
            });
        }
        Ok(AuthenticationOutcome::Authenticated(Principal::user(
            Uuid::from_u128(1),
            "test_bearer",
            Vec::new(),
        )?))
    }

    fn challenge(&self) -> Option<String> {
        Some("Bearer realm=\"test\"".to_owned())
    }
}

struct NoPrivileges;

#[async_trait]
impl PermissionBackend for NoPrivileges {
    async fn has_permission(
        &self,
        _context: &RequestContext,
        _permission: &str,
    ) -> ApiResult<PermissionDecision> {
        Ok(PermissionDecision::deny("not_granted"))
    }

    async fn has_object_permission(
        &self,
        _context: &RequestContext,
        _permission: &str,
        _resource: &PermissionResource,
    ) -> ApiResult<PermissionDecision> {
        Ok(PermissionDecision::deny("not_granted"))
    }

    async fn is_tenant_member(&self, _context: &RequestContext) -> ApiResult<bool> {
        Ok(false)
    }

    async fn is_tenant_admin(&self, _context: &RequestContext) -> ApiResult<bool> {
        Ok(false)
    }

    async fn is_global_admin(&self, _context: &RequestContext) -> ApiResult<bool> {
        Ok(false)
    }
}

struct ProtectedView;

#[async_trait]
impl APIView for ProtectedView {
    fn name(&self) -> &'static str {
        "protected"
    }

    fn allowed_methods(&self) -> Vec<Method> {
        vec![Method::GET]
    }

    async fn get(
        &self,
        _request: &ApiRequest,
        _context: &RequestContext,
        _services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        Ok(ApiResponse::empty(StatusCode::NO_CONTENT))
    }
}

fn application() -> RestApplication {
    let authentication = AuthenticationChain::new(vec![Arc::new(HeaderAuthentication)])
        .expect("authentication chain");
    let runtime = RestRuntimeBuilder::new(Arc::new(NoPrivileges))
        .authentication(authentication)
        .middleware(vec![Arc::new(SecurityHeadersMiddleware::default())])
        .build()
        .expect("runtime");
    let mut routes = RouteTable::new();
    routes
        .register(RouteSpec::http(
            "/protected",
            "protected",
            Arc::new(ProtectedView),
        ))
        .expect("route");
    RestApplication::new(routes, runtime)
}

fn request(
    method: Method,
    path: &str,
    authorization: Option<&'static str>,
    secure: bool,
) -> ApiRequest {
    let mut headers = HeaderMap::new();
    if let Some(value) = authorization {
        headers.insert(http::header::AUTHORIZATION, HeaderValue::from_static(value));
    }
    ApiRequest::new(
        method,
        path.parse().expect("uri"),
        headers,
        Vec::new(),
        None,
        if secure {
            TransportSecurity::Secure
        } else {
            TransportSecurity::Insecure
        },
        1024,
    )
    .expect("request")
}

#[tokio::test]
async fn anonymous_request_is_challenged_and_security_headers_are_present() {
    let response = application()
        .handle(request(Method::GET, "/protected", None, true))
        .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        response
            .headers()
            .contains_key(http::header::WWW_AUTHENTICATE)
    );
    assert!(response.headers().contains_key("x-request-id"));
    assert_eq!(
        response.headers().get("x-content-type-options"),
        Some(&HeaderValue::from_static("nosniff"))
    );
    assert!(response.headers().contains_key("strict-transport-security"));
}

#[tokio::test]
async fn invalid_presented_credentials_fail_closed() {
    let response = application()
        .handle(request(
            Method::GET,
            "/protected",
            Some("Bearer invalid"),
            true,
        ))
        .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        response
            .headers()
            .contains_key(http::header::WWW_AUTHENTICATE)
    );
}

#[tokio::test]
async fn authenticated_request_reaches_view() {
    let response = application()
        .handle(request(
            Method::GET,
            "/protected",
            Some("Bearer good"),
            true,
        ))
        .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(response.headers().contains_key("x-request-id"));
}

#[tokio::test]
async fn routing_errors_still_receive_security_headers_and_request_id() {
    let response = application()
        .handle(request(Method::GET, "/missing", None, true))
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(response.headers().contains_key("x-request-id"));
    assert!(response.headers().contains_key("content-security-policy"));
}

#[tokio::test]
async fn method_not_allowed_reports_allow_header() {
    let response = application()
        .handle(request(Method::POST, "/protected", None, true))
        .await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    let allow = response
        .headers()
        .get(http::header::ALLOW)
        .and_then(|value| value.to_str().ok())
        .expect("allow header");
    assert!(allow.split(',').any(|method| method.trim() == "GET"));
}

#[tokio::test]
async fn insecure_transport_is_rejected_without_hsts() {
    let response = application()
        .handle(request(
            Method::GET,
            "/protected",
            Some("Bearer good"),
            false,
        ))
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(response.headers().contains_key("x-request-id"));
    assert!(!response.headers().contains_key("strict-transport-security"));
    assert!(response.headers().contains_key("x-content-type-options"));
}
