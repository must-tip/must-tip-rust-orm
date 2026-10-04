use async_trait::async_trait;
use db_controller::{
    AtomicTransaction, Entity, ModelManager, PoolExecutor, QueryExecutor, QuerySet,
    RestAccessPolicy, RestAction, RestQueryPolicy, SerializerMode, SerializerPolicy,
    TransactionOptions, dynamic_predicate,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};
use sqlx::{FromRow, postgres::PgRow};
use std::{marker::PhantomData, sync::Arc};
use uuid::Uuid;

use crate::rest_framework::{
    exceptions::{ApiError, ApiResult},
    permissions::{PermissionExpr, PermissionResource},
    request::{ApiRequest, RequestContext},
    response::ApiResponse,
    views::ViewServices,
};

pub trait ModelFactory<T>: Send + Sync {
    /// Build a model for creation from transport-validated fields. The factory owns application
    /// defaults and framework-owned values (tenant, identifiers, actor metadata); client input is
    /// never allowed to set those fields through `SerializerPolicy`.
    fn build_create(&self, context: &RequestContext, validated: Map<String, Value>)
    -> ApiResult<T>;
}

#[async_trait]
pub trait ModelMutationAdapter<T>: Send + Sync {
    async fn create(&self, context: &RequestContext, validated: Map<String, Value>)
    -> ApiResult<T>;

    async fn update(
        &self,
        context: &RequestContext,
        current: &T,
        validated: Map<String, Value>,
        partial: bool,
    ) -> ApiResult<T>;

    async fn destroy(&self, context: &RequestContext, current: &T) -> ApiResult<()>;
}

pub struct OrmModelMutationAdapter<T, F> {
    pool: sqlx::PgPool,
    factory: Arc<F>,
    hard_delete_without_soft_delete: bool,
    _marker: PhantomData<fn() -> T>,
}

impl<T, F> OrmModelMutationAdapter<T, F> {
    pub fn new(pool: sqlx::PgPool, factory: Arc<F>) -> Self {
        Self {
            pool,
            factory,
            hard_delete_without_soft_delete: true,
            _marker: PhantomData,
        }
    }

    pub fn hard_delete_without_soft_delete(mut self, allow: bool) -> Self {
        self.hard_delete_without_soft_delete = allow;
        self
    }
}

#[async_trait]
impl<T, F> ModelMutationAdapter<T> for OrmModelMutationAdapter<T, F>
where
    T: Entity
        + Serialize
        + DeserializeOwned
        + for<'r> FromRow<'r, PgRow>
        + Send
        + Sync
        + Unpin
        + 'static,
    F: ModelFactory<T> + 'static,
{
    async fn create(
        &self,
        context: &RequestContext,
        validated: Map<String, Value>,
    ) -> ApiResult<T> {
        let model = self.factory.build_create(context, validated)?;
        model
            .validate_model()
            .map_err(|detail| ApiError::bad_request("model_validation_error", detail))?;
        scoped_queryset::<T>(&self.pool, context)?
            .create(&model)
            .await
            .map_err(ApiError::from)
    }

    async fn update(
        &self,
        context: &RequestContext,
        current: &T,
        validated: Map<String, Value>,
        _partial: bool,
    ) -> ApiResult<T> {
        if validated.is_empty() {
            return scoped_queryset::<T>(&self.pool, context)?
                .filter(primary_key_predicate(current)?)
                .get()
                .await
                .map_err(ApiError::from);
        }

        let tenant_scope = if T::tenant_column().is_some() {
            Some(
                context
                    .tenant_id()
                    .ok_or_else(|| ApiError::permission_denied("tenant context is required"))?,
            )
        } else {
            None
        };
        let mut transaction = AtomicTransaction::begin(&self.pool, TransactionOptions::default())
            .await
            .map_err(ApiError::from)?;
        if let Some(tenant_id) = tenant_scope
            && let Err(error) = transaction.set_tenant(tenant_id).await
        {
            let _ = transaction.rollback().await;
            return Err(ApiError::from(error));
        }

        let operation = async {
            let predicate = primary_key_predicate(current)?;
            let queryset =
                scope_queryset::<T, _>(transaction.queryset::<T>(), context)?.filter(predicate);
            if let Some(version_column) = T::version_column() {
                let map = current.to_map();
                let version = map
                    .get(version_column)
                    .and_then(Value::as_i64)
                    .ok_or_else(|| {
                        ApiError::internal("versioned model emitted invalid version value")
                    })?;
                queryset
                    .update_versioned(version, validated)
                    .await
                    .map_err(ApiError::from)?;
            } else {
                let affected = queryset.update(validated).await.map_err(ApiError::from)?;
                if affected != 1 {
                    return Err(ApiError::conflict(
                        "unexpected_update_count",
                        "update did not affect exactly one resource",
                    ));
                }
            }

            let updated = scope_queryset::<T, _>(transaction.queryset::<T>(), context)?
                .filter(primary_key_predicate(current)?)
                .get()
                .await
                .map_err(ApiError::from)?;
            updated
                .validate_model()
                .map_err(|detail| ApiError::bad_request("model_validation_error", detail))?;
            Ok::<T, ApiError>(updated)
        }
        .await;

        match operation {
            Ok(updated) => {
                transaction.commit().await.map_err(ApiError::from)?;
                Ok(updated)
            }
            Err(error) => {
                transaction.rollback().await.map_err(|rollback_error| {
                    tracing::error!(
                        error = %rollback_error,
                        "REST model update failed and transaction rollback also failed"
                    );
                    ApiError::internal("database transaction rollback failed")
                })?;
                Err(error)
            }
        }
    }

    async fn destroy(&self, context: &RequestContext, current: &T) -> ApiResult<()> {
        let queryset =
            scoped_queryset::<T>(&self.pool, context)?.filter(primary_key_predicate(current)?);
        let affected = if T::soft_delete_column().is_some() {
            queryset.soft_delete().await.map_err(ApiError::from)?
        } else if self.hard_delete_without_soft_delete {
            queryset.hard_delete().await.map_err(ApiError::from)?
        } else {
            return Err(ApiError::conflict(
                "hard_delete_disabled",
                "model does not support soft deletion and hard deletion is disabled",
            ));
        };
        if affected != 1 {
            return Err(ApiError::not_found());
        }
        Ok(())
    }
}

