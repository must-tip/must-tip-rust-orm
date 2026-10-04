# Architecture

## Scope

This workspace is a set of Rust libraries used by services; it is not itself a collection of HTTP microservices. The design keeps persistence, domain models, migrations, input security, and transport policy separate so a service can compose them without duplicate database logic.

## Dependency direction

```text
orm ---------------------> generated code references db_controller
models -------> orm + db_controller + sqlx
migration ----> models + db_controller + sqlx
db -----------> migration + sqlx
secure_input -> validation/sanitization dependencies
media --------> integration boundary
```

`db_controller` is the canonical persistence layer. `models` owns domain models. `migration` consumes `Entity` metadata instead of maintaining a second schema definition. `db` owns process-level connection policy. The REST module consumes `Entity`/QuerySet contracts but owns no HTTP server.

## Entity and metadata model

`Entity` is the source of truth for table, primary-key, columns, typed indexes, relationships, unique/check constraints, default ordering, tenant column, soft-delete column, version column, and validation.

`#[derive(Model)]` generates:

- persisted `ColumnSpec` metadata and typed field-index metadata;
- typed `Field<T>` descriptors for model fields;
- `RelationSpec` metadata and relation descriptors;
- SQLx `FromRow<PgRow>` decoding for persisted fields;
- `to_map()` for validated persistence;
- declared model validation;
- `Model::objects(pool)`, `full_clean()`, and `validate()` convenience methods.

Relation projections are not persisted columns and do not trigger hidden database access during row decoding.

## Query architecture

`QuerySet<T, E>` is the primary query abstraction. It stores a query plan consisting of typed predicates, ordering, visibility, tenant scope, pagination, row-lock policy, timeouts, and destructive-operation scope. SQL is generated only at the execution boundary.

Values are represented by `DbValue`; every bound value is checked against the model's SQL metadata before it is bound. Dynamic REST lookups parse request strings through the target column metadata. Identifiers are never taken directly from an unvalidated request.

The executor abstraction supports primary pooled, read-only pooled, and transaction-bound execution. `ReadOnlyPoolExecutor` rejects mutation terminals before SQLx; database permissions/physical replica roles remain the authoritative write barrier. Row locks are rejected for pooled executors because PostgreSQL row locks must be held by a transaction whose lifetime the caller controls.


## Query expressions and projections

The query AST supports typed scalar predicates, Boolean composition, field-to-field comparisons, atomic field copy/arithmetic updates, values/value-list projections, filtered/distinct aggregate annotations, bound-value `CASE/WHEN` projections, and deterministic `ROW_NUMBER`/`RANK`/`DENSE_RANK` windows with validated partition/order fields. These operations do not expose arbitrary SQL fragments.

Composite `get_or_create_by_fields` and transaction-only `update_or_create_by_fields` accept only primary/unique fields or an exact declared `UniqueConstraintSpec`, so application filters cannot be promoted accidentally into database conflict targets.

## Tenant isolation

Tenant-aware models expose a tenant column in `Entity` metadata. QuerySet reads and writes fail with `TenantScopeRequired` unless they carry `for_tenant(...)` or an explicitly authorized global scope.

`global_scope` accepts a `GlobalScopeAuthorization` implementation supplied by the application authorization layer. The ORM never decides that a caller is privileged by itself.

For sensitive workloads, `TenantRlsPolicy` generates PostgreSQL RLS policy SQL and `AtomicTransaction::set_tenant()` sets a transaction-local `app.tenant_id` GUC. Application QuerySet scoping and database RLS are independent controls.

## Model state and concurrency

`Tracked<T>` retains the originally loaded field map. Its `save()` persists only dirty mutable fields, excludes primary key/framework/version fields, opens one transaction, applies tenant scope, performs an optimistic version check when declared, reloads the row in the same transaction, then commits.

Explicit versioned QuerySet updates return `ConcurrentModification` when the expected version no longer matches.

## Relationships

Relations are metadata-driven. `select_related<R>` is for singular relationships and executes a validated join. `prefetch_related<R>` loads parent rows and related rows in batches, then builds an in-memory parent-to-related mapping. `filter_related<R>` and `exclude_related<R>` compile correlated `EXISTS`/`NOT EXISTS` predicates using the related model's own tenant and soft-delete policies. The ORM does not perform lazy N+1 queries implicitly.

The original public `ManyToMany<T>` compatibility wrapper is preserved. It stores legacy integer identifiers and is not changed to a new key representation without an approved breaking change; use relation metadata/`Prefetched<T, R>` for hydrated related models.

## Transactions

`AtomicTransaction` configures isolation/read-only/deferrable semantics and transaction-local statement/lock/idle timeouts. It supports QuerySets, savepoints, tenant context, commit/rollback, and post-commit callbacks. `retry_transaction` retries only retry-safe PostgreSQL serialization/deadlock errors according to a bounded policy.


## Read routing

`db::Database` owns one primary pool and zero or more environment-configured read-replica pools. `write_pool()`/legacy `pool()` always return primary; `read_pool()` round-robins initialized replicas and falls back to primary when policy permits. Callers must select a primary executor for writes, locks, transactions, migrations, and consistency-sensitive read-after-write operations. The library does not infer replication lag or business consistency requirements.

## REST boundary

The REST module intentionally stops before routing/authentication. A service adapter converts framework request parameters/body into `RestQueryPolicy` and `SerializerPolicy`, then maps `RestError`/`ModelError` to that service's HTTP/gRPC error contract. This prevents database behavior from depending on Axum, Actix, tonic, or another transport.

## Migration model

Two migration mechanisms coexist intentionally:

1. Existing immutable paired SQL migrations (`<name>_up.sql`/`<name>_down.sql`) remain supported for compatibility and controlled production operations.
2. Typed `MigrationOperation` plus `MigrationGraph` provides deterministic forward/reverse SQL and dependency validation for new migration tooling; destructive operations require complete companion rollback metadata when dependent objects exist.

The graph rejects duplicate IDs, missing dependencies, self-dependencies, and cycles before rendering SQL. `operations_for_model<T>()` derives table and singular FK creation operations from entity metadata and validates `SET NULL`/`SET DEFAULT` preconditions.

The typed operation model is not yet a complete Django migration autodetector; see `GOVERNANCE.md`.

## Media boundary

Production media storage, antivirus scanning, metadata stripping, thumbnailing, and transcoding contracts were not present in the supplied project. `media` therefore remains an explicit boundary rather than claiming unimplemented production behavior.

## Cross-tenant relation boundary

Privileged global scope is granted per model. `select_related` and `prefetch_related` enforce the related model's tenant and soft-delete policy independently; callers must explicitly authorize global access for each related model. Joined row locks target only the root table alias to avoid accidentally locking nullable outer-join rows.

Singular relation loading validates uniqueness for the effective query scope. A tenant-scoped one-to-one may use a composite unique constraint containing the related tenant column and relation key; globally scoped relation loading requires the remote key to be globally unique. Composite model unique constraints are emitted into direct model DDL.

## REST framework boundary

`crate/lex_rest` sits above `db_controller` and `models`. It owns transport/application concerns only: request/response, authentication interfaces, permission evaluation, API views/viewsets, routing, throttling/idempotency contracts, middleware, audit, OpenAPI metadata, and WebSocket/WSS policy. It reuses `db_controller::RestAccessPolicy`, serializers, filtering, pagination and QuerySet execution rather than creating a second persistence implementation. See `docs/LEX_REST.md`.
