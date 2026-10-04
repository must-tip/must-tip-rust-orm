//! A SaaS application built through the public facade, without workspace-only imports.
use async_trait::async_trait;
use axum::{
    Router,
    body::{Body, to_bytes},
};
use http::{Method, Request, StatusCode};
use lex_developer_acceptance::Project;
use lex_framework::{
    FrameworkSettings,
    db::{Database, Settings},
    db_controller::{
        self as orm, SqliteModelManager, TextField, UUIDField,
        rest::{SerializerMode, SerializerPolicy},
    },
    lex_rest::rest_framework::{
        adapters::axum::AxumAdapter,
        exceptions::{ApiError, ApiResult},
        middleware::{RestApplication, RestRuntimeBuilder, TenantResolver},
        permissions::{
            DatabasePermissionBackend, DatabasePermissionConfig, PermissionRepository, Permissions,
            RbacPermissionRepository,
        },
        request::{ApiRequest, Principal, RequestContext},
        response::ApiResponse,
        routing::{RouteSpec, RouteTable},
        views::{APIView, ViewServices},
    },
};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

struct MembershipTenant(RbacPermissionRepository);
#[async_trait]
impl TenantResolver for MembershipTenant {
    async fn resolve_tenant(
        &self,
        request: &ApiRequest,
        principal: &Principal,
    ) -> ApiResult<Option<orm::prelude::Uuid>> {
        let Some(header) = request.headers().get("x-tenant") else {
            return Ok(None);
        };
        let tenant = header
            .to_str()
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| ApiError::bad_request("invalid_tenant", "invalid tenant identifier"))?;
        let user = principal
            .user_id()
            .ok_or_else(|| ApiError::authentication("authentication required"))?;
        if self.0.membership(user, tenant).await?.is_none() {
            return Err(ApiError::permission_denied("tenant membership required"));
        }
        Ok(Some(tenant))
    }
}
struct Projects(SqliteModelManager<Project>);
#[async_trait]
impl APIView for Projects {
    fn name(&self) -> &'static str {
        "projects"
    }
    fn allowed_methods(&self) -> Vec<Method> {
        vec![Method::GET, Method::POST]
    }
    fn permission_policy(
        &self,
        _: &Method,
    ) -> lex_framework::lex_rest::rest_framework::permissions::PermissionExpr {
        Permissions::named("projects.manage")
    }
    async fn get(
        &self,
        _: &ApiRequest,
        context: &RequestContext,
        _: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        let tenant = context
            .tenant_id()
            .ok_or_else(|| ApiError::permission_denied("tenant required"))?;
        let rows = self
            .0
            .queryset()
            .for_tenant(UUIDField(tenant))
            .all()
            .await?;
        ApiResponse::json(StatusCode::OK, &rows)
    }
    async fn post(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        _: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        let tenant = context
            .tenant_id()
            .ok_or_else(|| ApiError::permission_denied("tenant required"))?;
        let input = SerializerPolicy::for_model::<Project>()
            .allow_write_fields(&["title"])?
            .validate_input(&request.json()?, SerializerMode::Create)?;
        let title = input
            .get("title")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::bad_request("invalid_title", "title is required"))?;
        let row = self
            .0
            .queryset()
            .for_tenant(UUIDField(tenant))
            .create(&Project {
                id: UUIDField::new(),
                tenant_id: UUIDField(tenant),
                title: TextField::new(title),
                deleted_at: None,
            })
            .await?;
        ApiResponse::json(StatusCode::CREATED, &row)
    }
}
async fn send(
    router: &Router,
    method: Method,
    tenant: Option<UUIDField>,
    token: Option<&str>,
    payload: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri("/projects");
    if let Some(tenant) = tenant {
        request = request.header("x-tenant", tenant.0.to_string());
    }
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let body = if let Some(payload) = payload {
        request = request.header("content-type", "application/json");
        Body::from(payload.to_string())
    } else {
        Body::empty()
    };
    let response = router
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    assert_eq!(
        response.headers()["content-type"]
            .to_str()
            .unwrap()
            .split(';')
            .next(),
        Some("application/json")
    );
    (
        status,
        serde_json::from_slice(&to_bytes(response.into_body(), 1_000_000).await.unwrap()).unwrap(),
    )
}
#[tokio::test]
async fn tenant_api_rejects_cross_tenant_payloads_and_revoked_roles_using_the_same_token() {
    let mut core = Settings::default();
    core.database.url = "sqlite::memory:".into();
    let database = Database::initialize_with(&core).await.unwrap();
    let pool = database.sqlite_pool().unwrap();
    lex_framework::migration::rbac::install_rbac_sqlite(pool)
        .await
        .unwrap();
    let [
        user,
        alpha,
        beta,
        member,
        role,
        permission,
        assignment,
        grant,
    ] = std::array::from_fn::<_, 8, _>(|_| UUIDField::new());
    // The application's trusted bootstrap creates identities, never a request payload.
    for (sql, values) in [
        (
            "INSERT INTO users(id,name,email,password) VALUES($1,'Alice','alice@example.test','not-a-login-password')",
            vec![user.0],
        ),
        (
            "INSERT INTO tenants(id,name) VALUES($1,'Alpha'),($2,'Beta')",
            vec![alpha.0, beta.0],
        ),
        (
            "INSERT INTO memberships(id,user_id,tenant_id) VALUES($1,$2,$3)",
            vec![member.0, user.0, alpha.0],
        ),
        (
            "INSERT INTO tenant_roles(id,tenant_id,name) VALUES($1,$2,'editor')",
            vec![role.0, alpha.0],
        ),
        (
            "INSERT INTO permission_definitions(id,name) VALUES($1,'projects.manage')",
            vec![permission.0],
        ),
        (
            "INSERT INTO membership_roles(id,tenant_id,membership_id,role_id) VALUES($1,$2,$3,$4)",
            vec![assignment.0, alpha.0, member.0, role.0],
        ),
        (
            "INSERT INTO role_permissions(id,tenant_id,role_id,permission_id) VALUES($1,$2,$3,$4)",
            vec![grant.0, alpha.0, role.0, permission.0],
        ),
    ] {
        let mut query = orm::sqlx::query(orm::sqlx::AssertSqlSafe(sql));
        for value in values {
            query = query.bind(value.to_string());
        }
        query.execute(pool).await.unwrap();
    }
    let manager = Project::objects_sqlite_auto(pool.clone()).await.unwrap();
    manager
        .queryset()
        .for_tenant(beta.clone())
        .create(&Project {
            id: UUIDField::new(),
            tenant_id: beta.clone(),
            title: TextField::new("Beta secret"),
            deleted_at: None,
        })
        .await
        .unwrap();
    let mut settings = FrameworkSettings {
        core,
        ..Default::default()
    };
    settings.security.authentication.backends = vec!["bearer".into()];
    let (authentication, service) = settings.authentication(&database).await.unwrap();
    let credential = service
        .create_bearer_token(user.0, ["projects.manage".into()])
        .await
        .unwrap();
    let repository = RbacPermissionRepository::sqlite(pool.clone());
    let runtime = RestRuntimeBuilder::new(Arc::new(DatabasePermissionBackend::new(
        Arc::new(repository.clone()),
        DatabasePermissionConfig::default(),
    )))
    .authentication(authentication)
    .tenant_resolver(Arc::new(MembershipTenant(repository)))
    .require_secure_transport(false)
    .build()
    .unwrap();
    let mut routes = RouteTable::new();
    routes
        .register(RouteSpec::http(
            "/projects",
            "projects",
            Arc::new(Projects(manager.clone())),
        ))
        .unwrap();
    let router = AxumAdapter::new(Arc::new(RestApplication::new(routes, runtime)))
        .router()
        .unwrap();
    assert_eq!(
        send(&router, Method::GET, Some(alpha.clone()), None, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&router, Method::GET, None, Some(&credential.token), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(
            &router,
            Method::GET,
            Some(beta.clone()),
            Some(&credential.token),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(
            &router,
            Method::POST,
            Some(alpha.clone()),
            Some(&credential.token),
            Some(json!({"title":"forged","tenant_id":beta.0}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let hostile = "'; DROP TABLE acceptance_projects; -- 世界";
    assert_eq!(
        send(
            &router,
            Method::POST,
            Some(alpha.clone()),
            Some(&credential.token),
            Some(json!({"title":hostile}))
        )
        .await
        .0,
        StatusCode::CREATED
    );
    let (status, rows) = send(
        &router,
        Method::GET,
        Some(alpha.clone()),
        Some(&credential.token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["title"], hostile);
    orm::sqlx::query("DELETE FROM role_permissions")
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        send(
            &router,
            Method::GET,
            Some(alpha.clone()),
            Some(&credential.token),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        manager
            .queryset()
            .for_tenant(beta)
            .get()
            .await
            .unwrap()
            .title
            .as_str(),
        "Beta secret"
    );
    database.close().await;
}