pub struct GenericAPIView<T> {
    pool: sqlx::PgPool,
    serializer: SerializerPolicy,
    query_policy: RestQueryPolicy,
    mutation: Option<Arc<dyn ModelMutationAdapter<T>>>,
    access_policy: Option<Arc<dyn RestAccessPolicy<T>>>,
    lookup_field: String,
    lookup_url_kwarg: String,
    owner_field: Option<String>,
    object_permission: PermissionExpr,
    _marker: PhantomData<fn() -> T>,
}

impl<T> GenericAPIView<T>
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
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            pool,
            serializer: SerializerPolicy::for_model::<T>(),
            query_policy: RestQueryPolicy::default(),
            mutation: None,
            access_policy: None,
            lookup_field: T::primary_key().to_owned(),
            lookup_url_kwarg: T::primary_key().to_owned(),
            owner_field: None,
            object_permission: PermissionExpr::Authenticated,
            _marker: PhantomData,
        }
    }

    pub fn serializer_policy(mut self, serializer: SerializerPolicy) -> Self {
        self.serializer = serializer;
        self
    }

    pub fn query_policy(mut self, query_policy: RestQueryPolicy) -> Self {
        self.query_policy = query_policy;
        self
    }

    pub fn mutation_adapter(mut self, mutation: Arc<dyn ModelMutationAdapter<T>>) -> Self {
        self.mutation = Some(mutation);
        self
    }

    /// Reuse the database layer's transport-neutral row/payload/instance policy as an additional
    /// application authorization boundary. `lex_rest` permissions still run independently.
    pub fn access_policy(mut self, access_policy: Arc<dyn RestAccessPolicy<T>>) -> Self {
        self.access_policy = Some(access_policy);
        self
    }

    pub fn lookup(
        mut self,
        model_field: impl Into<String>,
        url_kwarg: impl Into<String>,
    ) -> ApiResult<Self> {
        let model_field = model_field.into();
        if !T::schema().contains_key(&model_field) {
            return Err(ApiError::configuration("lookup field is not a model field"));
        }
        let url_kwarg = url_kwarg.into();
        if !valid_route_parameter_name(&url_kwarg) {
            return Err(ApiError::configuration("invalid lookup URL parameter name"));
        }
        self.lookup_field = model_field;
        self.lookup_url_kwarg = url_kwarg;
        Ok(self)
    }

    pub fn owner_field(mut self, field: impl Into<String>) -> ApiResult<Self> {
        let field = field.into();
        let schema = T::schema();
        let Some(spec) = schema.get(&field) else {
            return Err(ApiError::configuration("owner field is not a model field"));
        };
        if !spec.sql_type.eq_ignore_ascii_case("UUID") {
            return Err(ApiError::configuration("owner field must be UUID-backed"));
        }
        self.owner_field = Some(field);
        Ok(self)
    }

    pub fn object_permission(mut self, permission: PermissionExpr) -> Self {
        self.object_permission = permission;
        self
    }

    pub fn serializer(&self) -> &SerializerPolicy {
        &self.serializer
    }

    pub fn query_policy_ref(&self) -> &RestQueryPolicy {
        &self.query_policy
    }

    pub fn lookup_url_kwarg(&self) -> &str {
        &self.lookup_url_kwarg
    }

    pub fn queryset(&self, context: &RequestContext) -> ApiResult<QuerySet<T, PoolExecutor>> {
        scoped_queryset::<T>(&self.pool, context)
    }

    pub async fn list(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
    ) -> ApiResult<ApiResponse> {
        let params = request.query_single_map()?;
        let applied = self.apply_query_policy(&params, self.queryset(context)?)?;
        let cursor_policy = applied.cursor_page_policy().cloned();
        let rows = applied.queryset.all().await.map_err(ApiError::from)?;
        if let Some(cursor_policy) = cursor_policy {
            let page = cursor_policy.finalize(rows).map_err(ApiError::from)?;
            let results = page
                .results
                .iter()
                .map(|row| self.serializer.serialize(row).map_err(ApiError::from))
                .collect::<ApiResult<Vec<_>>>()?;
            return ApiResponse::json(
                http::StatusCode::OK,
                &json!({"results": results, "next": page.next}),
            );
        }

        let count_applied = self.apply_query_policy(&params, self.queryset(context)?)?;
        let total = count_applied
            .queryset
            .count()
            .await
            .map_err(ApiError::from)?;
        let results = rows
            .iter()
            .map(|row| self.serializer.serialize(row).map_err(ApiError::from))
            .collect::<ApiResult<Vec<_>>>()?;
        ApiResponse::json(
            http::StatusCode::OK,
            &json!({"count": total, "results": results}),
        )
    }

    pub async fn retrieve(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        let object = self.get_object(request, context, services, true).await?;
        let value = self.serializer.serialize(&object).map_err(ApiError::from)?;
        ApiResponse::json(http::StatusCode::OK, &value)
    }

    pub async fn create(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
    ) -> ApiResult<ApiResponse> {
        ensure_json_content_type(request)?;
        let body = request.json()?;
        let validated = if let Some(access) = self.access_policy.as_deref() {
            self.serializer
                .validate_input_authorized::<T, _>(&body, SerializerMode::Create, access)
                .map_err(ApiError::from)?
        } else {
            self.serializer
                .validate_input(&body, SerializerMode::Create)
                .map_err(ApiError::from)?
        };
        let mutation = self.mutation.as_ref().ok_or_else(|| {
            ApiError::configuration("create endpoint has no model mutation adapter")
        })?;
        let object = mutation.create(context, validated).await?;
        self.serializer
            .validate_model(&object)
            .map_err(ApiError::from)?;
        let value = self.serializer.serialize(&object).map_err(ApiError::from)?;
        ApiResponse::json(http::StatusCode::CREATED, &value)
    }

    pub async fn update(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
        partial: bool,
    ) -> ApiResult<ApiResponse> {
        ensure_json_content_type(request)?;
        let action = if partial {
            RestAction::PartialUpdate
        } else {
            RestAction::Update
        };
        let current = self
            .get_object_for_action(request, context, services, action, false)
            .await?;
        let body = request.json()?;
        let mode = if partial {
            SerializerMode::PartialUpdate
        } else {
            SerializerMode::Update
        };
        let validated = if let Some(access) = self.access_policy.as_deref() {
            self.serializer
                .validate_input_authorized::<T, _>(&body, mode, access)
                .map_err(ApiError::from)?
        } else {
            self.serializer
                .validate_input(&body, mode)
                .map_err(ApiError::from)?
        };
        let mutation = self.mutation.as_ref().ok_or_else(|| {
            ApiError::configuration("update endpoint has no model mutation adapter")
        })?;
        let updated = mutation
            .update(context, &current, validated, partial)
            .await?;
        self.serializer
            .validate_model(&updated)
            .map_err(ApiError::from)?;
        let value = self
            .serializer
            .serialize(&updated)
            .map_err(ApiError::from)?;
        ApiResponse::json(http::StatusCode::OK, &value)
    }

    pub async fn destroy(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        let current = self
            .get_object_for_action(request, context, services, RestAction::Destroy, false)
            .await?;
        let mutation = self.mutation.as_ref().ok_or_else(|| {
            ApiError::configuration("delete endpoint has no model mutation adapter")
        })?;
        mutation.destroy(context, &current).await?;
        Ok(ApiResponse::empty(http::StatusCode::NO_CONTENT))
    }

    pub async fn get_object(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
        read_only: bool,
    ) -> ApiResult<T> {
        let action = if read_only {
            RestAction::Retrieve
        } else {
            RestAction::Update
        };
        self.get_object_for_action(request, context, services, action, read_only)
            .await
    }

    async fn get_object_for_action(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
        action: RestAction,
        read_only: bool,
    ) -> ApiResult<T> {
        let raw = request
            .path_param(&self.lookup_url_kwarg)
            .ok_or_else(ApiError::not_found)?;
        let predicate = dynamic_predicate::<T>(&self.lookup_field, "exact", raw)
            .map_err(|_| ApiError::not_found())?;
        let object = self
            .apply_access_scope(action, self.queryset(context)?)?
            .filter(predicate)
            .get()
            .await
            .map_err(ApiError::from)?;
        if let Some(access) = self.access_policy.as_deref() {
            access
                .authorize_instance(action, &object)
                .map_err(ApiError::from)?;
        }
        let resource = self.permission_resource(&object)?;
        self.object_permission
            .enforce(services.permissions(), context, Some(&resource), read_only)
            .await?;
        Ok(object)
    }

    fn apply_access_scope(
        &self,
        action: RestAction,
        queryset: QuerySet<T, PoolExecutor>,
    ) -> ApiResult<QuerySet<T, PoolExecutor>> {
        let Some(access) = self.access_policy.as_deref() else {
            return Ok(queryset);
        };
        match access.scope_predicate(action).map_err(ApiError::from)? {
            Some(predicate) => Ok(queryset.filter(predicate)),
            None => Ok(queryset),
        }
    }

    fn apply_query_policy(
        &self,
        params: &std::collections::HashMap<String, String>,
        queryset: QuerySet<T, PoolExecutor>,
    ) -> ApiResult<db_controller::AppliedRestQuery<T, PoolExecutor>> {
        if let Some(access) = self.access_policy.as_deref() {
            self.query_policy
                .apply_authorized::<T, _, _>(params, queryset, access)
                .map_err(ApiError::from)
        } else {
            self.query_policy
                .apply::<T, _>(params, queryset)
                .map_err(ApiError::from)
        }
    }

    pub fn permission_resource(&self, object: &T) -> ApiResult<PermissionResource> {
        let map = object.to_map();
        let resource_id = map
            .get(T::primary_key())
            .map(json_scalar_to_string)
            .transpose()?;
        let tenant_id = T::tenant_column()
            .and_then(|field| map.get(field))
            .map(json_uuid)
            .transpose()?;
        let owner_user_id = self
            .owner_field
            .as_deref()
            .and_then(|field| map.get(field))
            .map(json_uuid)
            .transpose()?;
        Ok(PermissionResource {
            resource_type: T::table_name().to_owned(),
            resource_id,
            tenant_id,
            owner_user_id,
        })
    }
}

