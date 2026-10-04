use db_controller::{JsonField, ManyToMany, TextField, UUIDField};
use lex_rest::rest_framework::{
    generic::{ModelFactory, ModelMutationAdapter, OrmModelMutationAdapter},
    request::{Principal, RequestContext, TransportSecurity},
};
use models::Tenant;
use serde_json::{Map, Value, json};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

struct TenantFactory;

impl ModelFactory<Tenant> for TenantFactory {
    fn build_create(
        &self,
        _context: &RequestContext,
        _validated: Map<String, Value>,
    ) -> lex_rest::rest_framework::exceptions::ApiResult<Tenant> {
        Err(
            lex_rest::rest_framework::exceptions::ApiError::configuration(
                "test factory does not support create",
            ),
        )
    }
}

fn test_tenant(id: Uuid, name: &str) -> Tenant {
    Tenant {
        id: UUIDField(id),
        name: TextField::new(name),
        deleted_at: None,
        metadata: JsonField::new(json!({})),
        users: ManyToMany::default(),
        api_keys: ManyToMany::default(),
        roles: ManyToMany::default(),
    }
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and CONFIRM_DISPOSABLE_DATABASE=yes"]
async fn invalid_model_update_rolls_back_before_commit() {
    if std::env::var("CONFIRM_DISPOSABLE_DATABASE").as_deref() != Ok("yes") {
        panic!("refusing destructive PostgreSQL test without CONFIRM_DISPOSABLE_DATABASE=yes");
    }
    let database_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        panic!("TEST_DATABASE_URL must point to a disposable PostgreSQL database")
    });
    let pool = PgPool::connect(&database_url)
        .await
        .unwrap_or_else(|error| panic!("connect disposable PostgreSQL: {error}"));

    sqlx::query("DROP TABLE IF EXISTS tenants CASCADE")
        .execute(&pool)
        .await
        .unwrap_or_else(|error| panic!("drop tenants: {error}"));
    sqlx::query(
        "CREATE TABLE tenants (\
            id UUID PRIMARY KEY,\
            name TEXT NOT NULL UNIQUE,\
            deleted_at TIMESTAMPTZ NULL,\
            metadata JSONB NOT NULL DEFAULT '{}'::jsonb,\
            created_at TIMESTAMPTZ NOT NULL DEFAULT now(),\
            updated_at TIMESTAMPTZ NOT NULL DEFAULT now()\
        )",
    )
    .execute(&pool)
    .await
    .unwrap_or_else(|error| panic!("create tenants: {error}"));

    let tenant_id = Uuid::new_v4();
    sqlx::query("INSERT INTO tenants (id, name) VALUES ($1, $2)")
        .bind(tenant_id)
        .bind("valid-tenant")
        .execute(&pool)
        .await
        .unwrap_or_else(|error| panic!("insert tenant: {error}"));

    let current = test_tenant(tenant_id, "valid-tenant");
    let adapter = OrmModelMutationAdapter::new(pool.clone(), Arc::new(TenantFactory));
    let context = RequestContext::new(
        Principal::user(Uuid::new_v4(), "integration", Vec::new())
            .unwrap_or_else(|error| panic!("principal: {error}")),
        None,
        None,
        TransportSecurity::Secure,
        None,
        None,
    )
    .unwrap_or_else(|error| panic!("context: {error}"));

    let mut patch = Map::new();
    patch.insert("name".to_owned(), Value::String(String::new()));
    let error = adapter
        .update(&context, &current, patch, true)
        .await
        .expect_err("empty name must fail model validation and rollback");
    assert_eq!(error.code(), "model_validation_error");

    let persisted_name: String = sqlx::query_scalar("SELECT name FROM tenants WHERE id = $1")
        .bind(tenant_id)
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|error| panic!("reload tenant name: {error}"));
    assert_eq!(persisted_name, "valid-tenant");
}
