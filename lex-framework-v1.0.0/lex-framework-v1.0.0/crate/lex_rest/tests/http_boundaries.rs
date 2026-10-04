#![cfg(feature = "axum")]
use async_trait::async_trait;
use axum::{Router, body::Body};
use http::{Request, StatusCode};
use lex_rest::rest_framework::{
    adapters::axum::AxumAdapter,
    exceptions::ApiResult,
    middleware::{
        CorsMiddleware, RequestLimitsMiddleware, RequestMiddleware, RestApplication,
        RestRuntimeBuilder,
    },
    permissions::DenyAllPermissionBackend,
    request::{ApiRequest, RequestContext},
    response::ApiResponse,
    routing::{RouteSpec, RouteTable},
    views::{APIView, ViewAccess, ViewServices},
};
use std::sync::Arc;
use tower::ServiceExt;

struct Ready;
#[async_trait]
impl APIView for Ready {
    fn name(&self) -> &'static str {
        "ready"
    }
    fn allowed_methods(&self) -> Vec<http::Method> {
        vec![http::Method::GET]
    }
    fn access(&self) -> ViewAccess {
        ViewAccess::Public
    }
    async fn get(
        &self,
        _: &ApiRequest,
        _: &RequestContext,
        _: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        let mut response = ApiResponse::json(StatusCode::OK, &serde_json::json!({"ready":true}))?;
        response.headers_mut().insert(
            http::header::VARY,
            http::HeaderValue::from_static("Accept-Encoding"),
        );
        Ok(response)
    }
}
fn adapter(middleware: Vec<Arc<dyn RequestMiddleware>>) -> AxumAdapter {
    let mut routes = RouteTable::new();
    routes
        .register(RouteSpec::http("/ready", "ready", Arc::new(Ready)))
        .unwrap();
    let runtime = RestRuntimeBuilder::new(Arc::new(DenyAllPermissionBackend))
        .require_secure_transport(false)
        .middleware(middleware)
        .build()
        .unwrap();
    AxumAdapter::new(Arc::new(RestApplication::new(routes, runtime)))
}
fn cors(credentials: bool) -> CorsMiddleware {
    CorsMiddleware::new(
        vec!["https://app.example.test".into()],
        vec!["GET".into()],
        vec!["authorization".into()],
        vec![],
        credentials,
        60,
    )
    .unwrap()
}
async fn preflight(
    router: &Router,
    origin: &str,
    method: &str,
    header: &str,
) -> axum::response::Response {
    router
        .clone()
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/ready")
                .header("origin", origin)
                .header("access-control-request-method", method)
                .header("access-control-request-headers", header)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}
#[tokio::test]
async fn cors_preflight_rejects_unapproved_origins_methods_and_headers() {
    let router = adapter(vec![Arc::new(cors(true))]).router().unwrap();
    let allowed = preflight(&router, "https://app.example.test", "GET", "Authorization").await;
    assert!(allowed.status().is_success());
    assert_eq!(
        allowed.headers()["access-control-allow-origin"],
        "https://app.example.test"
    );
    assert_eq!(
        allowed.headers()["access-control-allow-credentials"],
        "true"
    );
    for (origin, method, header) in [
        ("https://attacker.example", "GET", "authorization"),
        ("https://app.example.test", "DELETE", "authorization"),
        ("https://app.example.test", "GET", "x-admin"),
    ] {
        assert_eq!(
            preflight(&router, origin, method, header).await.status(),
            StatusCode::FORBIDDEN
        );
    }
}
#[tokio::test]
async fn cors_keeps_existing_cache_variation_and_omits_credentials_when_disabled() {
    let response = adapter(vec![Arc::new(cors(false))])
        .router()
        .unwrap()
        .oneshot(
            Request::builder()
                .uri("/ready")
                .header("origin", "https://app.example.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        !response
            .headers()
            .contains_key("access-control-allow-credentials")
    );
    let vary = response
        .headers()
        .get_all(http::header::VARY)
        .iter()
        .map(|value| value.to_str().unwrap())
        .collect::<Vec<_>>();
    assert!(vary.contains(&"Accept-Encoding") && vary.contains(&"Origin"));
}
#[test]
fn malformed_cors_configuration_is_rejected_before_requests_are_served() {
    for origin in [
        "https://example.test/path",
        "https://user:password@example.test",
        "not an origin",
    ] {
        assert!(
            CorsMiddleware::new(
                vec![origin.into()],
                vec!["GET".into()],
                vec![],
                vec![],
                false,
                60
            )
            .is_err()
        );
    }
    assert!(
        CorsMiddleware::new(
            vec!["*".into()],
            vec!["GET".into()],
            vec![],
            vec![],
            true,
            60
        )
        .is_err()
    );
    assert!(
        CorsMiddleware::new(
            vec!["*".into()],
            vec!["GET\r\nX-Evil: yes".into()],
            vec![],
            vec![],
            false,
            60
        )
        .is_err()
    );
}
#[tokio::test]
async fn request_limits_accept_exact_boundaries_and_reject_oversized_requests() {
    let router = adapter(vec![Arc::new(
        RequestLimitsMiddleware::new(256, 4, 1024, 256).unwrap(),
    )])
    .max_body_bytes(8)
    .unwrap()
    .router()
    .unwrap();
    for (extra, expected) in [
        (247, StatusCode::OK),
        (248, StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE),
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/ready")
                    .header("x-padding", "a".repeat(extra))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    for (padding, expected) in [(247, StatusCode::OK), (248, StatusCode::URI_TOO_LONG)] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/ready?q={}", "a".repeat(padding)))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/ready")
                .header("a", "1")
                .header("b", "2")
                .header("c", "3")
                .header("d", "4")
                .header("e", "5")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE
    );
    for (bytes, expected) in [(8, StatusCode::OK), (9, StatusCode::PAYLOAD_TOO_LARGE)] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/ready")
                    .body(Body::from(vec![b'a'; bytes]))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    let cumulative = adapter(vec![Arc::new(
        RequestLimitsMiddleware::new(256, 8, 1024, 512).unwrap(),
    )])
    .router()
    .unwrap();
    let response = cumulative
        .oneshot(
            Request::builder()
                .uri("/ready")
                .header("a", "x".repeat(511))
                .header("b", "x".repeat(511))
                .header("c", "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE
    );
}