pub(crate) fn scoped_queryset<T>(
    pool: &sqlx::PgPool,
    context: &RequestContext,
) -> ApiResult<QuerySet<T, PoolExecutor>>
where
    T: Entity + for<'r> FromRow<'r, PgRow> + Send + Sync + Unpin,
{
    scope_queryset(ModelManager::<T>::objects(pool.clone()).queryset(), context)
}

fn scope_queryset<T, E>(
    queryset: QuerySet<T, E>,
    context: &RequestContext,
) -> ApiResult<QuerySet<T, E>>
where
    T: Entity,
    E: QueryExecutor,
{
    if T::tenant_column().is_some() {
        let tenant_id = context
            .tenant_id()
            .ok_or_else(|| ApiError::permission_denied("tenant context is required"))?;
        Ok(queryset.for_tenant(tenant_id))
    } else {
        Ok(queryset)
    }
}

fn primary_key_predicate<T: Entity>(object: &T) -> ApiResult<db_controller::Predicate> {
    let map = object.to_map();
    let value = map
        .get(T::primary_key())
        .ok_or_else(|| ApiError::internal("model did not emit its primary key"))?;
    let raw = json_scalar_to_string(value)?;
    dynamic_predicate::<T>(T::primary_key(), "exact", &raw).map_err(ApiError::from)
}

