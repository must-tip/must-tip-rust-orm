use async_trait::async_trait;
use db_controller::Entity;
use http::Method;
use serde::{Serialize, de::DeserializeOwned};
use sqlx::{FromRow, postgres::PgRow};
use std::sync::Arc;

use crate::rest_framework::{
    exceptions::ApiResult,
    generic::{GenericAPIView, RestModel},
    idempotency::IdempotencyPolicy,
    mixins::{
        CreateModelMixin, DestroyModelMixin, ListModelMixin, RetrieveModelMixin, UpdateModelMixin,
    },
    permissions::PermissionExpr,
    request::{ApiRequest, RequestContext},
    response::ApiResponse,
    routing::RouteSpec,
    schema::OperationMetadata,
    throttling::ThrottlePolicy,
    views::{APIView, ViewServices},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewSetActions {
    pub list: bool,
    pub retrieve: bool,
    pub create: bool,
    pub update: bool,
    pub partial_update: bool,
    pub destroy: bool,
}

impl ViewSetActions {
    pub const fn all() -> Self {
        Self {
            list: true,
            retrieve: true,
            create: true,
            update: true,
            partial_update: true,
            destroy: true,
        }
    }

    pub const fn read_only() -> Self {
        Self {
            list: true,
            retrieve: true,
            create: false,
            update: false,
            partial_update: false,
            destroy: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ViewSetOperationMetadata {
    pub list: Option<OperationMetadata>,
    pub retrieve: Option<OperationMetadata>,
    pub create: Option<OperationMetadata>,
    pub update: Option<OperationMetadata>,
    pub partial_update: Option<OperationMetadata>,
    pub destroy: Option<OperationMetadata>,
}

pub struct ModelViewSet<T> {
    core: Arc<GenericAPIView<T>>,
    name: &'static str,
    actions: ViewSetActions,
    read_permission: PermissionExpr,
    write_permission: PermissionExpr,
    throttles: Vec<ThrottlePolicy>,
    idempotency: IdempotencyPolicy,
    operation_metadata: ViewSetOperationMetadata,
}

impl<T> ModelViewSet<T> {
    pub fn new(name: &'static str, core: Arc<GenericAPIView<T>>) -> Self {
        Self {
            core,
            name,
            actions: ViewSetActions::all(),
            read_permission: PermissionExpr::Authenticated,
            write_permission: PermissionExpr::Authenticated,
            throttles: Vec::new(),
            idempotency: IdempotencyPolicy::default(),
            operation_metadata: ViewSetOperationMetadata::default(),
        }
    }

    pub fn actions(mut self, actions: ViewSetActions) -> Self {
        self.actions = actions;
        self
    }

    pub fn permissions(
        mut self,
        read_permission: PermissionExpr,
        write_permission: PermissionExpr,
    ) -> Self {
        self.read_permission = read_permission;
        self.write_permission = write_permission;
        self
    }

    pub fn throttles(mut self, throttles: Vec<ThrottlePolicy>) -> Self {
        self.throttles = throttles;
        self
    }

    pub fn idempotency(mut self, policy: IdempotencyPolicy) -> Self {
        self.idempotency = policy;
        self
    }

    pub fn operation_metadata(mut self, metadata: ViewSetOperationMetadata) -> Self {
        self.operation_metadata = metadata;
        self
    }

    /// Build the conventional collection/detail route pair used by DRF-style model viewsets.
    pub fn route_specs(self: &Arc<Self>, prefix: &str, basename: &str) -> ApiResult<Vec<RouteSpec>>
    where
        T: RestModel,
    {
        if prefix == "/"
            || !prefix.starts_with('/')
            || prefix.ends_with('/')
            || prefix.contains('?')
            || prefix.contains('#')
        {
            return Err(crate::rest_framework::exceptions::ApiError::configuration(
                "viewset prefix must be an absolute non-root path without a trailing slash",
            ));
        }
        if basename.is_empty()
            || basename.len() > 128
            || !basename
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
        {
            return Err(crate::rest_framework::exceptions::ApiError::configuration(
                "invalid viewset basename",
            ));
        }
        let detail = format!("{prefix}/{{{}}}", self.core.lookup_url_kwarg());
        Ok(vec![
            RouteSpec::http(prefix, format!("{basename}-list"), self.collection_view()),
            RouteSpec::http(detail, format!("{basename}-detail"), self.detail_view()),
        ])
    }

    pub fn collection_view(self: &Arc<Self>) -> Arc<dyn APIView>
    where
        T: Entity
            + Serialize
            + DeserializeOwned
            + for<'r> FromRow<'r, PgRow>
            + Send
            + Sync
            + Unpin
            + 'static,
    {
        Arc::new(CollectionView {
            viewset: Arc::clone(self),
        })
    }

    pub fn detail_view(self: &Arc<Self>) -> Arc<dyn APIView>
    where
        T: Entity
            + Serialize
            + DeserializeOwned
            + for<'r> FromRow<'r, PgRow>
            + Send
            + Sync
            + Unpin
            + 'static,
    {
        Arc::new(DetailView {
            viewset: Arc::clone(self),
        })
    }
}

impl<T> ListModelMixin<T> for ModelViewSet<T>
where
    T: Entity
        + Serialize
        + DeserializeOwned
        + for<'r> FromRow<'r, PgRow>
        + Send
        + Sync
        + Unpin
        + 'static,
{
    fn generic_view(&self) -> &GenericAPIView<T> {
        &self.core
    }
}
impl<T> RetrieveModelMixin<T> for ModelViewSet<T>
where
    T: Entity
        + Serialize
        + DeserializeOwned
        + for<'r> FromRow<'r, PgRow>
        + Send
        + Sync
        + Unpin
        + 'static,
{
    fn generic_view(&self) -> &GenericAPIView<T> {
        &self.core
    }
}
impl<T> CreateModelMixin<T> for ModelViewSet<T>
where
    T: Entity
        + Serialize
        + DeserializeOwned
        + for<'r> FromRow<'r, PgRow>
        + Send
        + Sync
        + Unpin
        + 'static,
{
    fn generic_view(&self) -> &GenericAPIView<T> {
        &self.core
    }
}
impl<T> UpdateModelMixin<T> for ModelViewSet<T>
where
    T: Entity
        + Serialize
        + DeserializeOwned
        + for<'r> FromRow<'r, PgRow>
        + Send
        + Sync
        + Unpin
        + 'static,
{
    fn generic_view(&self) -> &GenericAPIView<T> {
        &self.core
    }
}
impl<T> DestroyModelMixin<T> for ModelViewSet<T>
where
    T: Entity
        + Serialize
        + DeserializeOwned
        + for<'r> FromRow<'r, PgRow>
        + Send
        + Sync
        + Unpin
        + 'static,
{
    fn generic_view(&self) -> &GenericAPIView<T> {
        &self.core
    }
}

struct CollectionView<T> {
    viewset: Arc<ModelViewSet<T>>,
}
struct DetailView<T> {
    viewset: Arc<ModelViewSet<T>>,
}

#[async_trait]
impl<T> APIView for CollectionView<T>
where
    T: Entity
        + Serialize
        + DeserializeOwned
        + for<'r> FromRow<'r, PgRow>
        + Send
        + Sync
        + Unpin
        + 'static,
{
    fn name(&self) -> &'static str {
        self.viewset.name
    }

    fn allowed_methods(&self) -> Vec<Method> {
        let mut methods = Vec::new();
        if self.viewset.actions.list {
            methods.push(Method::GET);
        }
        if self.viewset.actions.create {
            methods.push(Method::POST);
        }
        methods
    }

    fn permission_policy(&self, method: &Method) -> PermissionExpr {
        if method == Method::GET || method == Method::HEAD {
            self.viewset.read_permission.clone()
        } else {
            self.viewset.write_permission.clone()
        }
    }

    fn throttle_policies(&self, _method: &Method) -> Vec<ThrottlePolicy> {
        self.viewset.throttles.clone()
    }

    fn idempotency_policy(&self, method: &Method) -> IdempotencyPolicy {
        if method == Method::POST {
            self.viewset.idempotency.clone()
        } else {
            IdempotencyPolicy::default()
        }
    }

    fn operation_metadata(&self, method: &Method) -> Option<OperationMetadata> {
        if method == Method::GET || method == Method::HEAD {
            self.viewset.operation_metadata.list.clone()
        } else if method == Method::POST {
            self.viewset.operation_metadata.create.clone()
        } else {
            None
        }
    }

    async fn get(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        _services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.viewset.list_model(request, context).await
    }

    async fn post(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        _services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.viewset.create_model(request, context).await
    }
}

#[async_trait]
impl<T> APIView for DetailView<T>
where
    T: Entity
        + Serialize
        + DeserializeOwned
        + for<'r> FromRow<'r, PgRow>
        + Send
        + Sync
        + Unpin
        + 'static,
{
    fn name(&self) -> &'static str {
        self.viewset.name
    }

    fn allowed_methods(&self) -> Vec<Method> {
        let mut methods = Vec::new();
        if self.viewset.actions.retrieve {
            methods.push(Method::GET);
        }
        if self.viewset.actions.update {
            methods.push(Method::PUT);
        }
        if self.viewset.actions.partial_update {
            methods.push(Method::PATCH);
        }
        if self.viewset.actions.destroy {
            methods.push(Method::DELETE);
        }
        methods
    }

    fn permission_policy(&self, method: &Method) -> PermissionExpr {
        if method == Method::GET || method == Method::HEAD {
            self.viewset.read_permission.clone()
        } else {
            self.viewset.write_permission.clone()
        }
    }

    fn throttle_policies(&self, _method: &Method) -> Vec<ThrottlePolicy> {
        self.viewset.throttles.clone()
    }

    fn idempotency_policy(&self, method: &Method) -> IdempotencyPolicy {
        if method == Method::PUT || method == Method::PATCH || method == Method::DELETE {
            self.viewset.idempotency.clone()
        } else {
            IdempotencyPolicy::default()
        }
    }

    fn operation_metadata(&self, method: &Method) -> Option<OperationMetadata> {
        if method == Method::GET || method == Method::HEAD {
            self.viewset.operation_metadata.retrieve.clone()
        } else if method == Method::PUT {
            self.viewset.operation_metadata.update.clone()
        } else if method == Method::PATCH {
            self.viewset.operation_metadata.partial_update.clone()
        } else if method == Method::DELETE {
            self.viewset.operation_metadata.destroy.clone()
        } else {
            None
        }
    }

    async fn get(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.viewset
            .retrieve_model(request, context, services)
            .await
    }

    async fn put(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.viewset
            .update_model(request, context, services, false)
            .await
    }

    async fn patch(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.viewset
            .update_model(request, context, services, true)
            .await
    }

    async fn delete(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.viewset.destroy_model(request, context, services).await
    }
}
