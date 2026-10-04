//! Intentionally public, development-only, loopback API with an ephemeral SQLite database.
use async_trait::async_trait;
use http::{Method, StatusCode};
use lex_framework::{
    db_controller::{
        SqliteModelManager, TextField, UUIDField,
        rest::{SerializerMode, SerializerPolicy},
        sqlx,
    },
    lex_rest::rest_framework::{
        adapters::axum::AxumAdapter,
        exceptions::{ApiError, ApiResult},
        middleware::{
            RequestLimitsMiddleware, RestApplication, RestRuntimeBuilder, SecurityHeadersMiddleware,
        },
        permissions::DenyAllPermissionBackend,
        request::{ApiRequest, RequestContext},
        response::ApiResponse,
        routing::{RouteSpec, RouteTable},
        views::{APIView, ViewAccess, ViewServices},
    },
    prelude::*,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "notes")]
#[model(global)]
struct Note {
    #[field(primary_key)]
    id: UUIDField,
    #[field(not_empty, max_length = 200)]
    title: TextField,
}
struct Notes(SqliteModelManager<Note>);
#[async_trait]
impl APIView for Notes {
    fn name(&self) -> &'static str {
        "notes"
    }
    fn access(&self) -> ViewAccess {
        ViewAccess::Public
    }
    fn allowed_methods(&self) -> Vec<Method> {
        vec![Method::GET, Method::POST]
    }
    async fn get(
        &self,
        _: &ApiRequest,
        _: &RequestContext,
        _: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        ApiResponse::json(StatusCode::OK, &self.0.queryset().all().await?)
    }
    async fn post(
        &self,
        request: &ApiRequest,
        _: &RequestContext,
        _: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        let input = SerializerPolicy::for_model::<Note>()
            .allow_write_fields(&["title"])?
            .validate_input(&request.json()?, SerializerMode::Create)?;
        let title = input
            .get("title")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ApiError::bad_request("invalid_title", "title must be text"))?;
        let saved = self
            .0
            .queryset()
            .create(&Note {
                id: UUIDField::new(),
                title: TextField::new(title),
            })
            .await?;
        ApiResponse::json(StatusCode::CREATED, &saved)
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if lex_framework::is_production() {
        return Err("this public ephemeral example is development-only".into());
    }
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    let notes = Note::objects_sqlite_auto(pool).await?;
    let mut routes = RouteTable::new();
    routes.register(RouteSpec::http("/notes", "notes", Arc::new(Notes(notes))))?;
    let runtime = RestRuntimeBuilder::new(Arc::new(DenyAllPermissionBackend))
        .require_secure_transport(false)
        .middleware(vec![
            Arc::new(SecurityHeadersMiddleware::default()),
            Arc::new(RequestLimitsMiddleware::default()),
        ])
        .build()?;
    let router = AxumAdapter::new(Arc::new(RestApplication::new(routes, runtime))).router()?;
    let port: u16 = std::env::var("LEX_EXAMPLE_PORT")
        .unwrap_or_else(|_| "8080".into())
        .parse()?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    axum::serve(listener, router).await?;
    Ok(())
}
