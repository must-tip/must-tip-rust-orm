#![cfg(feature = "axum")]
use async_trait::async_trait;
use axum::{Router, body::Body};
use http::{Method, Request, StatusCode};
use lex_framework::lex_rest::rest_framework::{
    adapters::axum::AxumAdapter,
    exceptions::ApiResult,
    middleware::{RestApplication, RestRuntimeBuilder},
    permissions::{Permissions, ScopePermissionBackend},
    request::{ApiRequest, RequestContext},
    response::ApiResponse,
    routing::{RouteSpec, RouteTable},
    views::{APIView, ViewServices},
};
use lex_framework::{
    FrameworkSettings,
    db::{Database, Settings},
};
use std::sync::Arc;
use tower::ServiceExt;

struct Protected;
#[async_trait]
impl APIView for Protected {
    fn name(&self) -> &'static str {
        "protected"
    }
    fn allowed_methods(&self) -> Vec<Method> {
        vec![
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ]
    }
    fn permission_policy(
        &self,
        _: &Method,
    ) -> lex_framework::lex_rest::rest_framework::permissions::PermissionExpr {
        Permissions::named("projects.read")
    }
    async fn get(
        &self,
        _: &ApiRequest,
        _: &RequestContext,
        _: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        ApiResponse::json(StatusCode::OK, &serde_json::json!({"ok":true}))
    }
    async fn post(
        &self,
        r: &ApiRequest,
        c: &RequestContext,
        s: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.get(r, c, s).await
    }
    async fn put(
        &self,
        r: &ApiRequest,
        c: &RequestContext,
        s: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.get(r, c, s).await
    }
    async fn patch(
        &self,
        r: &ApiRequest,
        c: &RequestContext,
        s: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.get(r, c, s).await
    }
    async fn delete(
        &self,
        r: &ApiRequest,
        c: &RequestContext,
        s: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.get(r, c, s).await
    }
}
async fn send(router: &Router, method: Method, credentials: &[(&str, String)]) -> StatusCode {
    let mut request = Request::builder().method(method).uri("/projects");
    for (name, value) in credentials {
        request = request.header(*name, value);
    }
    router
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}
#[tokio::test]
async fn protected_http_endpoint_rejects_expired_revoked_ambiguous_and_unscoped_credentials() {
    let mut core = Settings::default();
    core.database.url = "sqlite::memory:".into();
    let database = Database::initialize_with(&core).await.unwrap();
    let mut settings = FrameworkSettings {
        core: core.clone(),
        media: lex_framework::media::MediaSettings::default(),
        security: lex_framework::SecuritySettings::default(),
    };
    settings.security.authentication.backends = vec!["session".into(), "bearer".into()];
    let (chain, service) = settings.authentication(&database).await.unwrap();
    let mut routes = RouteTable::new();
    routes
        .register(RouteSpec::http(
            "/projects",
            "projects",
            Arc::new(Protected),
        ))
        .unwrap();
    let runtime = RestRuntimeBuilder::new(Arc::new(ScopePermissionBackend::default()))
        .authentication(chain)
        .require_secure_transport(false)
        .build()
        .unwrap();
    let router = AxumAdapter::new(Arc::new(RestApplication::new(routes, runtime)))
        .router()
        .unwrap();
    let user = uuid::Uuid::new_v4();
    let session = service
        .create_session(user, ["projects.read".into()])
        .await
        .unwrap();
    let bearer = service
        .create_bearer_token(user, ["projects.read".into()])
        .await
        .unwrap();
    let cookie = [("cookie", format!("lex_session={}", session.token))];
    let authorization = [("authorization", format!("Bearer {}", bearer.token))];
    assert_eq!(
        send(&router, Method::GET, &[]).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(send(&router, Method::GET, &cookie).await, StatusCode::OK);
    assert_eq!(
        send(&router, Method::GET, &authorization).await,
        StatusCode::OK
    );
    for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
        assert_eq!(
            send(&router, method.clone(), &cookie).await,
            StatusCode::FORBIDDEN
        );
        let csrf = [
            ("cookie", cookie[0].1.clone()),
            ("x-csrf-token", session.csrf_token.clone()),
        ];
        assert_eq!(send(&router, method.clone(), &csrf).await, StatusCode::OK);
        let wrong_csrf = [
            ("cookie", cookie[0].1.clone()),
            ("x-csrf-token", "wrong-token".into()),
        ];
        assert_eq!(
            send(&router, method.clone(), &wrong_csrf).await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(send(&router, method, &authorization).await, StatusCode::OK);
    }
    let both = [
        ("cookie", cookie[0].1.clone()),
        ("authorization", authorization[0].1.clone()),
    ];
    assert_eq!(
        send(&router, Method::GET, &both).await,
        StatusCode::UNAUTHORIZED
    );
    let invalid_later = [
        ("cookie", cookie[0].1.clone()),
        ("authorization", "Bearer invalid".into()),
    ];
    assert_eq!(
        send(&router, Method::GET, &invalid_later).await,
        StatusCode::UNAUTHORIZED
    );
    let unscoped = service.create_bearer_token(user, []).await.unwrap();
    assert_eq!(
        send(
            &router,
            Method::GET,
            &[("authorization", format!("Bearer {}", unscoped.token))]
        )
        .await,
        StatusCode::FORBIDDEN
    );
    let pool = database.sqlite_pool().unwrap();
    let stored: String = lex_framework::db_controller::sqlx::query_scalar(
        "SELECT token_hash FROM _lex_auth_sessions",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_ne!(stored, session.token);
    assert_eq!(stored.len(), 64);
    assert!(!format!("{session:?} {bearer:?}").contains(&session.token));
    assert!(!format!("{bearer:?}").contains(&bearer.token));
    service.revoke_bearer_token(&bearer.token).await.unwrap();
    assert_eq!(
        send(&router, Method::GET, &authorization).await,
        StatusCode::UNAUTHORIZED
    );
    lex_framework::db_controller::sqlx::query("UPDATE _lex_auth_sessions SET expires_at_epoch=0")
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        send(&router, Method::GET, &cookie).await,
        StatusCode::UNAUTHORIZED
    );
    database.close().await;
}

#[cfg(feature = "cookie-integration")]
#[tokio::test]
async fn signed_cookie_http_authentication_rejects_tampering_and_requires_csrf() {
    assert!(
        std::env::var("LEX_TEST_COOKIE_SECRET")
            .expect("set a disposable test-only cookie secret")
            .len()
            >= 32
    );
    let mut core = Settings::default();
    core.database.url = "sqlite::memory:".into();
    let database = Database::initialize_with(&core).await.unwrap();
    let mut settings = FrameworkSettings {
        core,
        ..Default::default()
    };
    settings.security.authentication.backends = vec!["cookie".into()];
    settings.security.authentication.cookie.secret_env = "LEX_TEST_COOKIE_SECRET".into();
    let (authentication, service) = settings.authentication(&database).await.unwrap();
    let issued = service
        .create_cookie(uuid::Uuid::new_v4(), ["projects.read".into()])
        .unwrap();
    assert!(!format!("{issued:?}").contains(&issued.value));
    let mut routes = RouteTable::new();
    routes
        .register(RouteSpec::http(
            "/projects",
            "projects",
            Arc::new(Protected),
        ))
        .unwrap();
    let runtime = RestRuntimeBuilder::new(Arc::new(ScopePermissionBackend::default()))
        .authentication(authentication)
        .require_secure_transport(false)
        .build()
        .unwrap();
    let router = AxumAdapter::new(Arc::new(RestApplication::new(routes, runtime)))
        .router()
        .unwrap();
    let cookie = [("cookie", format!("lex_auth={}", issued.value))];
    assert_eq!(send(&router, Method::GET, &cookie).await, StatusCode::OK);
    for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
        assert_eq!(
            send(&router, method.clone(), &cookie).await,
            StatusCode::FORBIDDEN
        );
        let csrf = [
            ("cookie", cookie[0].1.clone()),
            ("x-csrf-token", issued.csrf_token.clone()),
        ];
        assert_eq!(send(&router, method.clone(), &csrf).await, StatusCode::OK);
        let wrong = [
            ("cookie", cookie[0].1.clone()),
            ("x-csrf-token", "wrong-token".into()),
        ];
        assert_eq!(send(&router, method, &wrong).await, StatusCode::FORBIDDEN);
    }
    let (payload, signature) = issued.value.split_once('.').unwrap();
    let mut tampered = payload.as_bytes().to_vec();
    tampered[0] = if tampered[0] == b'A' { b'B' } else { b'A' };
    for value in [
        format!("{}.{}", String::from_utf8(tampered).unwrap(), signature),
        format!("{payload}.invalid-signature"),
        "malformed".into(),
    ] {
        assert_eq!(
            send(
                &router,
                Method::GET,
                &[("cookie", format!("lex_auth={value}"))]
            )
            .await,
            StatusCode::UNAUTHORIZED
        );
    }
    let unscoped = service.create_cookie(uuid::Uuid::new_v4(), []).unwrap();
    assert_eq!(
        send(
            &router,
            Method::GET,
            &[("cookie", format!("lex_auth={}", unscoped.value))]
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert!(
        service
            .create_session(uuid::Uuid::new_v4(), [])
            .await
            .is_err()
    );
    assert!(
        service
            .create_bearer_token(uuid::Uuid::new_v4(), [])
            .await
            .is_err()
    );
    database.close().await;
}
