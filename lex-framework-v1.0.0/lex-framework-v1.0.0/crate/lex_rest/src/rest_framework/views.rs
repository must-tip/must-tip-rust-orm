use async_trait::async_trait;
use http::{Method, StatusCode};
use serde_json::json;
use std::sync::Arc;

use crate::rest_framework::{
    exceptions::{ApiError, ApiResult},
    idempotency::IdempotencyPolicy,
    permissions::{PermissionBackend, PermissionExpr, PermissionResource},
    request::{ApiRequest, RequestContext},
    response::{ApiBody, ApiResponse},
    schema::OperationMetadata,
    throttling::ThrottlePolicy,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewAccess {
    /// Secure default. Anonymous requests are rejected before the handler runs.
    Authenticated,
    /// Explicitly public endpoint. Authentication may still occur when credentials are supplied,
    /// but anonymous callers are allowed to reach the handler.
    Public,
}

/// Declarative per-method permission map for an `APIView`.
///
/// Views are authenticated by default. Override only the methods that differ.
/// This keeps permissions close to the view while avoiding repetitive method
/// matching in application code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewPermissions {
    default: PermissionExpr,
    get: Option<PermissionExpr>,
    post: Option<PermissionExpr>,
    put: Option<PermissionExpr>,
    patch: Option<PermissionExpr>,
    delete: Option<PermissionExpr>,
}

impl Default for ViewPermissions {
    fn default() -> Self {
        Self::authenticated()
    }
}

impl ViewPermissions {
    pub fn new(default: PermissionExpr) -> Self {
        Self {
            default,
            get: None,
            post: None,
            put: None,
            patch: None,
            delete: None,
        }
    }

    pub fn authenticated() -> Self {
        Self::new(PermissionExpr::Authenticated)
    }
    pub fn public() -> Self {
        Self::new(PermissionExpr::AllowAny)
    }

    /// Common API policy: anyone may read; authenticated users may mutate.
    pub fn public_read_authenticated_write() -> Self {
        Self::authenticated().get(PermissionExpr::AllowAny)
    }

    pub fn get(mut self, rule: PermissionExpr) -> Self {
        self.get = Some(rule);
        self
    }
    pub fn post(mut self, rule: PermissionExpr) -> Self {
        self.post = Some(rule);
        self
    }
    pub fn put(mut self, rule: PermissionExpr) -> Self {
        self.put = Some(rule);
        self
    }
    pub fn patch(mut self, rule: PermissionExpr) -> Self {
        self.patch = Some(rule);
        self
    }
    pub fn delete(mut self, rule: PermissionExpr) -> Self {
        self.delete = Some(rule);
        self
    }

    pub fn read(mut self, rule: PermissionExpr) -> Self {
        self.get = Some(rule);
        self
    }

    pub fn write(mut self, rule: PermissionExpr) -> Self {
        self.post = Some(rule.clone());
        self.put = Some(rule.clone());
        self.patch = Some(rule.clone());
        self.delete = Some(rule);
        self
    }

    pub fn for_method(&self, method: &Method) -> PermissionExpr {
        // Framework-owned OPTIONS responses expose route metadata/preflight behavior,
        // not protected business data. The requested business method remains protected.
        if method == Method::OPTIONS {
            PermissionExpr::AllowAny
        } else if method == Method::GET || method == Method::HEAD {
            self.get.clone().unwrap_or_else(|| self.default.clone())
        } else if method == Method::POST {
            self.post.clone().unwrap_or_else(|| self.default.clone())
        } else if method == Method::PUT {
            self.put.clone().unwrap_or_else(|| self.default.clone())
        } else if method == Method::PATCH {
            self.patch.clone().unwrap_or_else(|| self.default.clone())
        } else if method == Method::DELETE {
            self.delete.clone().unwrap_or_else(|| self.default.clone())
        } else {
            self.default.clone()
        }
    }
}

