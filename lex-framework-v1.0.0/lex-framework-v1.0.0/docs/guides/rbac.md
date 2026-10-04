# Normalized tenant RBAC and upgrading

The supported relationship is User → Membership → MembershipRole → TenantRole → RolePermission → PermissionDefinition. A Membership belongs to one user/tenant pair; a TenantRole belongs to one tenant. Multiple roles and overlapping grants are allowed. PermissionDefinition's existing unique `name` is the permission code; it has not been renamed to break callers.

Install with `migration::rbac::install_rbac_postgres` or `install_rbac_sqlite` before enabling `[security.permissions] backend = "database"`. The facade chooses `RbacPermissionRepository` for the active backend. Missing tables are server errors that deny access, not anonymous fallback.

| Constraint | Purpose |
|---|---|
| membership_user_tenant(user_id,tenant_id) | one membership per user/tenant |
| membership_id_tenant(id,tenant_id) | referenced composite parent key |
| tenant_role_name(tenant_id,name) | role names unique within tenant |
| tenant_role_id_tenant(id,tenant_id) | referenced composite parent key |
| membership_role_assignment(membership_id,role_id) | deduplicated assignments |
| role_permission_grant(role_id,permission_id) | deduplicated grants |
| PermissionDefinition.name unique | stable permission codes |

Both bridge tables carry tenant_id, constrained together with membership_id/role_id. Two independent FKs are insufficient: composite FKs prevent a valid Alpha membership from receiving a valid Beta role. User, tenant, membership, role and permission deletion is RESTRICT where referenced. Explicitly revoke assignments/grants first; identity deletion must not silently erase authorization history. Lookup indexes use deterministic names. Installation also verifies the actual complete, nonpartial unique-key shapes on both databases. Existing tables or same-name indexes do not certify integrity by themselves; missing required keys require a reviewed repair/rebuild.

Permission lookup is live on primary, with no framework authorization cache. Removed grants, removed assignments and soft-deleted users/tenants/memberships take effect on the next request. Authentication scopes remain credential snapshots: the scope backend does not automatically turn token scopes into database membership. Use the database backend for persisted SaaS permissions. API-key scopes further constrain permissions by default. Global/tenant admin bypasses are explicit permission backend configuration, not a role-name convention.

Legacy `TenantRolePermission` combines membership, role and permission. It remains public for compatibility; `SqlxPermissionRepository::new(pool)` is the legacy adapter. Before switching the facade's database backend, run `normalize_legacy_rbac_postgres` in a reviewed migration. It retains source grants, deduplicates pairs, rejects missing references/cross-tenant grants, and rejects members of the same role having different permission sets: unioning them would escalate privileges. Split such roles explicitly first. Existing populated normalized authorization state is never silently merged. Back up first, stop competing authorization writers, test against a copy, then change repository configuration.

SQLite currently supports fresh constrained RBAC installation and rejects unsuitable legacy schemas. Automatic SQLite legacy table rebuilding/conversion remains a release limitation. Do not drop legacy tables or advertise a destructive reset as an upgrade path. Retained PostgreSQL source grants aid rollback before new normalized writes; after writes diverge, rollback requires a reviewed data reconciliation.
