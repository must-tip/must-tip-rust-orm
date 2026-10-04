> **Historical documentation.** These pre-audit notes may describe obsolete APIs or ownership. Use the [current developer documentation](../README.md) and executable acceptance examples for the supported contract.

# Permissions

Lex separates authentication (who is calling) from authorization (what the caller may do).

## Zero-setup default: scopes

```toml
[security.permissions]
backend = "scopes"
global_admin_scope = "admin:global"
tenant_member_scope = "tenant:member"
tenant_admin_scope = "tenant:admin"
```

With this backend, named permission rules map directly to scopes issued with sessions or bearer tokens.

```rust
Permissions::named("articles.edit")
```

matches a credential containing scope `articles.edit`.

## Available rules

```rust
Permissions::public();
Permissions::authenticated();
Permissions::anonymous_only();
Permissions::global_admin();
Permissions::tenant_member();
Permissions::tenant_admin();
Permissions::owner();
Permissions::owner_or_read_only();
Permissions::named("invoice.refund");
Permissions::all(["invoice.read", "invoice.refund"]);
Permissions::any(["support.agent", "invoice.refund"]);
Permissions::and([Permissions::tenant_member(), Permissions::named("invoice.read")]);
Permissions::or([Permissions::tenant_admin(), Permissions::owner()]);
Permissions::not(Permissions::named("account.suspended"));
```

## Per-method permission example

```rust
fn permissions(&self) -> ViewPermissions {
    ViewPermissions::authenticated()
        .get(Permissions::named("orders.read"))
        .post(Permissions::named("orders.create"))
        .patch(Permissions::named("orders.edit"))
        .delete(Permissions::tenant_admin())
}
```

## Require a permission after loading data

Sometimes authorization requires object metadata that is not available before the handler runs.

```rust
services.require(context, "invoice.refund").await?;
```

For owner/tenant-aware checks:

```rust
let resource = PermissionResource {
    resource_type: "invoice".into(),
    resource_id: Some(invoice.id.to_string()),
    tenant_id: Some(invoice.tenant_id),
    owner_user_id: Some(invoice.owner_user_id),
};

services
    .require_object(
        context,
        Permissions::owner_or_read_only(),
        &resource,
        true,
    )
    .await?;
```

## PostgreSQL database-role permissions

For projects that use the existing persisted tenant membership/role/permission models:

```toml
[security.permissions]
backend = "database"
```

This mode currently requires PostgreSQL. It uses the existing database permission backend, tenant membership records, tenant roles, and permission definitions.

## Choosing a backend

Use `scopes` when:

- you want the easiest setup;
- session/token issuance already knows the caller's capabilities;
- you use SQLite for development;
- services are scope-oriented.

Use `database` when:

- role assignments must change without reissuing credentials;
- permissions are centrally stored in PostgreSQL;
- tenant membership/role tables are authoritative.