#[derive(Clone)]
pub struct ViewServices {
    permissions: Arc<dyn PermissionBackend>,
}

impl ViewServices {
    pub fn new(permissions: Arc<dyn PermissionBackend>) -> Self {
        Self { permissions }
    }

    pub fn permissions(&self) -> &dyn PermissionBackend {
        self.permissions.as_ref()
    }

    /// Require a named permission from inside business logic. This is useful
    /// when the rule depends on data that is only available after loading an
    /// object. View-level rules should still be declared with `ViewPermissions`.
    pub async fn require(&self, context: &RequestContext, permission: &str) -> ApiResult<()> {
        PermissionExpr::HasPermission(permission.to_owned())
            .enforce(self.permissions(), context, None, false)
            .await
    }

    /// Require object-level authorization with tenant/owner metadata.
    pub async fn require_object(
        &self,
        context: &RequestContext,
        rule: PermissionExpr,
        resource: &PermissionResource,
        read_only: bool,
    ) -> ApiResult<()> {
        rule.enforce(self.permissions(), context, Some(resource), read_only)
            .await
    }
}

#[async_trait]
pub trait APIView: Send + Sync {
    fn name(&self) -> &'static str;

    fn allowed_methods(&self) -> Vec<Method>;

    /// Secure default: every view requires an authenticated principal.
    /// Override `access()` with `ViewAccess::Public` for endpoints such as health checks,
    /// landing pages, registration, or login.
    fn access(&self) -> ViewAccess {
        ViewAccess::Authenticated
    }

    /// Declarative access rules for this view. The secure default is authenticated
    /// for every method. Override this method for per-method or named permissions.
    fn permissions(&self) -> ViewPermissions {
        match self.access() {
            ViewAccess::Authenticated => ViewPermissions::authenticated(),
            ViewAccess::Public => ViewPermissions::public(),
        }
    }

    /// Low-level compatibility hook. Existing implementations may continue to
    /// override this directly; new code should normally override `permissions()`.
    fn permission_policy(&self, method: &Method) -> PermissionExpr {
        self.permissions().for_method(method)
    }

    fn throttle_policies(&self, _method: &Method) -> Vec<ThrottlePolicy> {
        Vec::new()
    }

    fn idempotency_policy(&self, _method: &Method) -> IdempotencyPolicy {
        IdempotencyPolicy::default()
    }

    fn operation_metadata(&self, _method: &Method) -> Option<OperationMetadata> {
        None
    }

    fn method_not_allowed(&self) -> ApiError {
        let mut allowed = self
            .allowed_methods()
            .into_iter()
            .map(|method| method.as_str().to_owned())
            .collect::<std::collections::BTreeSet<_>>();
        if allowed.contains(Method::GET.as_str()) {
            allowed.insert(Method::HEAD.as_str().to_owned());
        }
        allowed.insert(Method::OPTIONS.as_str().to_owned());
        ApiError::method_not_allowed(allowed)
    }

    async fn get(
        &self,
        _request: &ApiRequest,
        _context: &RequestContext,
        _services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        Err(self.method_not_allowed())
    }

    async fn post(
        &self,
        _request: &ApiRequest,
        _context: &RequestContext,
        _services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        Err(self.method_not_allowed())
    }

    async fn put(
        &self,
        _request: &ApiRequest,
        _context: &RequestContext,
        _services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        Err(self.method_not_allowed())
    }

    async fn patch(
        &self,
        _request: &ApiRequest,
        _context: &RequestContext,
        _services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        Err(self.method_not_allowed())
    }

    async fn delete(
        &self,
        _request: &ApiRequest,
        _context: &RequestContext,
        _services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        Err(self.method_not_allowed())
    }