fn json_scalar_to_string(value: &Value) -> ApiResult<String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Number(value) => Ok(value.to_string()),
        Value::Bool(value) => Ok(value.to_string()),
        _ => Err(ApiError::internal("model identifier is not scalar")),
    }
}

fn json_uuid(value: &Value) -> ApiResult<Uuid> {
    value
        .as_str()
        .ok_or_else(|| ApiError::internal("UUID model field is not a string"))?
        .parse()
        .map_err(|_| ApiError::internal("UUID model field is invalid"))
}

fn valid_route_parameter_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().enumerate().all(|(index, ch)| {
            ch == '_' || (ch.is_ascii_alphanumeric() && (index > 0 || !ch.is_ascii_digit()))
        })
}

fn ensure_json_content_type(request: &ApiRequest) -> ApiResult<()> {
    let Some(content_type) = request
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    else {
        return Err(ApiError::unsupported_media_type());
    };
    let media_type = content_type
        .split(';')
        .next()
        .map(str::trim)
        .unwrap_or_default();
    if media_type == "application/json" || media_type.ends_with("+json") {
        Ok(())
    } else {
        Err(ApiError::unsupported_media_type())
    }
}

/// Concrete DRF-style list endpoint.
pub struct ListAPIView<T> {
    core: Arc<GenericAPIView<T>>,
    name: &'static str,
    permission: PermissionExpr,
}

