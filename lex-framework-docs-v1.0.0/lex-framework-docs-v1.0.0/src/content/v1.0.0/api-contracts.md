# API and Data Contracts

## `db_controller::Entity`

`Entity` is the canonical model contract. Implementations expose table/schema information, model metadata, relationships/constraints, serialization for persistence, and validation. `effective_schema::<T>()` completes framework columns only when the model does not explicitly declare them.

Tenant, soft-delete, and optimistic-version behavior are opt-in through entity metadata and are then enforced by QuerySet.

## QuerySet

Create a manager from the existing API or generated model method, then derive a QuerySet:

```rust
let users = User::objects(pool.clone())
    .queryset()
    .for_tenant(tenant_id)
    .filter(User::EMAIL.icontains("@example.com"))
    .order_by(db_controller::OrderBy::desc("created_at"))
    .limit(50)
    .all()
    .await?;
```

Implemented operations include filtering/exclusion, ordering, pagination, distinct, deleted-row visibility, tenant/global scope, timeouts, row-lock policy, CRUD/bulk operations, optimistic updates, typed field copy/arithmetic expressions, soft-delete lifecycle, values/value-list projection, filtered/distinct aggregate annotations, CASE projections, deterministic ranking windows, explain, singular relation join loading, batched prefetch, and typed one-hop related EXISTS/NOT EXISTS filtering.

`get()` enforces exactly one visible row. `get_or_create_by_fields` and transaction-only `update_or_create_by_fields` accept only an actual primary/unique field or an exact declared composite `UniqueConstraintSpec`. Destructive bulk operations require a user predicate or explicit `allow_all_mutation()`.

`global_scope(&authorization)` requires an application-defined `GlobalScopeAuthorization`; the ORM does not contain a hard-coded administrative bypass.

## Legacy repository

`Repository<T>` remains for compatibility for global models. Tenant-aware models return `TenantScopeRequired` through that API because its original method signatures cannot carry tenant authorization. Migrate tenant-owned service code to `ModelManager<T>::queryset().for_tenant(...)`.

## Transactions

Use `AtomicTransaction` when operations must share one PostgreSQL transaction or need locks/savepoints/post-commit behavior. QuerySets built from `AtomicTransaction::queryset()` use the same SQLx transaction. `select_for_update` on a pooled QuerySet is rejected.

`retry_transaction` retries only PostgreSQL serialization/deadlock failures and only within configured retry bounds.


## Read replica contract

`db::Database::write_pool()` and the compatibility `pool()` are always primary. `read_pool()` returns an initialized replica using round-robin selection or primary fallback when no replica is available and configuration permits fallback. Construct `ReadOnlyPoolExecutor` for ORM reads that are allowed to observe replica lag; it rejects mutation execution paths. Database role permissions and physical replica configuration remain the authoritative write protection.

Do not use a replica for locking queries, transactions, migrations, read-after-write consistency, or business commands whose correctness depends on the latest primary state.

## Serializer contract

`SerializerPolicy::for_model::<T>()` derives field policy from model metadata:

- primary keys are read-only by default;
- encrypted fields are write-only by default;
- non-null/no-default fields are required on create;
- relation projections are read-only;
- unknown/read-only/null/type-invalid input is rejected;
- model validation runs before a fully constructed model is accepted.

Partial update returns a validated field map instead of constructing an invalid partial Rust struct. Service code applies that map through QuerySet/Tracked persistence.

Generic model deserialization never hydrates write-only/encrypted request values directly into an entity. Sensitive values (for example passwords, API keys, or application-encrypted secrets) must first pass service-owned hashing/encryption/KMS transformation and then be placed into the domain model. This prevents transport-layer plaintext from being persisted merely because it passed shape validation.


## REST authorization contract

`RestAccessPolicy<T>` is application supplied. It can add a mandatory list/retrieve scope predicate and authorize create/update/delete payloads or instances. It does not authenticate callers and must be constructed only after the owning service has resolved authoritative identity, tenant, and permissions. `RestQueryPolicy::apply_authorized` applies the access scope before client filter/search/order parameters.

## Filter/search/order contract

`RestQueryPolicy` is deny-by-default. A service explicitly allowlists fields and lookups. It validates model policy at application time, rejects encrypted fields from filter/search/order, bounds request parameter count/value length/`IN` cardinality, and can reject unknown parameters.

Supported lookup policy values cover exact/not-equal, case-insensitive exact, comparisons, null, `IN`, range, and text lookups where the field type permits them.

## Pagination contract

Supported strategies:

- page-number;
- limit/offset;
- signed keyset cursor.

Cursor mode requires an HMAC-SHA256 key of at least 32 bytes, enforces token size/version/signature, supports a small bounded set of previous verification keys for rotation, and requires non-null ordering fields. A primary-key tie-breaker is included for stable ordering.

Cursor signing keys must come from the service's secret manager/environment integration; they are not persisted by this library.

## Migration contract

Existing paired SQL migrations remain immutable after application. A SHA-256 digest covers both directions and drift is rejected.

Typed migrations use `MigrationOperation` and `MigrationGraph`. Operations render explicit forward and reverse SQL, including `RunSql { up, down }`, and the graph validates dependencies and renders UP in topological order and DOWN in reverse order. Model-generated operations include typed model indexes, model-level composite unique constraints, conservative CHECK constraints, and singular foreign keys so typed migrations and direct model metadata do not silently diverge.

Destructive operations such as dropping a table or column can have dependent database objects outside the encoded operation. Those migrations require reviewed companion operations or explicit paired SQL that restores every dependent index/constraint/object; the existence of `down_sql()` alone is not evidence of lossless rollback.

## Configuration contract

Required secret:

- `DATABASE_URL`

Non-secret overrides use `APP__` nesting, such as `APP__DATABASE__MAX_CONNECTIONS`, `APP__DATABASE__CONNECT_TIMEOUT_SECS`, `APP__DATABASE__RUN_MIGRATIONS`, `APP__SERVER__HOST`, and `APP__SERVER__PORT`.

`APP_PROFILE=production` disables `.env` loading.

## Stable error contract

`ModelError::code()` and `RestError.code` provide stable machine-readable error categories. Do not expose raw SQL/database diagnostics to external clients; map them to the service's public error format at the transport boundary.

## Compatibility note

`User.role` uses the existing strongly typed `UserRole` enum and serializes lowercase values. The earlier runtime-state `ChoiceField<String>` representation is not a valid persisted-field abstraction and is not restored.

## lex_rest API contract

`lex_rest` exposes framework-neutral `ApiRequest`, `ApiResponse`, immutable `Principal`/`RequestContext`, `AuthenticationBackend`, `PermissionBackend`, `APIView`, `GenericAPIView<T>`, CRUD generic views, `ModelViewSet<T>`, `RouteTable`, distributed throttle/idempotency store contracts, and `WebSocketView`/`WebSocketSession`. HTTP adapters are optional features; persistence APIs remain owned by `db_controller`. See `docs/LEX_REST.md`.
