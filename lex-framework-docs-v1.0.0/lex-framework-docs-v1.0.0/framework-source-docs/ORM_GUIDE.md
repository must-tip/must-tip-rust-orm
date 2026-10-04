# ORM Guide

The ORM is centered on `Entity`, the `#[derive(Model)]` macro, typed `Field<T>` descriptors, `ModelManager<T>`, and `QuerySet<T, E>`.

## Recommended imports

```rust
use db_controller::prelude::*;
use orm::Model;
```

## Model definition

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "projects")]
#[timestamps]
#[model(ordering = "-created_at")]
struct Project {
    #[field(primary_key)]
    id: UUIDField,

    #[field(not_empty, unique, max_length = 120)]
    name: TextField,

    #[field(default = false)]
    archived: BooleanField,
}
```

The macro exposes field descriptors such as `Project::ID`, `Project::NAME`, and `Project::ARCHIVED`.

## Query construction

```rust
let qs = Project::objects(pool.clone())
    .queryset()
    .filter(Project::ARCHIVED.eq(false))
    .filter(Project::NAME.icontains("platform"))
    .order_by(Project::NAME.asc())
    .limit(50);
```

QuerySets are plans; use a terminal to execute.

## Terminals

```text
all()
first()
last()
earliest(field)
latest(field)
get()
exists()
count()
values(fields)
values_list(...)
aggregate(...)
explain(analyze)
```

## Mutation safety

Generic UPDATE/DELETE requires a caller predicate. An intentional whole-table mutation must explicitly call `allow_all_mutation()`.

Tenant ownership, primary keys, soft-delete markers, timestamps, and optimistic version columns are protected in generic mutation paths.

## Soft delete

Deleted rows are hidden by default when a model declares a soft-delete column.

- `with_deleted()` includes them.
- `only_deleted()` returns only them.
- `soft_delete()` marks rows deleted.
- `restore()` restores.
- `hard_delete()` physically deletes.

## Tenant scope

For models declared with `#[model(tenant = "tenant_id")]`:

```rust
let rows = TenantModel::objects(pool.clone())
    .queryset()
    .for_tenant(tenant_id)
    .all()
    .await?;
```

Unscoped tenant access fails closed.

## Transactions and locks

Create transaction-local QuerySets with `AtomicTransaction::queryset::<T>()`. `select_for_update` is rejected outside that context.

## Relation loading

Use `select_related` for singular relations and `prefetch_related` for batched many relations. No relation property performs implicit database I/O.

## Raw/dynamic query caution

Dynamic identifiers are validated and quoted before SQL construction. Runtime values are bound. Do not reintroduce raw string interpolation in service code.

For full behavior, see `docs/DEVELOPER_GUIDE.md` and `docs/API_CONTRACTS.md`.