impl<T> ListAPIView<T> {
    pub fn new(name: &'static str, core: Arc<GenericAPIView<T>>) -> Self {
        Self {
            core,
            name,
            permission: PermissionExpr::Authenticated,
        }
    }

    pub fn permission(mut self, permission: PermissionExpr) -> Self {
        self.permission = permission;
        self
    }
}

/// Concrete DRF-style list/create endpoint.
pub struct ListCreateAPIView<T> {
    core: Arc<GenericAPIView<T>>,
    name: &'static str,
    read_permission: PermissionExpr,
    write_permission: PermissionExpr,
    idempotency: crate::rest_framework::idempotency::IdempotencyPolicy,
}

impl<T> ListCreateAPIView<T> {
    pub fn new(name: &'static str, core: Arc<GenericAPIView<T>>) -> Self {
        Self {
            core,
            name,
            read_permission: PermissionExpr::Authenticated,
            write_permission: PermissionExpr::Authenticated,
            idempotency: crate::rest_framework::idempotency::IdempotencyPolicy::default(),
        }
    }

    pub fn permissions(mut self, read: PermissionExpr, write: PermissionExpr) -> Self {
        self.read_permission = read;
        self.write_permission = write;
        self
    }

    pub fn idempotency(
        mut self,
        policy: crate::rest_framework::idempotency::IdempotencyPolicy,
    ) -> Self {
        self.idempotency = policy;
        self
    }
}

/// Concrete DRF-style retrieve endpoint.
pub struct RetrieveAPIView<T> {
    core: Arc<GenericAPIView<T>>,
    name: &'static str,
    permission: PermissionExpr,
}

impl<T> RetrieveAPIView<T> {
    pub fn new(name: &'static str, core: Arc<GenericAPIView<T>>) -> Self {
        Self {
            core,
            name,
            permission: PermissionExpr::Authenticated,
        }
    }

    pub fn permission(mut self, permission: PermissionExpr) -> Self {
        self.permission = permission;
        self
    }
}

/// Concrete DRF-style retrieve/update endpoint.
pub struct RetrieveUpdateAPIView<T> {
    core: Arc<GenericAPIView<T>>,
    name: &'static str,
    read_permission: PermissionExpr,
    write_permission: PermissionExpr,
    idempotency: crate::rest_framework::idempotency::IdempotencyPolicy,
}

