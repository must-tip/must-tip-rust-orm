# Models and queries

The `Model` derive implements `Entity`, model metadata, validation, typed PostgreSQL/SQLite row decoding, and manager constructors. It does not create a running API or evolve existing database tables. Start with the executable models in `integration/consumer/src/lib.rs`; that application depends only on the facade and ordinary ecosystem crates.

```rust
use lex_framework::prelude::*;
use lex_framework::db_controller::{TextField, UUIDField};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "articles")]
pub struct Article {
    #[field(primary_key)]
    pub id: UUIDField,
    pub tenant_id: UUIDField,
    #[field(not_empty, max_length = 200)]
    pub title: TextField,
}
```

Prefer an explicit primary key on the Rust struct. Schema generation can supply an implicit database `id`, but the derive cannot inject a readable Rust struct field. Use explicit IDs for identity-dependent APIs. `UUIDField::new()` generates an ID; constructors require every Rust field even when metadata declares a database default. SQL defaults apply when database inserts omit that column, not when a Rust field is supplied.

`Option<T>` represents nullable persisted values. Qualified field paths are supported, including nullable `DecimalField`. Exact decimal values are stored as text on SQLite and NUMERIC on PostgreSQL; use decimal strings, not floating-point intermediates. Composite uniqueness uses `#[model(unique(name = "article_tenant_title", fields = "tenant_id,title"))]`. Names and fields are checked at derive time. Two primary keys, nullable primary keys and unknown options produce compiler diagnostics.

For PostgreSQL, use `Article::objects(pool.clone())`. For SQLite, use `Article::objects_sqlite(pool.clone())`; `objects_sqlite_auto(pool).await?` can bootstrap missing tables for development. Register metadata explicitly with `Article::register()` when generating migrations for an application. Auto bootstrap leaves existing tables unchanged and is not a schema synchronization tool.

QuerySet builders describe a query; terminal async methods perform IO. Chained `filter` predicates are ANDed. Use a predicate's `and`/`or` composition for grouped conditions. `exclude` negates its predicate. Values are bound parameters; SQL-looking text and Unicode are legitimate data. Identifiers must exist in model metadata and pass identifier validation.

```rust,ignore
let rows = manager.queryset()
    .for_tenant(authorized_tenant_id)
    .filter(Article::TITLE.contains("release"))
    .order_by(lex_framework::db_controller::OrderBy::asc("title"))
    .page(1, 20)?
    .all().await?;
```

The tenant ID must come from an authorized request context. A model with `tenant_id` is tenant-aware unless explicitly marked `#[model(global)]`; missing scope fails before database execution. Privileged global scope requires a `GlobalScopeAuthorization` implementation and separate authorization for related tenant models. Marking a model global is a security decision, not a workaround for a missing resolver.

| Terminal | Result |
|---|---|
| all | matching typed rows with ordering/pagination |
| get | one row; NotFound for zero, MultipleObjectsReturned for more than one |
| first / last | optional row using deterministic effective ordering |
| count / exists | matching cardinality/existence; limit/offset do not narrow these terminals |
| values | JSON objects for selected columns |
| values_list | JSON arrays, or scalar values for one field with flat=true |
| aggregate | one aggregate result; backend-specific functions can be unsupported |
| create / bulk_create | persisted returned models |
| update / bulk_update | affected row count |
| soft_delete / restore / hard_delete | explicit deletion mode and affected count |

Pages are one-based; page sizes must be 1–10,000. Explicit ordering is necessary when an application needs a stable pagination order. `distinct().values(...)` and `values_list(...)` reject ordering; call `clear_ordering()` and order the resulting values separately. Query errors identify invalid projections rather than silently changing semantics.

Bulk create/update execute as one atomic statement on both backends. A constraint error leaves the whole batch unchanged. `ignore_conflicts` skips uniqueness conflicts; it is not permission to ignore all validation or foreign-key errors. A batch is capped at 5,000 rows. Bulk updates are deliberate set operations; use `Tracked` for stale-editor conflict detection.

Ordinary queries hide soft-deleted records. `with_deleted` and `only_deleted` change visibility without removing tenant boundaries. `restore` does not resolve unique-key conflicts automatically: database unique constraints still include soft-deleted rows unless an application defines a reviewed alternative index.

Relationships are metadata, not implicit database calls. Ordinary model loads leave singular relations unloaded; `select_related` explicitly joins and `prefetch_related` explicitly fetches related records. Both preserve tenant scope and hide deleted related rows. A singular relation requires a unique target key, possibly unique within the selected tenant. Use composite foreign keys in application migrations when a relationship must enforce matching tenants at the database level.

The portable query, relation and transaction stories are executable in `integration/consumer/tests`. Read [database differences](../guides/databases.md), [transactions](transactions.md), [migrations](migrations.md) and [security](../guides/security.md) before deploying.

SQLite exact-decimal fields support storage and retrieval but reject numeric comparisons/order/arithmetic, non-count aggregation, unique/relation keys and distinct projections. These return InvalidQuery instead of lexical ordering or rounded REAL values. Use PostgreSQL for exact numeric SQL, or an integer representation appropriate to the application. See the backend compatibility guide.
