use serde::{Deserialize, Serialize};

use orm::Model;

use db_controller::{
    BooleanField, DateTimeField, JsonField, ManyToMany, PasswordField, TextField, UUIDField,
};

/// 🔐 Enum for Role/Permission Actions
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
#[sqlx(type_name = "text", rename_all = "lowercase")]
pub enum PermissionAction {
    Read,
    Write,
    Update,
    Delete,
    Manage,
}

/// 🎭 Enum for User Role
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[sqlx(type_name = "text", rename_all = "lowercase")]
pub enum UserRole {
    User,
    Admin,
    SuperAdmin,
}

/// 👛 API Key Model
#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "api_keys")]
#[timestamps]
pub struct ApiKey {
    #[field(primary_key)]
    pub id: UUIDField,

    #[field(index)]
    pub owner_id: UUIDField,
    #[relation(local = "owner_id", model = "User", remote = "id")]
    pub owner: Option<User>,

    #[field(index, nullable)]
    pub tenant_id: Option<UUIDField>,
    #[relation(local = "tenant_id", model = "Tenant", remote = "id")]
    pub tenant: Option<Tenant>,

    #[field(not_empty)]
    #[field(max_length = 100)]
    pub scope: TextField,

    #[field(encrypted)]
    #[serde(skip_serializing)]
    pub key_hash: TextField,

    #[field(default = false)]
    pub revoked: BooleanField,

    #[field(nullable)]
    pub expires_at: Option<DateTimeField>,

    #[field(default = "'No description'")]
    #[field(max_length = 255)]
    pub description: Option<TextField>,

    #[field(soft_delete)]
    pub deleted_at: Option<DateTimeField>,

    #[field(default = "'{}'::jsonb")]
    pub metadata: JsonField<serde_json::Value>,
}

/// 👤 User Model
#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "users")]
#[timestamps]
pub struct User {
    #[field(primary_key)]
    pub id: UUIDField,

    #[field(not_empty, max_length = 100)]
    pub name: TextField,

    #[field(not_empty, unique, regex = r"^[^@]+@[^@]+\.[^@]+$", max_length = 255)]
    pub email: TextField,

    #[field(encrypted)]
    #[serde(skip_serializing)]
    pub password: PasswordField,

    #[field(default = "'user'", max_length = 50)]
    pub role: UserRole,

    #[field(default = false)]
    pub verified: BooleanField,

    #[field(nullable)]
    #[serde(skip_serializing)]
    pub verification_token: Option<TextField>,

    #[field(nullable)]
    pub token_expires_at: Option<DateTimeField>,

    #[field(default = false)]
    pub is_global_admin: BooleanField,

    #[field(default = false)]
    pub is_kyc_verified: BooleanField,

    #[field(soft_delete)]
    pub deleted_at: Option<DateTimeField>,

    #[field(default = "'{}'::jsonb")]
    pub metadata: JsonField<serde_json::Value>,

    /// Many Tenants via Membership
    #[relation(model = "Membership", local = "id", remote = "user_id", many)]
    #[serde(default)]
    pub tenants: ManyToMany<UUIDField>,

    /// Owned API keys
    #[relation(model = "ApiKey", local = "id", remote = "owner_id", many)]
    #[serde(default)]
    pub api_keys: ManyToMany<UUIDField>,
}

/// 🏢 Tenant Model
#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "tenants")]
#[timestamps]
pub struct Tenant {
    #[field(primary_key)]
    pub id: UUIDField,

    #[field(not_empty, unique, max_length = 150)]
    pub name: TextField,

    #[field(soft_delete)]
    pub deleted_at: Option<DateTimeField>,

    #[field(default = "'{}'::jsonb")]
    pub metadata: JsonField<serde_json::Value>,

    /// Many Users via Membership
    #[relation(model = "Membership", local = "id", remote = "tenant_id", many)]
    #[serde(default)]
    pub users: ManyToMany<UUIDField>,