impl<T> RetrieveUpdateAPIView<T> {
    pub fn new(name: &'static str, core: Arc<GenericAPIView<T>>) -> Self {
        Self {
            core,
            name,
            read_permission: PermissionExpr::Authenticated,
            write_permission: PermissionExpr::Authenticated,
            idempotency: crate::rest_framework::idempotency::IdempotencyPolicy::default(),
        }
    }

    pub fn permissions(mut self, read: PermissionExpr, write: PermissionExpr) -> Self {
        self.read_permission = read;
        self.write_permission = write;
        self
    }

    pub fn idempotency(
        mut self,
        policy: crate::rest_framework::idempotency::IdempotencyPolicy,
    ) -> Self {
        self.idempotency = policy;
        self
    }
}

/// Concrete DRF-style retrieve/update/destroy endpoint.
pub struct RetrieveUpdateDestroyAPIView<T> {
    core: Arc<GenericAPIView<T>>,
    name: &'static str,
    read_permission: PermissionExpr,
    write_permission: PermissionExpr,
    idempotency: crate::rest_framework::idempotency::IdempotencyPolicy,
}

impl<T> RetrieveUpdateDestroyAPIView<T> {
    pub fn new(name: &'static str, core: Arc<GenericAPIView<T>>) -> Self {
        Self {
            core,
            name,
            read_permission: PermissionExpr::Authenticated,
            write_permission: PermissionExpr::Authenticated,
            idempotency: crate::rest_framework::idempotency::IdempotencyPolicy::default(),
        }
    }

    pub fn permissions(mut self, read: PermissionExpr, write: PermissionExpr) -> Self {
        self.read_permission = read;
        self.write_permission = write;
        self
    }

    pub fn idempotency(
        mut self,
        policy: crate::rest_framework::idempotency::IdempotencyPolicy,
    ) -> Self {
        self.idempotency = policy;
        self
    }
}

/// Concrete DRF-style create-only endpoint.
pub struct CreateAPIView<T> {
    core: Arc<GenericAPIView<T>>,
    name: &'static str,
    permission: PermissionExpr,
    idempotency: crate::rest_framework::idempotency::IdempotencyPolicy,
}

impl<T> CreateAPIView<T> {
    pub fn new(name: &'static str, core: Arc<GenericAPIView<T>>) -> Self {
        Self {
            core,
            name,
            permission: PermissionExpr::Authenticated,
            idempotency: Default::default(),
        }
    }

    pub fn permission(mut self, permission: PermissionExpr) -> Self {
        self.permission = permission;
        self
    }

    pub fn idempotency(
        mut self,
        policy: crate::rest_framework::idempotency::IdempotencyPolicy,
    ) -> Self {
        self.idempotency = policy;
        self
    }
}

/// Concrete DRF-style update-only endpoint.
pub struct UpdateAPIView<T> {
    core: Arc<GenericAPIView<T>>,
    name: &'static str,
    permission: PermissionExpr,
    idempotency: crate::rest_framework::idempotency::IdempotencyPolicy,
}

impl<T> UpdateAPIView<T> {
    pub fn new(name: &'static str, core: Arc<GenericAPIView<T>>) -> Self {
        Self {
            core,
            name,
            permission: PermissionExpr::Authenticated,
            idempotency: Default::default(),
        }
    }

    pub fn permission(mut self, permission: PermissionExpr) -> Self {
        self.permission = permission;
        self
    }

    pub fn idempotency(
        mut self,
        policy: crate::rest_framework::idempotency::IdempotencyPolicy,
    ) -> Self {
        self.idempotency = policy;
        self
    }
}

/// Concrete DRF-style destroy-only endpoint.
pub struct DestroyAPIView<T> {
    core: Arc<GenericAPIView<T>>,
    name: &'static str,
    permission: PermissionExpr,
    idempotency: crate::rest_framework::idempotency::IdempotencyPolicy,
}

impl<T> DestroyAPIView<T> {
    pub fn new(name: &'static str, core: Arc<GenericAPIView<T>>) -> Self {
        Self {
            core,
            name,
            permission: PermissionExpr::Authenticated,
            idempotency: Default::default(),
        }
    }

    pub fn permission(mut self, permission: PermissionExpr) -> Self {
        self.permission = permission;
        self
    }

    pub fn idempotency(
        mut self,
        policy: crate::rest_framework::idempotency::IdempotencyPolicy,
    ) -> Self {
        self.idempotency = policy;
        self
    }
}

