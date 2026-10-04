# Views and URLs

## Protected view: the default

A normal view requires authentication without any extra policy code.

```rust
use async_trait::async_trait;
use http::{Method, StatusCode};
use lex_framework::views::{
    ApiRequest, ApiResponse, ApiResult, APIView, RequestContext, ViewServices,
};

pub struct ProfileView;

#[async_trait]
impl APIView for ProfileView {
    fn name(&self) -> &'static str { "profile" }
    fn allowed_methods(&self) -> Vec<Method> { vec![Method::GET] }

    async fn get(
        &self,
        _request: &ApiRequest,
        context: &RequestContext,
        _services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        let user_id = context.user_id();
        ApiResponse::json(StatusCode::OK, &serde_json::json!({"user_id": user_id}))
    }
}
```

## Public view

```rust
use lex_framework::permissions::{Permissions, ViewPermissions};

fn permissions(&self) -> ViewPermissions {
    ViewPermissions::new(Permissions::public())
}
```

Use this for health checks, login, registration, public catalogs, and landing endpoints.

## Public reads, protected writes

```rust
fn permissions(&self) -> ViewPermissions {
    ViewPermissions::public_read_authenticated_write()
}
```

## Per-method rules

```rust
fn permissions(&self) -> ViewPermissions {
    ViewPermissions::authenticated()
        .get(Permissions::public())
        .post(Permissions::named("articles.create"))
        .patch(Permissions::named("articles.edit"))
        .delete(Permissions::tenant_admin())
}
```

Unspecified methods use the secure default supplied to `ViewPermissions`.

## Request context shortcuts

```rust
context.is_authenticated();
context.user_id();
context.tenant_id();
context.has_scope("articles.edit");
context.request_id();
```

Advanced principal information remains available with `context.principal()`.

## URLs

Each application owns local routes in `src/urls.rs`. The CLI mounts them beneath the path in `lex_app.toml` and maintains the project route table through `lex refresh`.

```rust
pub fn routes() -> ApiResult<Vec<RouteSpec>> {
    Ok(vec![RouteSpec::http(
        "/",
        "products-index",
        Arc::new(ProductsIndexView),
    )])
}
```

If the app mount is `/api/v1/products`, the resulting URL is `/api/v1/products/`.