    async fn dispatch(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        context.ensure_not_expired()?;
        let allowed = self.allowed_methods();
        if allowed.is_empty()
            || allowed.iter().any(|method| {
                method != Method::GET
                    && method != Method::POST
                    && method != Method::PUT
                    && method != Method::PATCH
                    && method != Method::DELETE
            })
        {
            return Err(ApiError::configuration(
                "APIView allowed_methods must contain only GET/POST/PUT/PATCH/DELETE",
            ));
        }
        let method = request.method();
        let method_allowed = allowed.iter().any(|candidate| candidate == method)
            || (method == Method::HEAD && allowed.contains(&Method::GET))
            || method == Method::OPTIONS;
        if !method_allowed {
            return Err(self.method_not_allowed());
        }

        if method == Method::OPTIONS {
            return ApiResponse::json(
                StatusCode::OK,
                &json!({
                    "name": self.name(),
                    "methods": allowed.iter().map(Method::as_str).collect::<Vec<_>>(),
                }),
            );
        }

        let mut response = if method == Method::GET || method == Method::HEAD {
            self.get(request, context, services).await?
        } else if method == Method::POST {
            self.post(request, context, services).await?
        } else if method == Method::PUT {
            self.put(request, context, services).await?
        } else if method == Method::PATCH {
            self.patch(request, context, services).await?
        } else if method == Method::DELETE {
            self.delete(request, context, services).await?
        } else {
            return Err(self.method_not_allowed());
        };
        if method == Method::HEAD {
            let (status, headers, _) = response.into_parts();
            response = ApiResponse::new(status, ApiBody::Empty);
            *response.headers_mut() = headers;
        }
        Ok(response)
    }
}

pub fn is_read_only_method(method: &Method) -> bool {
    method == Method::GET || method == Method::HEAD || method == Method::OPTIONS
}

#[cfg(test)]
mod tests {
    use super::*;

    struct PrivateView;
    #[async_trait]
    impl APIView for PrivateView {
        fn name(&self) -> &'static str {
            "private"
        }
        fn allowed_methods(&self) -> Vec<Method> {
            vec![Method::GET]
        }
    }

    struct PublicView;
    #[async_trait]
    impl APIView for PublicView {
        fn name(&self) -> &'static str {
            "public"
        }
        fn allowed_methods(&self) -> Vec<Method> {
            vec![Method::GET]
        }
        fn access(&self) -> ViewAccess {
            ViewAccess::Public
        }
    }

    struct MixedView;
    #[async_trait]
    impl APIView for MixedView {
        fn name(&self) -> &'static str {
            "mixed"
        }
        fn allowed_methods(&self) -> Vec<Method> {
            vec![Method::GET, Method::POST, Method::DELETE]
        }
        fn permissions(&self) -> ViewPermissions {
            ViewPermissions::authenticated()
                .get(PermissionExpr::AllowAny)
                .post(PermissionExpr::HasPermission("articles.create".into()))
                .delete(PermissionExpr::TenantAdmin)
        }
    }

    #[test]
    fn view_permissions_support_per_method_rules() {
        assert!(matches!(
            MixedView.permission_policy(&Method::GET),
            PermissionExpr::AllowAny
        ));
        assert!(
            matches!(MixedView.permission_policy(&Method::POST), PermissionExpr::HasPermission(ref p) if p == "articles.create")
        );
        assert!(matches!(
            MixedView.permission_policy(&Method::DELETE),
            PermissionExpr::TenantAdmin
        ));
        assert!(matches!(
            MixedView.permission_policy(&Method::PATCH),
            PermissionExpr::Authenticated
        ));
        assert!(matches!(
            MixedView.permission_policy(&Method::OPTIONS),
            PermissionExpr::AllowAny
        ));
    }

    #[test]
    fn protected_is_the_default_view_access() {
        assert!(matches!(
            PrivateView.permission_policy(&Method::GET),
            PermissionExpr::Authenticated
        ));
    }

    #[test]
    fn public_view_maps_to_allow_any() {
        assert!(matches!(
            PublicView.permission_policy(&Method::GET),
            PermissionExpr::AllowAny
        ));
    }
}