/// Concrete DRF-style retrieve/destroy endpoint.
pub struct RetrieveDestroyAPIView<T> {
    core: Arc<GenericAPIView<T>>,
    name: &'static str,
    read_permission: PermissionExpr,
    write_permission: PermissionExpr,
    idempotency: crate::rest_framework::idempotency::IdempotencyPolicy,
}

impl<T> RetrieveDestroyAPIView<T> {
    pub fn new(name: &'static str, core: Arc<GenericAPIView<T>>) -> Self {
        Self {
            core,
            name,
            read_permission: PermissionExpr::Authenticated,
            write_permission: PermissionExpr::Authenticated,
            idempotency: Default::default(),
        }
    }

    pub fn permissions(mut self, read: PermissionExpr, write: PermissionExpr) -> Self {
        self.read_permission = read;
        self.write_permission = write;
        self
    }

    pub fn idempotency(
        mut self,
        policy: crate::rest_framework::idempotency::IdempotencyPolicy,
    ) -> Self {
        self.idempotency = policy;
        self
    }
}

/// Common bound for models that can participate in the transport-neutral REST generic views.
pub trait RestModel:
    Entity + Serialize + DeserializeOwned + for<'r> FromRow<'r, PgRow> + Send + Sync + Unpin + 'static
{
}

impl<T> RestModel for T where
    T: Entity
        + Serialize
        + DeserializeOwned
        + for<'r> FromRow<'r, PgRow>
        + Send
        + Sync
        + Unpin
        + 'static
{
}

#[async_trait::async_trait]
impl<T> crate::rest_framework::views::APIView for ListAPIView<T>
where
    T: RestModel,
{
    fn name(&self) -> &'static str {
        self.name
    }
    fn allowed_methods(&self) -> Vec<http::Method> {
        vec![http::Method::GET]
    }
    fn permission_policy(&self, _: &http::Method) -> PermissionExpr {
        self.permission.clone()
    }
    async fn get(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        _: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.core.list(request, context).await
    }
}

#[async_trait::async_trait]
impl<T> crate::rest_framework::views::APIView for ListCreateAPIView<T>
where
    T: RestModel,
{
    fn name(&self) -> &'static str {
        self.name
    }
    fn allowed_methods(&self) -> Vec<http::Method> {
        vec![http::Method::GET, http::Method::POST]
    }
    fn permission_policy(&self, method: &http::Method) -> PermissionExpr {
        if method == http::Method::GET || method == http::Method::HEAD {
            self.read_permission.clone()
        } else {
            self.write_permission.clone()
        }
    }
    fn idempotency_policy(
        &self,
        method: &http::Method,
    ) -> crate::rest_framework::idempotency::IdempotencyPolicy {
        if method == http::Method::POST {
            self.idempotency.clone()
        } else {
            Default::default()
        }
    }
    async fn get(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        _: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.core.list(request, context).await
    }
    async fn post(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        _: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.core.create(request, context).await
    }
}

#[async_trait::async_trait]
impl<T> crate::rest_framework::views::APIView for RetrieveAPIView<T>
where
    T: RestModel,
{
    fn name(&self) -> &'static str {
        self.name
    }
    fn allowed_methods(&self) -> Vec<http::Method> {
        vec![http::Method::GET]
    }
    fn permission_policy(&self, _: &http::Method) -> PermissionExpr {
        self.permission.clone()
    }
    async fn get(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.core.retrieve(request, context, services).await
    }
}

#[async_trait::async_trait]
impl<T> crate::rest_framework::views::APIView for RetrieveUpdateAPIView<T>
where
    T: RestModel,
{
    fn name(&self) -> &'static str {
        self.name
    }
    fn allowed_methods(&self) -> Vec<http::Method> {
        vec![http::Method::GET, http::Method::PUT, http::Method::PATCH]
    }
    fn permission_policy(&self, method: &http::Method) -> PermissionExpr {
        if method == http::Method::GET || method == http::Method::HEAD {
            self.read_permission.clone()
        } else {
            self.write_permission.clone()
        }
    }
    fn idempotency_policy(
        &self,
        method: &http::Method,
    ) -> crate::rest_framework::idempotency::IdempotencyPolicy {
        if method == http::Method::PUT || method == http::Method::PATCH {
            self.idempotency.clone()
        } else {
            Default::default()
        }
    }
    async fn get(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.core.retrieve(request, context, services).await
    }
    async fn put(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.core.update(request, context, services, false).await
    }
    async fn patch(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.core.update(request, context, services, true).await
    }
}

