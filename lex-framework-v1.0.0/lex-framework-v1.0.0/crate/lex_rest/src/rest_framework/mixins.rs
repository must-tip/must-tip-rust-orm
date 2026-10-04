use async_trait::async_trait;
use db_controller::Entity;
use serde::{Serialize, de::DeserializeOwned};
use sqlx::{FromRow, postgres::PgRow};

use crate::rest_framework::{
    exceptions::ApiResult,
    generic::GenericAPIView,
    request::{ApiRequest, RequestContext},
    response::ApiResponse,
    views::ViewServices,
};

#[async_trait]
pub trait ListModelMixin<T>: Send + Sync
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
    fn generic_view(&self) -> &GenericAPIView<T>;

    async fn list_model(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
    ) -> ApiResult<ApiResponse> {
        self.generic_view().list(request, context).await
    }
}

#[async_trait]
pub trait RetrieveModelMixin<T>: Send + Sync
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
    fn generic_view(&self) -> &GenericAPIView<T>;

    async fn retrieve_model(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.generic_view()
            .retrieve(request, context, services)
            .await
    }
}

#[async_trait]
pub trait CreateModelMixin<T>: Send + Sync
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
    fn generic_view(&self) -> &GenericAPIView<T>;

    async fn create_model(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
    ) -> ApiResult<ApiResponse> {
        self.generic_view().create(request, context).await
    }
}

#[async_trait]
pub trait UpdateModelMixin<T>: Send + Sync
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
    fn generic_view(&self) -> &GenericAPIView<T>;

    async fn update_model(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
        partial: bool,
    ) -> ApiResult<ApiResponse> {
        self.generic_view()
            .update(request, context, services, partial)
            .await
    }
}

#[async_trait]
pub trait DestroyModelMixin<T>: Send + Sync
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
    fn generic_view(&self) -> &GenericAPIView<T>;

    async fn destroy_model(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.generic_view()
            .destroy(request, context, services)
            .await
    }
}
