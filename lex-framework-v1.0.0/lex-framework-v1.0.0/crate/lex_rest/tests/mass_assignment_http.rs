#![cfg(feature = "axum")]
use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use db_controller::{SerializerMode, SerializerPolicy};
use http::{Method, Request, StatusCode};
use lex_rest::rest_framework::{
    adapters::axum::AxumAdapter,
    exceptions::ApiResult,
    middleware::{RestApplication, RestRuntimeBuilder},
    permissions::DenyAllPermissionBackend,
    request::{ApiRequest, RequestContext},
    response::ApiResponse,
    routing::{RouteSpec, RouteTable},
    views::{APIView, ViewAccess, ViewServices},
};
use std::sync::Arc;
use tower::ServiceExt;
struct Profile;
#[async_trait]
impl APIView for Profile {
    fn name(&self) -> &'static str {
        "profile-write-policy"
    }
    fn access(&self) -> ViewAccess {
        ViewAccess::Public
    }
    fn allowed_methods(&self) -> Vec<Method> {
        vec![Method::PATCH]
    }
    async fn patch(
        &self,
        r: &ApiRequest,
        _: &RequestContext,
        _: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        let fields = SerializerPolicy::for_model::<models::User>()
            .allow_write_fields(&["name"])?
            .validate_input(&r.json()?, SerializerMode::PartialUpdate)?;
        ApiResponse::json(StatusCode::OK, &fields)
    }
}
#[tokio::test]
async fn model_endpoint_rejects_privilege_and_ownership_fields_supplied_by_clients() {
    let mut routes = RouteTable::new();
    routes
        .register(RouteSpec::http("/profile", "profile", Arc::new(Profile)))
        .unwrap();
    let runtime = RestRuntimeBuilder::new(Arc::new(DenyAllPermissionBackend))
        .require_secure_transport(false)
        .build()
        .unwrap();
    let router = AxumAdapter::new(Arc::new(RestApplication::new(routes, runtime)))
        .router()
        .unwrap();
    for field in [
        "id",
        "tenant_id",
        "owner_id",
        "is_admin",
        "is_global_admin",
        "role",
        "permissions",
        "deleted_at",
        "version",
        "password",
        "email",
    ] {
        let payload = serde_json::json!({field:"attacker", "name":"safe"});
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/profile")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{field}");
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(error["fields"][field].is_array(), "{field}: {error}");
    }
    let response = router
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/profile")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"name":"世界"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        SerializerPolicy::for_model::<models::User>()
            .allow_write_fields(&["is_global_admin"])
            .is_err()
    );
    assert!(
        SerializerPolicy::for_model::<models::User>()
            .allow_write_fields(&["missing"])
            .is_err()
    );
}