#[async_trait::async_trait]
impl<T> crate::rest_framework::views::APIView for RetrieveUpdateDestroyAPIView<T>
where
    T: RestModel,
{
    fn name(&self) -> &'static str {
        self.name
    }
    fn allowed_methods(&self) -> Vec<http::Method> {
        vec![
            http::Method::GET,
            http::Method::PUT,
            http::Method::PATCH,
            http::Method::DELETE,
        ]
    }
    fn permission_policy(&self, method: &http::Method) -> PermissionExpr {
        if method == http::Method::GET || method == http::Method::HEAD {
            self.read_permission.clone()
        } else {
            self.write_permission.clone()
        }
    }
    fn idempotency_policy(
        &self,
        method: &http::Method,
    ) -> crate::rest_framework::idempotency::IdempotencyPolicy {
        if method == http::Method::PUT
            || method == http::Method::PATCH
            || method == http::Method::DELETE
        {
            self.idempotency.clone()
        } else {
            Default::default()
        }
    }
    async fn get(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.core.retrieve(request, context, services).await
    }
    async fn put(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.core.update(request, context, services, false).await
    }
    async fn patch(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.core.update(request, context, services, true).await
    }
    async fn delete(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.core.destroy(request, context, services).await
    }
}

#[async_trait::async_trait]
impl<T> crate::rest_framework::views::APIView for CreateAPIView<T>
where
    T: RestModel,
{
    fn name(&self) -> &'static str {
        self.name
    }
    fn allowed_methods(&self) -> Vec<http::Method> {
        vec![http::Method::POST]
    }
    fn permission_policy(&self, _: &http::Method) -> PermissionExpr {
        self.permission.clone()
    }
    fn idempotency_policy(
        &self,
        _: &http::Method,
    ) -> crate::rest_framework::idempotency::IdempotencyPolicy {
        self.idempotency.clone()
    }
    async fn post(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        _: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.core.create(request, context).await
    }
}

#[async_trait::async_trait]
impl<T> crate::rest_framework::views::APIView for UpdateAPIView<T>
where
    T: RestModel,
{
    fn name(&self) -> &'static str {
        self.name
    }
    fn allowed_methods(&self) -> Vec<http::Method> {
        vec![http::Method::PUT, http::Method::PATCH]
    }
    fn permission_policy(&self, _: &http::Method) -> PermissionExpr {
        self.permission.clone()
    }
    fn idempotency_policy(
        &self,
        _: &http::Method,
    ) -> crate::rest_framework::idempotency::IdempotencyPolicy {
        self.idempotency.clone()
    }
    async fn put(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.core.update(request, context, services, false).await
    }
    async fn patch(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.core.update(request, context, services, true).await
    }
}

#[async_trait::async_trait]
impl<T> crate::rest_framework::views::APIView for DestroyAPIView<T>
where
    T: RestModel,
{
    fn name(&self) -> &'static str {
        self.name
    }
    fn allowed_methods(&self) -> Vec<http::Method> {
        vec![http::Method::DELETE]
    }
    fn permission_policy(&self, _: &http::Method) -> PermissionExpr {
        self.permission.clone()
    }
    fn idempotency_policy(
        &self,
        _: &http::Method,
    ) -> crate::rest_framework::idempotency::IdempotencyPolicy {
        self.idempotency.clone()
    }
    async fn delete(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.core.destroy(request, context, services).await
    }
}

#[async_trait::async_trait]
impl<T> crate::rest_framework::views::APIView for RetrieveDestroyAPIView<T>
where
    T: RestModel,
{
    fn name(&self) -> &'static str {
        self.name
    }
    fn allowed_methods(&self) -> Vec<http::Method> {
        vec![http::Method::GET, http::Method::DELETE]
    }
    fn permission_policy(&self, method: &http::Method) -> PermissionExpr {
        if method == http::Method::GET || method == http::Method::HEAD {
            self.read_permission.clone()
        } else {
            self.write_permission.clone()
        }
    }
    fn idempotency_policy(
        &self,
        method: &http::Method,
    ) -> crate::rest_framework::idempotency::IdempotencyPolicy {
        if method == http::Method::DELETE {
            self.idempotency.clone()
        } else {
            Default::default()
        }
    }
    async fn get(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.core.retrieve(request, context, services).await
    }
    async fn delete(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        self.core.destroy(request, context, services).await
    }
}
