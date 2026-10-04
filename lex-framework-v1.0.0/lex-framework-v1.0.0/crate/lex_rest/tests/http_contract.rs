#![cfg(feature = "axum")]

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use http::{Method, Request, StatusCode};
use lex_rest::rest_framework::{
    adapters::axum::AxumAdapter,
    exceptions::ApiResult,
    middleware::{RestApplication, RestRuntimeBuilder, SecurityHeadersMiddleware},
    permissions::DenyAllPermissionBackend,
    request::{ApiRequest, RequestContext, TransportSecurity},
    response::ApiResponse,
    routing::{RouteSpec, RouteTable},
    views::{APIView, ViewAccess, ViewServices},
};
use std::sync::Arc;
use tower::ServiceExt;

struct Echo;

#[async_trait]
impl APIView for Echo {
    fn name(&self) -> &'static str {
        "echo"
    }
    fn access(&self) -> ViewAccess {
        ViewAccess::Public
    }
    fn allowed_methods(&self) -> Vec<Method> {
        vec![Method::POST]
    }
    async fn post(
        &self,
        request: &ApiRequest,
        _: &RequestContext,
        _: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        let data: serde_json::Value = request.json()?;
        ApiResponse::json(StatusCode::CREATED, &data)
    }
}

fn adapter() -> AxumAdapter {
    let mut routes = RouteTable::new();
    routes
        .register(RouteSpec::http("/echo", "echo", Arc::new(Echo)))
        .unwrap();
    let runtime = RestRuntimeBuilder::new(Arc::new(DenyAllPermissionBackend))
        .require_secure_transport(false)
        .middleware(vec![Arc::new(SecurityHeadersMiddleware::default())])
        .build()
        .unwrap();
    AxumAdapter::new(Arc::new(RestApplication::new(routes, runtime)))
}

#[tokio::test]
async fn public_endpoint_returns_json_through_the_actual_http_router() {
    let router = adapter().router().unwrap();
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/echo")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"title":"Hello 世界"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        response.headers()["content-type"],
        "application/json; charset=utf-8"
    );
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["title"],
        "Hello 世界"
    );
}

#[tokio::test]
async fn malformed_json_and_wrong_content_type_have_consistent_http_errors() {
    for (content_type, body, expected) in [
        ("application/json", "{", StatusCode::BAD_REQUEST),
        ("text/plain", "{}", StatusCode::UNSUPPORTED_MEDIA_TYPE),
    ] {
        let response = adapter()
            .router()
            .unwrap()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/echo")
                    .header("content-type", content_type)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(error["code"].is_string());
        assert!(error["detail"].is_string());
        assert!(error["request_id"].is_string());
    }
}

#[tokio::test]
async fn unsupported_method_reports_the_allowed_methods() {
    let response = adapter()
        .router()
        .unwrap()
        .oneshot(Request::builder().uri("/echo").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert!(
        response.headers()["allow"]
            .to_str()
            .unwrap()
            .contains("POST")
    );
}

#[tokio::test]
async fn plain_http_is_not_assumed_to_be_tls_by_default() {
    let response = adapter()
        .router()
        .unwrap()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/echo")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(!response.headers().contains_key("strict-transport-security"));
}

#[tokio::test]
async fn explicitly_secure_transport_adds_hsts() {
    let response = adapter()
        .transport_security(TransportSecurity::Secure)
        .router()
        .unwrap()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/echo")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.headers().contains_key("strict-transport-security"));
}