    /// Many API keys scoped to this tenant
    #[relation(model = "ApiKey", local = "id", remote = "tenant_id", many)]
    #[serde(default)]
    pub api_keys: ManyToMany<UUIDField>,

    /// Tenant-specific roles
    #[relation(model = "TenantRole", local = "id", remote = "tenant_id", many)]
    #[serde(default)]
    pub roles: ManyToMany<UUIDField>,
}

/// 🧑‍🤝‍🧑 Membership Model (bridge User ⇄ Tenant)
#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "memberships")]
#[model(
    unique(name = "membership_user_tenant", fields = "user_id,tenant_id"),
    unique(name = "membership_id_tenant", fields = "id,tenant_id")
)]
#[timestamps]
pub struct Membership {
    #[field(primary_key)]
    pub id: UUIDField,

    #[field(index)]
    pub user_id: UUIDField,
    #[relation(local = "user_id", model = "User", remote = "id")]
    pub user: Option<User>,

    #[field(index)]
    pub tenant_id: UUIDField,
    #[relation(local = "tenant_id", model = "Tenant", remote = "id")]
    pub tenant: Option<Tenant>,

    #[field(default = false)]
    pub is_tenant_admin: BooleanField,

    #[field(soft_delete)]
    pub deleted_at: Option<DateTimeField>,

    #[field(default = "'{}'::jsonb")]
    pub metadata: JsonField<serde_json::Value>,

    /// Delegated permissions for this membership
    #[relation(
        model = "TenantRolePermission",
        local = "id",
        remote = "membership_id",
        many
    )]
    #[serde(default)]
    pub delegated_permissions: ManyToMany<UUIDField>,
}

/// 🔑 Tenant-specific Role
#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "tenant_roles")]
#[model(
    unique(name = "tenant_role_name", fields = "tenant_id,name"),
    unique(name = "tenant_role_id_tenant", fields = "id,tenant_id")
)]
#[timestamps]
pub struct TenantRole {
    #[field(primary_key)]
    pub id: UUIDField,

    #[field(index)]
    pub tenant_id: UUIDField,
    #[relation(local = "tenant_id", model = "Tenant", remote = "id")]
    pub tenant: Option<Tenant>,

    #[field(not_empty, max_length = 100)]
    pub name: TextField,

    #[field(nullable, max_length = 255)]
    pub description: Option<TextField>,

    /// Permissions granted by this role
    #[relation(model = "TenantRolePermission", local = "id", remote = "role_id", many)]
    #[serde(default)]
    pub permissions: ManyToMany<UUIDField>,
}

/// 🔐 TenantRole ⇄ PermissionDefinition bridge
#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "tenant_role_permissions")]
pub struct TenantRolePermission {
    #[field(index)]
    pub membership_id: UUIDField,
    #[relation(local = "membership_id", model = "Membership", remote = "id")]
    pub membership: Option<Membership>,

    #[field(index)]
    pub role_id: UUIDField,
    #[relation(local = "role_id", model = "TenantRole", remote = "id")]
    pub role: Option<TenantRole>,

    #[field(index)]
    pub permission_id: UUIDField,
    #[relation(local = "permission_id", model = "PermissionDefinition", remote = "id")]
    pub permission: Option<PermissionDefinition>,
}

/// 📖 Global PermissionDefinition
#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "permission_definitions")]
pub struct PermissionDefinition {
    #[field(primary_key)]
    pub id: UUIDField,

    #[field(not_empty, unique, max_length = 100)]
    pub name: TextField,

    #[field(nullable, max_length = 255)]
    pub description: Option<TextField>,
}

/// 🔐 System-wide RoleDefinition (optional)
#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "role_definitions")]
pub struct RoleDefinition {
    #[field(primary_key)]
    pub id: UUIDField,

    #[field(not_empty, unique, max_length = 100)]
    pub name: TextField,

    #[field(nullable, max_length = 255)]
    pub description: Option<TextField>,
}
