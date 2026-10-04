pub mod rbac;
pub mod user;
pub use rbac::{MembershipRole, RolePermission};

pub use user::{
    ApiKey, Membership, PermissionAction, PermissionDefinition, RoleDefinition, Tenant, TenantRole,
    TenantRolePermission, User, UserRole,
};

pub fn register_all_models() {
    use db_controller::Entity;
    ApiKey::register();
    User::register();
    Tenant::register();
    Membership::register();
    TenantRole::register();
    TenantRolePermission::register();
    MembershipRole::register();
    RolePermission::register();
    PermissionDefinition::register();
    RoleDefinition::register();
}

#[cfg(test)]
mod tests {
    use super::{TenantRolePermission, User, UserRole};
    use db_controller::{Entity, effective_schema};
    use sqlx::{FromRow, postgres::PgRow, sqlite::SqliteRow};

    fn assert_repository_row<T>()
    where
        T: Entity
            + for<'r> FromRow<'r, PgRow>
            + for<'r> FromRow<'r, SqliteRow>
            + Send
            + Sync
            + Unpin,
    {
    }

    #[test]
    fn generated_models_are_repository_row_compatible() {
        assert_repository_row::<User>();
        assert_repository_row::<TenantRolePermission>();
    }

    #[test]
    fn user_schema_has_uuid_primary_key_and_excludes_relations() {
        let schema = User::schema();
        let id = schema.get("id").unwrap_or_else(|| panic!("id missing"));
        assert_eq!(id.sql_type, "UUID");
        assert!(id.primary_key);
        assert!(!schema.contains_key("tenants"));
        assert!(!schema.contains_key("api_keys"));
    }

    #[test]
    fn implicit_framework_columns_are_visible_to_migrations() {
        let schema = effective_schema::<TenantRolePermission>();
        assert_eq!(
            schema.get("id").map(|value| value.sql_type.as_str()),
            Some("BIGSERIAL")
        );
        assert!(schema.contains_key("created_at"));
        assert!(schema.contains_key("updated_at"));
    }

    #[test]
    fn roles_use_one_lowercase_database_representation() {
        let value = serde_json::to_value(UserRole::User)
            .unwrap_or_else(|error| panic!("role serialization failed: {error}"));
        assert_eq!(value, serde_json::json!("user"));
    }
}
