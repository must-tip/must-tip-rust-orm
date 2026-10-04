> **Historical documentation.** Check the current quickstart and testing matrix before using these examples.

# Recipes

## Public registration endpoint

```rust
fn permissions(&self) -> ViewPermissions {
    ViewPermissions::new(Permissions::public())
}
```

After creating the user, your application can issue a framework session or bearer token.

## Public product list, admin-only mutation

```rust
fn permissions(&self) -> ViewPermissions {
    ViewPermissions::authenticated()
        .get(Permissions::public())
        .post(Permissions::tenant_admin())
        .patch(Permissions::tenant_admin())
        .delete(Permissions::tenant_admin())
}
```

## Named permissions using session scopes

When login succeeds:

```rust
let session = auth_service.create_session(
    user.id,
    ["orders.read".into(), "orders.create".into()],
).await?;
```

View:

```rust
fn permissions(&self) -> ViewPermissions {
    ViewPermissions::authenticated()
        .get(Permissions::named("orders.read"))
        .post(Permissions::named("orders.create"))
}
```

## Owner-only update

Declare authentication at view level, load the object, then enforce ownership with object metadata:

```rust
services.require_object(
    context,
    Permissions::owner(),
    &PermissionResource {
        resource_type: "profile".into(),
        resource_id: Some(profile.id.to_string()),
        tenant_id: profile.tenant_id,
        owner_user_id: Some(profile.user_id),
    },
    false,
).await?;
```

## Hybrid web + mobile application

```toml
[security.authentication]
backends = ["session", "bearer"]

[security.permissions]
backend = "scopes"
```

The same views work for browser sessions and mobile/API bearer tokens because both produce the same framework `Principal` and permission scopes.
