use async_trait::async_trait;
use db_controller::{Field, ModelManager};
use models::{Membership, PermissionDefinition, TenantRole, TenantRolePermission, User};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashSet},
    future::Future,
    pin::Pin,
    sync::Arc,
};
use uuid::Uuid;

mod normalized;
pub use normalized::RbacPermissionRepository;

use crate::rest_framework::{
    exceptions::{ApiError, ApiResult},
    request::{PrincipalKind, RequestContext},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionResource {
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub tenant_id: Option<Uuid>,
    pub owner_user_id: Option<Uuid>,
}

impl PermissionResource {
    pub fn new(resource_type: impl Into<String>) -> Self {
        Self {
            resource_type: resource_type.into(),
            resource_id: None,
            tenant_id: None,
            owner_user_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionDecision {
    pub allowed: bool,
    pub reason: &'static str,
}

impl PermissionDecision {
    pub const fn allow(reason: &'static str) -> Self {
        Self {
            allowed: true,
            reason,
        }
    }

    pub const fn deny(reason: &'static str) -> Self {
        Self {
            allowed: false,
            reason,
        }
    }
}

#[async_trait]
pub trait PermissionBackend: Send + Sync {
    async fn has_permission(
        &self,
        context: &RequestContext,
        permission: &str,
    ) -> ApiResult<PermissionDecision>;

    async fn has_object_permission(
        &self,
        context: &RequestContext,
        permission: &str,
        resource: &PermissionResource,
    ) -> ApiResult<PermissionDecision> {
        if let Some(resource_tenant) = resource.tenant_id
            && context.tenant_id() != Some(resource_tenant)
        {
            return Ok(PermissionDecision::deny("cross_tenant_object"));
        }
        self.has_permission(context, permission).await
    }

    async fn is_tenant_member(&self, context: &RequestContext) -> ApiResult<bool>;

    async fn is_tenant_admin(&self, context: &RequestContext) -> ApiResult<bool>;

    async fn is_global_admin(&self, context: &RequestContext) -> ApiResult<bool>;
}

#[derive(Debug, Clone)]
pub struct ScopePermissionConfig {
    pub global_admin_scope: String,
    pub tenant_member_scope: String,
    pub tenant_admin_scope: String,
}

impl Default for ScopePermissionConfig {
    fn default() -> Self {
        Self {
            global_admin_scope: "admin:global".into(),
            tenant_member_scope: "tenant:member".into(),
            tenant_admin_scope: "tenant:admin".into(),
        }
    }
}

/// Zero-setup permission backend backed by authenticated principal scopes.
///
/// It is the framework-friendly default for SQLite and simple applications.
/// Named permissions map directly to scopes, while the standard admin/member
/// rules use configurable reserved scopes. Applications that need persisted
/// tenant-role membership may select the database permission backend instead.
#[derive(Debug, Clone, Default)]
pub struct ScopePermissionBackend {
    config: ScopePermissionConfig,
}

impl ScopePermissionBackend {
    pub fn new(config: ScopePermissionConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl PermissionBackend for ScopePermissionBackend {
    async fn has_permission(
        &self,
        context: &RequestContext,
        permission: &str,
    ) -> ApiResult<PermissionDecision> {
        DatabasePermissionBackend::<SqlxPermissionRepository>::validate_permission_name(
            permission,
        )?;
        if !context.principal().is_authenticated() {
            return Ok(PermissionDecision::deny("anonymous"));
        }
        Ok(if context.principal().has_scope(permission) {
            PermissionDecision::allow("principal_scope")
        } else {
            PermissionDecision::deny("scope_missing")
        })
    }

    async fn is_tenant_member(&self, context: &RequestContext) -> ApiResult<bool> {
        Ok(context.principal().is_authenticated()
            && context.tenant_id().is_some()
            && (context
                .principal()
                .has_scope(&self.config.tenant_member_scope)
                || context
                    .principal()
                    .has_scope(&self.config.tenant_admin_scope)
                || context
                    .principal()
                    .has_scope(&self.config.global_admin_scope)))
    }

    async fn is_tenant_admin(&self, context: &RequestContext) -> ApiResult<bool> {
        Ok(context.principal().is_authenticated()
            && context.tenant_id().is_some()
            && (context
                .principal()
                .has_scope(&self.config.tenant_admin_scope)
                || context
                    .principal()
                    .has_scope(&self.config.global_admin_scope)))
    }

    async fn is_global_admin(&self, context: &RequestContext) -> ApiResult<bool> {
        Ok(context.principal().is_authenticated()
            && context
                .principal()
                .has_scope(&self.config.global_admin_scope))
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DenyAllPermissionBackend;

#[async_trait]
impl PermissionBackend for DenyAllPermissionBackend {
    async fn has_permission(
        &self,
        _context: &RequestContext,
        _permission: &str,
    ) -> ApiResult<PermissionDecision> {
        Ok(PermissionDecision::deny(
            "permission_backend_not_configured",
        ))
    }

    async fn is_tenant_member(&self, _context: &RequestContext) -> ApiResult<bool> {
        Ok(false)
    }

    async fn is_tenant_admin(&self, _context: &RequestContext) -> ApiResult<bool> {
        Ok(false)
    }

    async fn is_global_admin(&self, _context: &RequestContext) -> ApiResult<bool> {
        Ok(false)
    }
}

#[derive(Debug, Clone)]
pub struct UserSecurityRecord {
    pub is_global_admin: bool,
}

#[derive(Debug, Clone)]
pub struct MembershipSecurityRecord {
    pub id: Uuid,
    pub is_tenant_admin: bool,
}

#[async_trait]
pub trait PermissionRepository: Send + Sync {
    async fn user_security(&self, user_id: Uuid) -> ApiResult<Option<UserSecurityRecord>>;

    async fn membership(
        &self,
        user_id: Uuid,
        tenant_id: Uuid,
    ) -> ApiResult<Option<MembershipSecurityRecord>>;

    async fn permission_names(
        &self,
        membership_id: Uuid,
        tenant_id: Uuid,
    ) -> ApiResult<BTreeSet<String>>;
}

#[derive(Clone)]
pub struct SqlxPermissionRepository {
    pool: sqlx::PgPool,
}

impl SqlxPermissionRepository {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PermissionRepository for SqlxPermissionRepository {
    async fn user_security(&self, user_id: Uuid) -> ApiResult<Option<UserSecurityRecord>> {
        let user = ModelManager::<User>::objects(self.pool.clone())
            .queryset()
            .filter(Field::<User>::new("id").eq(user_id))
            .first()
            .await
            .map_err(ApiError::from)?;
        Ok(user.map(|user| UserSecurityRecord {
            is_global_admin: user.is_global_admin.is_true(),
        }))
    }

    async fn membership(
        &self,
        user_id: Uuid,
        tenant_id: Uuid,
    ) -> ApiResult<Option<MembershipSecurityRecord>> {
        let membership = ModelManager::<Membership>::objects(self.pool.clone())
            .queryset()
            .for_tenant(tenant_id)
            .filter(Field::<Membership>::new("user_id").eq(user_id))
            .first()
            .await
            .map_err(ApiError::from)?;
        Ok(membership.map(|membership| MembershipSecurityRecord {
            id: membership.id.0,
            is_tenant_admin: membership.is_tenant_admin.is_true(),
        }))
    }

    async fn permission_names(
        &self,
        membership_id: Uuid,
        tenant_id: Uuid,
    ) -> ApiResult<BTreeSet<String>> {
        let grants = ModelManager::<TenantRolePermission>::objects(self.pool.clone())
            .queryset()
            .filter(Field::<TenantRolePermission>::new("membership_id").eq(membership_id))
            .all()
            .await
            .map_err(ApiError::from)?;
        if grants.is_empty() {
            return Ok(BTreeSet::new());
        }

        // A grant is effective only when its role still exists in the authenticated tenant. This
        // prevents stale/mis-scoped bridge rows from authorizing a membership across tenants.
        let role_ids = grants
            .iter()
            .map(|grant| grant.role_id.0)
            .collect::<HashSet<_>>();
        let valid_roles = ModelManager::<TenantRole>::objects(self.pool.clone())
            .queryset()
            .for_tenant(tenant_id)
            .filter(Field::<TenantRole>::new("id").is_in(role_ids))
            .all()
            .await
            .map_err(ApiError::from)?;
        let valid_role_ids = valid_roles
            .into_iter()
            .map(|role| role.id.0)
            .collect::<HashSet<_>>();

        let permission_ids = grants
            .into_iter()
            .filter(|grant| valid_role_ids.contains(&grant.role_id.0))
            .map(|grant| grant.permission_id.0)
            .collect::<HashSet<_>>();
        if permission_ids.is_empty() {
            return Ok(BTreeSet::new());
        }

        let definitions = ModelManager::<PermissionDefinition>::objects(self.pool.clone())
            .queryset()
            .filter(Field::<PermissionDefinition>::new("id").is_in(permission_ids))
            .all()
            .await
            .map_err(ApiError::from)?;
        Ok(definitions
            .into_iter()
            .map(|definition| definition.name.0)
            .collect())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DatabasePermissionConfig {
    pub global_admin_full_access: bool,
    pub tenant_admin_full_access: bool,
    /// Service identities may be authorized directly by explicitly authenticated scopes.
    pub allow_service_scopes: bool,
    /// API-key scopes are an upper bound on the owning user's database permissions.
    pub require_api_key_scope: bool,
}

impl Default for DatabasePermissionConfig {
    fn default() -> Self {
        Self {
            global_admin_full_access: true,
            tenant_admin_full_access: true,
            allow_service_scopes: false,
            require_api_key_scope: true,
        }
    }
}

pub struct DatabasePermissionBackend<R = SqlxPermissionRepository> {
    repository: Arc<R>,
    config: DatabasePermissionConfig,
}

impl DatabasePermissionBackend<SqlxPermissionRepository> {
    pub fn from_pool(pool: sqlx::PgPool) -> Self {
        Self::new(
            Arc::new(SqlxPermissionRepository::new(pool)),
            DatabasePermissionConfig::default(),
        )
    }
}

impl<R> DatabasePermissionBackend<R> {
    pub fn new(repository: Arc<R>, config: DatabasePermissionConfig) -> Self {
        Self { repository, config }
    }

    fn validate_permission_name(permission: &str) -> ApiResult<()> {
        if permission.is_empty()
            || permission.len() > 255
            || permission.chars().any(char::is_whitespace)
            || !permission
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | ':' | '-'))
        {
            return Err(ApiError::configuration("invalid permission code"));
        }
        Ok(())
    }
}

#[async_trait]
impl<R> PermissionBackend for DatabasePermissionBackend<R>
where
    R: PermissionRepository + 'static,
{
    async fn has_permission(
        &self,
        context: &RequestContext,
        permission: &str,
    ) -> ApiResult<PermissionDecision> {
        Self::validate_permission_name(permission)?;
        if !context.principal().is_authenticated() {
            return Ok(PermissionDecision::deny("anonymous"));
        }

        if matches!(context.principal().kind(), PrincipalKind::Service)
            && self.config.allow_service_scopes
            && context.principal().has_scope(permission)
        {
            return Ok(PermissionDecision::allow("service_scope"));
        }

        if matches!(context.principal().kind(), PrincipalKind::ApiKey)
            && self.config.require_api_key_scope
            && !context.principal().has_scope(permission)
        {
            return Ok(PermissionDecision::deny("api_key_scope_missing"));
        }

        let Some(user_id) = context.principal().user_id() else {
            return Ok(PermissionDecision::deny("principal_has_no_user"));
        };

        if self.config.global_admin_full_access
            && self
                .repository
                .user_security(user_id)
                .await?
                .is_some_and(|user| user.is_global_admin)
        {
            return Ok(PermissionDecision::allow("global_admin"));
        }

        let Some(tenant_id) = context.tenant_id() else {
            return Ok(PermissionDecision::deny("tenant_required"));
        };
        let Some(membership) = self.repository.membership(user_id, tenant_id).await? else {
            return Ok(PermissionDecision::deny("not_tenant_member"));
        };

        if self.config.tenant_admin_full_access && membership.is_tenant_admin {
            return Ok(PermissionDecision::allow("tenant_admin"));
        }

        let permissions = self
            .repository
            .permission_names(membership.id, tenant_id)
            .await?;
        Ok(if permissions.contains(permission) {
            PermissionDecision::allow("assigned_permission")
        } else {
            PermissionDecision::deny("permission_not_assigned")
        })
    }

    async fn has_object_permission(
        &self,
        context: &RequestContext,
        permission: &str,
        resource: &PermissionResource,
    ) -> ApiResult<PermissionDecision> {
        if let Some(resource_tenant) = resource.tenant_id
            && context
                .tenant_id()
                .is_some_and(|tenant| tenant != resource_tenant)
        {
            if self.is_global_admin(context).await? {
                return self.has_permission(context, permission).await;
            }
            return Ok(PermissionDecision::deny("cross_tenant_object"));
        }
        self.has_permission(context, permission).await
    }

    async fn is_tenant_member(&self, context: &RequestContext) -> ApiResult<bool> {
        let (Some(user_id), Some(tenant_id)) = (context.principal().user_id(), context.tenant_id())
        else {
            return Ok(false);
        };
        Ok(self
            .repository
            .membership(user_id, tenant_id)
            .await?
            .is_some())
    }

    async fn is_tenant_admin(&self, context: &RequestContext) -> ApiResult<bool> {
        let (Some(user_id), Some(tenant_id)) = (context.principal().user_id(), context.tenant_id())
        else {
            return Ok(false);
        };
        Ok(self
            .repository
            .membership(user_id, tenant_id)
            .await?
            .is_some_and(|membership| membership.is_tenant_admin))
    }

    async fn is_global_admin(&self, context: &RequestContext) -> ApiResult<bool> {
        let Some(user_id) = context.principal().user_id() else {
            return Ok(false);
        };
        Ok(self
            .repository
            .user_security(user_id)
            .await?
            .is_some_and(|user| user.is_global_admin))
    }
}

/// Ergonomic constructors for the framework permission engine.
///
/// Application views should normally use this type instead of constructing
/// `PermissionExpr` variants manually. The underlying permission engine remains
/// unchanged, so existing advanced policies continue to work.
pub struct Permissions;

impl Permissions {
    pub fn public() -> PermissionExpr {
        PermissionExpr::AllowAny
    }
    pub fn deny_all() -> PermissionExpr {
        PermissionExpr::DenyAll
    }
    pub fn authenticated() -> PermissionExpr {
        PermissionExpr::Authenticated
    }
    pub fn anonymous_only() -> PermissionExpr {
        PermissionExpr::AnonymousOnly
    }
    pub fn global_admin() -> PermissionExpr {
        PermissionExpr::GlobalAdmin
    }
    pub fn tenant_member() -> PermissionExpr {
        PermissionExpr::TenantMember
    }
    pub fn tenant_admin() -> PermissionExpr {
        PermissionExpr::TenantAdmin
    }
    pub fn owner() -> PermissionExpr {
        PermissionExpr::Owner
    }
    pub fn owner_or_read_only() -> PermissionExpr {
        PermissionExpr::OwnerOrReadOnly
    }

    pub fn named(permission: impl Into<String>) -> PermissionExpr {
        PermissionExpr::HasPermission(permission.into())
    }

    pub fn all<I, S>(permissions: I) -> PermissionExpr
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        PermissionExpr::HasAll(permissions.into_iter().map(Into::into).collect())
    }

    pub fn any<I, S>(permissions: I) -> PermissionExpr
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        PermissionExpr::HasAny(permissions.into_iter().map(Into::into).collect())
    }

    pub fn and<I>(rules: I) -> PermissionExpr
    where
        I: IntoIterator<Item = PermissionExpr>,
    {
        PermissionExpr::And(rules.into_iter().collect())
    }

    pub fn or<I>(rules: I) -> PermissionExpr
    where
        I: IntoIterator<Item = PermissionExpr>,
    {
        PermissionExpr::Or(rules.into_iter().collect())
    }

    pub fn not(rule: PermissionExpr) -> PermissionExpr {
        PermissionExpr::Not(Box::new(rule))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionExpr {
    AllowAny,
    DenyAll,
    Authenticated,
    AnonymousOnly,
    GlobalAdmin,
    TenantMember,
    TenantAdmin,
    Owner,
    OwnerOrReadOnly,
    HasPermission(String),
    HasAll(Vec<String>),
    HasAny(Vec<String>),
    And(Vec<PermissionExpr>),
    Or(Vec<PermissionExpr>),
    Not(Box<PermissionExpr>),
}

impl PermissionExpr {
    pub fn and(self, other: Self) -> Self {
        Self::And(vec![self, other])
    }

    pub fn or(self, other: Self) -> Self {
        Self::Or(vec![self, other])
    }

    pub fn negate(self) -> Self {
        Self::Not(Box::new(self))
    }

    pub fn evaluate<'a>(
        &'a self,
        backend: &'a dyn PermissionBackend,
        context: &'a RequestContext,
        resource: Option<&'a PermissionResource>,
        read_only: bool,
    ) -> Pin<Box<dyn Future<Output = ApiResult<bool>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(resource_tenant) = resource.and_then(|resource| resource.tenant_id)
                && context.tenant_id() != Some(resource_tenant)
                && !backend.is_global_admin(context).await?
            {
                return Ok(false);
            }
            match self {
                Self::AllowAny => Ok(true),
                Self::DenyAll => Ok(false),
                Self::Authenticated => Ok(context.principal().is_authenticated()),
                Self::AnonymousOnly => Ok(!context.principal().is_authenticated()),
                Self::GlobalAdmin => backend.is_global_admin(context).await,
                Self::TenantMember => backend.is_tenant_member(context).await,
                Self::TenantAdmin => backend.is_tenant_admin(context).await,
                Self::Owner => Ok(resource
                    .and_then(|resource| resource.owner_user_id)
                    .is_some_and(|owner| context.principal().user_id() == Some(owner))),
                Self::OwnerOrReadOnly => {
                    if read_only {
                        // Object-only policy: fail closed when evaluated without a resource.
                        Ok(resource.is_some())
                    } else {
                        Ok(resource
                            .and_then(|resource| resource.owner_user_id)
                            .is_some_and(|owner| context.principal().user_id() == Some(owner)))
                    }
                }
                Self::HasPermission(permission) => {
                    let decision = if let Some(resource) = resource {
                        backend
                            .has_object_permission(context, permission, resource)
                            .await?
                    } else {
                        backend.has_permission(context, permission).await?
                    };
                    Ok(decision.allowed)
                }
                Self::HasAll(permissions) => {
                    if permissions.is_empty() {
                        return Ok(false);
                    }
                    for permission in permissions {
                        let decision = if let Some(resource) = resource {
                            backend
                                .has_object_permission(context, permission, resource)
                                .await?
                        } else {
                            backend.has_permission(context, permission).await?
                        };
                        if !decision.allowed {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
                Self::HasAny(permissions) => {
                    for permission in permissions {
                        let decision = if let Some(resource) = resource {
                            backend
                                .has_object_permission(context, permission, resource)
                                .await?
                        } else {
                            backend.has_permission(context, permission).await?
                        };
                        if decision.allowed {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
                Self::And(expressions) => {
                    if expressions.is_empty() {
                        return Ok(false);
                    }
                    for expression in expressions {
                        if !expression
                            .evaluate(backend, context, resource, read_only)
                            .await?
                        {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
                Self::Or(expressions) => {
                    for expression in expressions {
                        if expression
                            .evaluate(backend, context, resource, read_only)
                            .await?
                        {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
                Self::Not(expression) => Ok(!expression
                    .evaluate(backend, context, resource, read_only)
                    .await?),
            }
        })
    }

    pub async fn enforce(
        &self,
        backend: &dyn PermissionBackend,
        context: &RequestContext,
        resource: Option<&PermissionResource>,
        read_only: bool,
    ) -> ApiResult<()> {
        if self.evaluate(backend, context, resource, read_only).await? {
            Ok(())
        } else if context.principal().is_authenticated() {
            Err(ApiError::permission_denied("permission denied"))
        } else {
            Err(ApiError::authentication("authentication required"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rest_framework::request::{Principal, TransportSecurity};
    use std::{collections::BTreeMap, sync::Mutex};

    #[derive(Default)]
    struct MemoryRepository {
        users: Mutex<BTreeMap<Uuid, UserSecurityRecord>>,
        memberships: Mutex<BTreeMap<(Uuid, Uuid), MembershipSecurityRecord>>,
        permissions: Mutex<BTreeMap<Uuid, BTreeSet<String>>>,
    }

    #[async_trait]
    impl PermissionRepository for MemoryRepository {
        async fn user_security(&self, user_id: Uuid) -> ApiResult<Option<UserSecurityRecord>> {
            Ok(self.users.lock().expect("users").get(&user_id).cloned())
        }

        async fn membership(
            &self,
            user_id: Uuid,
            tenant_id: Uuid,
        ) -> ApiResult<Option<MembershipSecurityRecord>> {
            Ok(self
                .memberships
                .lock()
                .expect("memberships")
                .get(&(user_id, tenant_id))
                .cloned())
        }

        async fn permission_names(
            &self,
            membership_id: Uuid,
            _tenant_id: Uuid,
        ) -> ApiResult<BTreeSet<String>> {
            Ok(self
                .permissions
                .lock()
                .expect("permissions")
                .get(&membership_id)
                .cloned()
                .unwrap_or_default())
        }
    }

    #[tokio::test]
    async fn assigned_membership_permission_is_enforced() {
        let user_id = Uuid::new_v4();
        let tenant_id = Uuid::new_v4();
        let membership_id = Uuid::new_v4();
        let repository = Arc::new(MemoryRepository::default());
        repository.memberships.lock().expect("memberships").insert(
            (user_id, tenant_id),
            MembershipSecurityRecord {
                id: membership_id,
                is_tenant_admin: false,
            },
        );
        repository.permissions.lock().expect("permissions").insert(
            membership_id,
            BTreeSet::from(["users.view_user".to_owned()]),
        );
        let backend = DatabasePermissionBackend::new(
            repository,
            DatabasePermissionConfig {
                global_admin_full_access: false,
                tenant_admin_full_access: false,
                allow_service_scopes: false,
                require_api_key_scope: true,
            },
        );
        let principal = Principal::user(user_id, "test", Vec::new()).expect("principal");
        let context = RequestContext::new(
            principal,
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
                .expect("decision")
                .allowed
        );
        assert!(
            !backend
                .has_permission(&context, "users.delete_user")
                .await
                .expect("decision")
                .allowed
        );
    }

    #[tokio::test]
    async fn owner_permission_cannot_bypass_tenant_boundary() {
        let user_id = Uuid::new_v4();
        let tenant_id = Uuid::new_v4();
        let other_tenant = Uuid::new_v4();
        let repository = Arc::new(MemoryRepository::default());
        let backend =
            DatabasePermissionBackend::new(repository, DatabasePermissionConfig::default());
        let context = RequestContext::new(
            Principal::user(user_id, "test", Vec::new()).expect("principal"),
            Some(tenant_id),
            None,
            TransportSecurity::Secure,
            None,
            None,
        )
        .expect("context");
        let mut resource = PermissionResource::new("users");
        resource.tenant_id = Some(other_tenant);
        resource.owner_user_id = Some(user_id);
        assert!(
            !PermissionExpr::Owner
                .evaluate(&backend, &context, Some(&resource), false)
                .await
                .expect("decision")
        );
        assert!(
            !PermissionExpr::OwnerOrReadOnly
                .evaluate(&backend, &context, Some(&resource), true)
                .await
                .expect("decision")
        );
    }

    #[tokio::test]
    async fn api_key_scope_caps_owner_permissions() {
        let user_id = Uuid::new_v4();
        let tenant_id = Uuid::new_v4();
        let membership_id = Uuid::new_v4();
        let repository = Arc::new(MemoryRepository::default());
        repository.memberships.lock().expect("memberships").insert(
            (user_id, tenant_id),
            MembershipSecurityRecord {
                id: membership_id,
                is_tenant_admin: false,
            },
        );
        repository.permissions.lock().expect("permissions").insert(
            membership_id,
            BTreeSet::from(["users.view_user".to_owned(), "users.delete_user".to_owned()]),
        );
        let backend = DatabasePermissionBackend::new(
            repository,
            DatabasePermissionConfig {
                global_admin_full_access: false,
                tenant_admin_full_access: false,
                allow_service_scopes: false,
                require_api_key_scope: true,
            },
        );
        let principal = Principal::api_key(
            user_id,
            Uuid::new_v4(),
            "api_key",
            vec!["users.view_user".to_owned()],
        )
        .expect("principal");
        let context = RequestContext::new(
            principal,
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
                .expect("decision")
                .allowed
        );
        assert!(
            !backend
                .has_permission(&context, "users.delete_user")
                .await
                .expect("decision")
                .allowed
        );
    }

    #[tokio::test]
    async fn empty_composition_and_owner_read_without_object_fail_closed() {
        let repository = Arc::new(MemoryRepository::default());
        let backend =
            DatabasePermissionBackend::new(repository, DatabasePermissionConfig::default());
        let context = RequestContext::new(
            Principal::anonymous(),
            None,
            None,
            TransportSecurity::Secure,
            None,
            None,
        )
        .expect("context");
        assert!(
            !PermissionExpr::HasAll(Vec::new())
                .evaluate(&backend, &context, None, false)
                .await
                .expect("decision")
        );
        assert!(
            !PermissionExpr::And(Vec::new())
                .evaluate(&backend, &context, None, false)
                .await
                .expect("decision")
        );
        assert!(
            !PermissionExpr::OwnerOrReadOnly
                .evaluate(&backend, &context, None, true)
                .await
                .expect("decision")
        );
    }

    #[tokio::test]
    async fn object_scope_rejects_cross_tenant_access() {
        let user_id = Uuid::new_v4();
        let tenant_id = Uuid::new_v4();
        let other_tenant = Uuid::new_v4();
        let repository = Arc::new(MemoryRepository::default());
        let backend =
            DatabasePermissionBackend::new(repository, DatabasePermissionConfig::default());
        let context = RequestContext::new(
            Principal::user(user_id, "test", Vec::new()).expect("principal"),
            Some(tenant_id),
            None,
            TransportSecurity::Secure,
            None,
            None,
        )
        .expect("context");
        let mut resource = PermissionResource::new("users");
        resource.tenant_id = Some(other_tenant);
        assert!(
            !backend
                .has_object_permission(&context, "users.view_user", &resource)
                .await
                .expect("decision")
                .allowed
        );
    }
    #[tokio::test]
    async fn scope_backend_maps_named_and_role_permissions_to_principal_scopes() {
        let backend = ScopePermissionBackend::default();
        let principal = Principal::user(
            Uuid::new_v4(),
            "test",
            vec!["articles.edit".to_owned(), "tenant:admin".to_owned()],
        )
        .expect("principal");
        let context = RequestContext::new(
            principal,
            Some(Uuid::new_v4()),
            None,
            TransportSecurity::Secure,
            None,
            None,
        )
        .expect("context");

        assert!(
            backend
                .has_permission(&context, "articles.edit")
                .await
                .expect("permission")
                .allowed
        );
        assert!(backend.is_tenant_member(&context).await.expect("member"));
        assert!(backend.is_tenant_admin(&context).await.expect("admin"));
        assert!(!backend.is_global_admin(&context).await.expect("global"));
    }

    #[test]
    fn permission_constructors_keep_application_code_declarative() {
        assert!(matches!(Permissions::public(), PermissionExpr::AllowAny));
        assert!(matches!(
            Permissions::authenticated(),
            PermissionExpr::Authenticated
        ));
        assert!(
            matches!(Permissions::named("articles.edit"), PermissionExpr::HasPermission(ref p) if p == "articles.edit")
        );
        assert!(
            matches!(Permissions::all(["a", "b"]), PermissionExpr::HasAll(ref p) if p.len() == 2)
        );
        assert!(
            matches!(Permissions::any(["a", "b"]), PermissionExpr::HasAny(ref p) if p.len() == 2)
        );
    }

    #[tokio::test]
    async fn deny_all_backend_fails_closed() {
        let backend = DenyAllPermissionBackend;
        let context = RequestContext::new(
            Principal::anonymous(),
            None,
            None,
            TransportSecurity::Secure,
            None,
            None,
        )
        .expect("context");
        assert!(
            !backend
                .has_permission(&context, "anything")
                .await
                .expect("decision")
                .allowed
        );
        assert!(
            !backend
                .is_tenant_member(&context)
                .await
                .expect("membership")
        );
        assert!(
            !backend
                .is_tenant_admin(&context)
                .await
                .expect("tenant admin")
        );
        assert!(
            !backend
                .is_global_admin(&context)
                .await
                .expect("global admin")
        );
    }
}
