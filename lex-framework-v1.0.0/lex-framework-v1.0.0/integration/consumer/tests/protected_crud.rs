//! Protected resource lifecycle using the facade on either database.
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
        self as orm, ModelManager, SqliteModelManager, TextField, UUIDField,
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

#[derive(Clone)]
enum Store {
    Sqlite(SqliteModelManager<Project>),
    Postgres(Arc<ModelManager<Project>>),
}
macro_rules! query {
    ($store:expr, $manager:ident => $body:expr) => {
        match $store {
            Store::Sqlite($manager) => $body,
            Store::Postgres($manager) => $body,
        }
    };
}
struct VerifiedTenant(RbacPermissionRepository);
#[async_trait]
impl TenantResolver for VerifiedTenant {
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
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| ApiError::bad_request("invalid_tenant", "invalid tenant"))?;
        let user = principal
            .user_id()
            .ok_or_else(|| ApiError::authentication("authentication required"))?;
        if self.0.membership(user, tenant).await?.is_none() {
            return Err(ApiError::permission_denied("tenant membership required"));
        }
        Ok(Some(tenant))
    }
}
struct Resource(Store);
fn identity(request: &ApiRequest, context: &RequestContext) -> ApiResult<(UUIDField, UUIDField)> {
    let tenant = context
        .tenant_id()
        .ok_or_else(|| ApiError::permission_denied("tenant required"))?;
    let id = request
        .path_param("id")
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| ApiError::bad_request("invalid_id", "invalid project identifier"))?;
    Ok((UUIDField(tenant), UUIDField(id)))
}
#[async_trait]
impl APIView for Resource {
    fn name(&self) -> &'static str {
        "protected_projects"
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
        Permissions::named("projects.manage")
    }
    async fn get(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        _: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        if request.path_param("id").is_some() {
            let (tenant, id) = identity(request, context)?;
            let row = query!(&self.0, manager => manager.queryset().for_tenant(tenant).filter(Project::ID.eq(id)).get().await)?;
            return ApiResponse::json(StatusCode::OK, &row);
        }
        let tenant = UUIDField(
            context
                .tenant_id()
                .ok_or_else(|| ApiError::permission_denied("tenant required"))?,
        );
        let rows = query!(&self.0, manager => manager.queryset().for_tenant(tenant).order_by(Project::TITLE.asc()).all().await)?;
        ApiResponse::json(StatusCode::OK, &rows)
    }
    async fn post(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        _: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        let tenant = UUIDField(
            context
                .tenant_id()
                .ok_or_else(|| ApiError::permission_denied("tenant required"))?,
        );
        let fields = SerializerPolicy::for_model::<Project>()
            .allow_write_fields(&["title"])?
            .validate_input(&request.json()?, SerializerMode::Create)?;
        let title = fields
            .get("title")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::bad_request("invalid_title", "title required"))?;
        let record = Project {
            id: UUIDField::new(),
            tenant_id: tenant.clone(),
            title: TextField::new(title),
            deleted_at: None,
        };
        let row = query!(&self.0, manager => manager.queryset().for_tenant(tenant).create(&record).await)?;
        ApiResponse::json(StatusCode::CREATED, &row)
    }
    async fn put(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.patch(request, context, services).await
    }
    async fn patch(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        _: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        let (tenant, id) = identity(request, context)?;
        let fields = SerializerPolicy::for_model::<Project>()
            .allow_write_fields(&["title"])?
            .validate_input(&request.json()?, SerializerMode::Update)?;
        // This resource has one writable required field, so full and partial updates both
        // require title. Load within the same scope before mutation to preserve 404 semantics.
        if !fields.contains_key("title") {
            return Err(ApiError::bad_request("invalid_title", "title required"));
        }
        query!(&self.0, manager => manager.queryset().for_tenant(tenant.clone()).filter(Project::ID.eq(id.clone())).get().await)?;
        query!(&self.0, manager => manager.queryset().for_tenant(tenant.clone()).filter(Project::ID.eq(id.clone())).update(fields).await)?;
        let saved = query!(&self.0, manager => manager.queryset().for_tenant(tenant).filter(Project::ID.eq(id)).get().await)?;
        ApiResponse::json(StatusCode::OK, &saved)
    }
    async fn delete(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        _: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        let (tenant, id) = identity(request, context)?;
        query!(&self.0, manager => manager.queryset().for_tenant(tenant.clone()).filter(Project::ID.eq(id.clone())).get().await)?;
        query!(&self.0, manager => manager.queryset().for_tenant(tenant).filter(Project::ID.eq(id)).soft_delete().await)?;
        ApiResponse::json(StatusCode::OK, &json!({"deleted":true}))
    }
}
async fn send(
    router: &Router,
    method: Method,
    path: &str,
    tenant: Option<&UUIDField>,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(tenant) = tenant {
        request = request.header("x-tenant", tenant.0.to_string());
    }
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let body = if let Some(body) = body {
        request = request.header("content-type", "application/json");
        Body::from(body.to_string())
    } else {
        Body::empty()
    };
    let response = router
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    (
        status,
        serde_json::from_slice(&to_bytes(response.into_body(), 1_000_000).await.unwrap()).unwrap(),
    )
}
async fn lifecycle(core: Settings) {
    let database = Database::initialize_with(&core).await.unwrap();
    let pg = database.postgres_write_pool().is_some();
    let (store, repository) = if let Some(pool) = database.postgres_write_pool() {
        lex_framework::migration::rbac::install_rbac_postgres(pool)
            .await
            .unwrap();
        orm::sqlx::raw_sql(orm::sqlx::AssertSqlSafe(orm::generate_create_table_sql::<
            Project,
        >()))
        .execute(pool)
        .await
        .unwrap();
        (
            Store::Postgres(Arc::new(Project::objects(pool.clone()))),
            RbacPermissionRepository::postgres(pool.clone()),
        )
    } else {
        let pool = database.sqlite_pool().unwrap();
        lex_framework::migration::rbac::install_rbac_sqlite(pool)
            .await
            .unwrap();
        (
            Store::Sqlite(Project::objects_sqlite_auto(pool.clone()).await.unwrap()),
            RbacPermissionRepository::sqlite(pool.clone()),
        )
    };
    let [user, alpha, beta, member, role, permission] =
        std::array::from_fn::<_, 6, _>(|_| UUIDField::new());
    for (sql, values) in [
        (
            "INSERT INTO users(id,name,email,password) VALUES($1,'Alice','crud@test.test','not-a-login-password')",
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
            vec![UUIDField::new().0, alpha.0, member.0, role.0],
        ),
        (
            "INSERT INTO role_permissions(id,tenant_id,role_id,permission_id) VALUES($1,$2,$3,$4)",
            vec![UUIDField::new().0, alpha.0, role.0, permission.0],
        ),
    ] {
        let mut statement = sql.to_owned();
        if pg {
            for number in 1..=values.len() {
                statement = statement.replace(&format!("${number}"), &format!("${number}::uuid"));
            }
        }
        if let Some(pool) = database.postgres_write_pool() {
            let mut query = orm::sqlx::query(orm::sqlx::AssertSqlSafe(statement));
            for value in values {
                query = query.bind(value.to_string());
            }
            query.execute(pool).await.unwrap();
        } else {
            let mut query = orm::sqlx::query(orm::sqlx::AssertSqlSafe(statement));
            for value in values {
                query = query.bind(value.to_string());
            }
            query
                .execute(database.sqlite_pool().unwrap())
                .await
                .unwrap();
        }
    }
    let beta_row = Project {
        id: UUIDField::new(),
        tenant_id: beta.clone(),
        title: TextField::new("Beta secret"),
        deleted_at: None,
    };
    query!(&store, manager => manager.queryset().for_tenant(beta.clone()).create(&beta_row).await)
        .unwrap();
    let mut settings = FrameworkSettings {
        core,
        ..Default::default()
    };
    settings.security.authentication.backends = vec!["bearer".into()];
    let (auth, service) = settings.authentication(&database).await.unwrap();
    let credential = service
        .create_bearer_token(user.0, ["projects.manage".into()])
        .await
        .unwrap();
    let runtime = RestRuntimeBuilder::new(Arc::new(DatabasePermissionBackend::new(
        Arc::new(repository.clone()),
        DatabasePermissionConfig::default(),
    )))
    .authentication(auth)
    .tenant_resolver(Arc::new(VerifiedTenant(repository)))
    .require_secure_transport(false)
    .build()
    .unwrap();
    let mut routes = RouteTable::new();
    let view = Arc::new(Resource(store.clone()));
    routes
        .register(RouteSpec::http("/projects", "project_list", view.clone()))
        .unwrap();
    routes
        .register(RouteSpec::http("/projects/{id}", "project_detail", view))
        .unwrap();
    let router = AxumAdapter::new(Arc::new(RestApplication::new(routes, runtime)))
        .router()
        .unwrap();
    let token = credential.token.as_str();
    assert_eq!(
        send(&router, Method::GET, "/projects", Some(&alpha), None, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&router, Method::GET, "/projects", None, Some(token), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(
            &router,
            Method::GET,
            "/projects",
            Some(&beta),
            Some(token),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (created, row) = send(
        &router,
        Method::POST,
        "/projects",
        Some(&alpha),
        Some(token),
        Some(json!({"title":"first 世界"})),
    )
    .await;
    assert_eq!(created, StatusCode::CREATED);
    let path = format!("/projects/{}", row["id"].as_str().unwrap());
    assert_eq!(
        send(&router, Method::GET, &path, Some(&alpha), Some(token), None)
            .await
            .1["title"],
        "first 世界"
    );
    for method in [Method::PUT, Method::PATCH] {
        assert_eq!(
            send(
                &router,
                method.clone(),
                &path,
                Some(&alpha),
                Some(token),
                Some(json!({"title":"forged","tenant_id":beta.0}))
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
        let (status, saved) = send(
            &router,
            method,
            &path,
            Some(&alpha),
            Some(token),
            Some(json!({"title":"updated"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(saved["title"], "updated");
    }
    let other = format!("/projects/{}", beta_row.id.0);
    for method in [Method::GET, Method::PATCH, Method::DELETE] {
        let body = if method == Method::PATCH {
            Some(json!({"title":"stolen"}))
        } else {
            None
        };
        assert_eq!(
            send(&router, method, &other, Some(&alpha), Some(token), body)
                .await
                .0,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        send(
            &router,
            Method::DELETE,
            &path,
            Some(&alpha),
            Some(token),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        send(&router, Method::GET, &path, Some(&alpha), Some(token), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(
            &router,
            Method::GET,
            "/projects",
            Some(&alpha),
            Some(token),
            None
        )
        .await
        .1,
        json!([])
    );
    let soft = query!(&store,manager => manager.queryset().for_tenant(alpha.clone()).only_deleted().get().await).unwrap();
    assert_eq!(soft.title.as_str(), "updated");
    let untouched =
        query!(&store,manager => manager.queryset().for_tenant(beta.clone()).get().await).unwrap();
    assert_eq!(untouched.title.as_str(), "Beta secret");
    if let Some(pool) = database.postgres_write_pool() {
        orm::sqlx::query("DELETE FROM membership_roles")
            .execute(pool)
            .await
            .unwrap();
    } else {
        orm::sqlx::query("DELETE FROM membership_roles")
            .execute(database.sqlite_pool().unwrap())
            .await
            .unwrap();
    }
    assert_eq!(
        send(
            &router,
            Method::GET,
            "/projects",
            Some(&alpha),
            Some(token),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    database.close().await;
}
#[tokio::test]
async fn sqlite_member_can_manage_own_projects_but_cannot_read_or_mutate_another_tenant() {
    let mut core = Settings::default();
    core.database.url = "sqlite::memory:".into();
    lifecycle(core).await;
}
#[cfg(feature = "postgres-integration")]
#[tokio::test]
async fn postgres_member_can_manage_own_projects_but_cannot_read_or_mutate_another_tenant() {
    assert_eq!(
        std::env::var("CONFIRM_DISPOSABLE_DATABASE").as_deref(),
        Ok("yes")
    );
    let url = std::env::var("TEST_DATABASE_URL").unwrap();
    let admin = orm::sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("lex_http_{}", UUIDField::new().0.simple());
    orm::sqlx::query(orm::sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .unwrap();
    let separator = if url.contains('?') { '&' } else { '?' };
    let mut core = Settings::default();
    core.database.url = format!("{url}{separator}options=-csearch_path%3D{schema}");
    // The isolated loopback test server has no TLS; deployment defaults stay unchanged.
    core.database.require_ssl = false;
    core.database.run_migrations = false;
    lifecycle(core).await;
    orm::sqlx::query(orm::sqlx::AssertSqlSafe(format!(
        "DROP SCHEMA {schema} CASCADE"
    )))
    .execute(&admin)
    .await
    .unwrap();
}
