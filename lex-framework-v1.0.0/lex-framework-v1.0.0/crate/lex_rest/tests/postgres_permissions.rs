use lex_rest::rest_framework::{
    permissions::{
        DatabasePermissionBackend, DatabasePermissionConfig, PermissionBackend,
        SqlxPermissionRepository,
    },
    request::{Principal, RequestContext, TransportSecurity},
};
use std::sync::Arc;
use uuid::Uuid;

fn require_disposable_database() -> String {
    assert_eq!(
        std::env::var("CONFIRM_DISPOSABLE_DATABASE").ok().as_deref(),
        Some("yes"),
        "refusing destructive permission test without CONFIRM_DISPOSABLE_DATABASE=yes"
    );
    std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL must be configured")
}

#[tokio::test]
#[ignore = "requires a disposable PostgreSQL database"]
async fn database_permission_backend_enforces_membership_role_and_permission_integrity() {
    let database_url = require_disposable_database();
    let pool = sqlx::PgPool::connect(&database_url)
        .await
        .expect("connect disposable PostgreSQL");

    sqlx::query(
        "DROP TABLE IF EXISTS tenant_role_permissions, permission_definitions, tenant_roles, memberships CASCADE",
    )
    .execute(&pool)
    .await
    .expect("drop test tables");
    sqlx::query(
        "CREATE TABLE memberships (\
            id UUID PRIMARY KEY, user_id UUID NOT NULL, tenant_id UUID NOT NULL, \
            is_tenant_admin BOOLEAN NOT NULL DEFAULT FALSE, deleted_at TIMESTAMPTZ NULL, \
            metadata JSONB NOT NULL DEFAULT '{}'::jsonb\
        )",
    )
    .execute(&pool)
    .await
    .expect("create memberships");
    sqlx::query(
        "CREATE TABLE tenant_roles (\
            id UUID PRIMARY KEY, tenant_id UUID NOT NULL, name TEXT NOT NULL, description TEXT NULL\
        )",
    )
    .execute(&pool)
    .await
    .expect("create tenant roles");
    sqlx::query(
        "CREATE TABLE permission_definitions (\
            id UUID PRIMARY KEY, name TEXT NOT NULL UNIQUE, description TEXT NULL\
        )",
    )
    .execute(&pool)
    .await
    .expect("create permission definitions");
    sqlx::query(
        "CREATE TABLE tenant_role_permissions (\
            membership_id UUID NOT NULL, role_id UUID NOT NULL, permission_id UUID NOT NULL\
        )",
    )
    .execute(&pool)
    .await
    .expect("create grants");

    let user_id = Uuid::new_v4();
    let tenant_id = Uuid::new_v4();
    let other_tenant_id = Uuid::new_v4();
    let membership_id = Uuid::new_v4();
    let role_id = Uuid::new_v4();
    let permission_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO memberships (id, user_id, tenant_id, is_tenant_admin) VALUES ($1, $2, $3, FALSE)",
    )
    .bind(membership_id)
    .bind(user_id)
    .bind(tenant_id)
    .execute(&pool)
    .await
    .expect("insert membership");
    sqlx::query("INSERT INTO tenant_roles (id, tenant_id, name) VALUES ($1, $2, 'viewer')")
        .bind(role_id)
        .bind(tenant_id)
        .execute(&pool)
        .await
        .expect("insert role");
    sqlx::query("INSERT INTO permission_definitions (id, name) VALUES ($1, 'users.view_user')")
        .bind(permission_id)
        .execute(&pool)
        .await
        .expect("insert permission");
    sqlx::query(
        "INSERT INTO tenant_role_permissions (membership_id, role_id, permission_id) VALUES ($1, $2, $3)",
    )
    .bind(membership_id)
    .bind(role_id)
    .bind(permission_id)
    .execute(&pool)
    .await
    .expect("insert grant");

    let backend = DatabasePermissionBackend::new(
        Arc::new(SqlxPermissionRepository::new(pool.clone())),
        DatabasePermissionConfig {
            global_admin_full_access: false,
            tenant_admin_full_access: false,
            allow_service_scopes: false,
            require_api_key_scope: true,
        },
    );
    let context = RequestContext::new(
        Principal::user(user_id, "integration", Vec::new()).expect("principal"),
        Some(tenant_id),
        None,
        TransportSecurity::Secure,
        None,
        None,
    )
    .expect("context");

    assert!(
        backend
            .has_permission(&context, "users.view_user")
            .await
            .expect("permission decision")
            .allowed
    );

    sqlx::query("UPDATE tenant_roles SET tenant_id = $1 WHERE id = $2")
        .bind(other_tenant_id)
        .bind(role_id)
        .execute(&pool)
        .await
        .expect("move role across tenant");
    assert!(
        !backend
            .has_permission(&context, "users.view_user")
            .await
            .expect("permission decision")
            .allowed
    );

    sqlx::query("UPDATE tenant_roles SET tenant_id = $1 WHERE id = $2")
        .bind(tenant_id)
        .bind(role_id)
        .execute(&pool)
        .await
        .expect("restore role tenant");
    sqlx::query("UPDATE memberships SET deleted_at = now() WHERE id = $1")
        .bind(membership_id)
        .execute(&pool)
        .await
        .expect("soft delete membership");
    assert!(
        !backend
            .has_permission(&context, "users.view_user")
            .await
            .expect("permission decision")
            .allowed
    );

    pool.close().await;
}
