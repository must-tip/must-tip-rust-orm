//! Normalized tenant roles. Install the accompanying constrained schema through a migration;
//! auto-created ORM tables alone are not an RBAC integrity boundary.

use db_controller::UUIDField;
use orm::Model;
use serde::{Deserialize, Serialize};

/// Assigns a tenant role to a membership. Tenant is duplicated to enforce composite foreign keys.
#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "membership_roles")]
#[model(unique(name = "membership_role_assignment", fields = "membership_id,role_id"))]
pub struct MembershipRole {
    #[field(primary_key)]
    pub id: UUIDField,
    pub tenant_id: UUIDField,
    pub membership_id: UUIDField,
    pub role_id: UUIDField,
}

/// Grants a permission to a tenant role, independently of the role's assigned members.
#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "role_permissions")]
#[model(unique(name = "role_permission_grant", fields = "role_id,permission_id"))]
pub struct RolePermission {
    #[field(primary_key)]
    pub id: UUIDField,
    pub tenant_id: UUIDField,
    pub role_id: UUIDField,
    pub permission_id: UUIDField,
}
