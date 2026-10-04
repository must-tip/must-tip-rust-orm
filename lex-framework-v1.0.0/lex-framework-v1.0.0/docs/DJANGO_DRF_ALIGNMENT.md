> **Historical documentation.** These pre-audit notes may describe obsolete APIs or ownership. Use the [current developer documentation](README.md) and executable acceptance examples for the supported contract.

# Django ORM and DRF Alignment

## Goal

The workspace adopts Django ORM and Django REST Framework concepts where they improve correctness and developer ergonomics, while keeping Rust ownership, static typing, explicit async execution, PostgreSQL semantics, and service authorization boundaries explicit.

## Django ORM mapping

| Django concept | Rust database-system equivalent |
|---|---|
| `Model.objects` | generated `Model::objects(PgPool)` returning `ModelManager<Model>` |
| `QuerySet` | `QuerySet<T, E>` over a typed query plan and executor |
| `filter` / `exclude` / `Q` | `Predicate`, typed `Field<T>`, AND/OR/NOT composition |
| field lookups | typed field methods plus validated `dynamic_predicate()` |
| relation traversal | typed `filter_related::<R>()` / `exclude_related::<R>()` using correlated `EXISTS`/`NOT EXISTS` |
| `get`, `first`, `last`, `earliest`, `latest`, `exists`, `count` | QuerySet terminal methods |
| `values` / `values_list` | typed/validated projection terminals |
| aggregation / annotation | aggregate projections, filtered/distinct aggregates, `CASE/WHEN`, deterministic row-number/rank windows |
| `F()` | field-to-field comparison and atomic `UpdateExpression` copy/arithmetic operations |
| `select_related` | singular metadata-driven join hydration |
| `prefetch_related` | batched related-query hydration through `Prefetched<T, R>` |
| `get_or_create` | constraint-validated `get_or_create_by` / composite `get_or_create_by_fields` |
| `update_or_create` | transaction-only single/composite unique-key update-or-create |
| `bulk_create` / `bulk_update` | bounded validated bulk persistence, tenant and version aware |
| `save(update_fields=...)` | `Tracked<T>` dirty tracking + transaction-scoped `save()` |
| `refresh_from_db` | scoped refresh on QuerySet/Tracked state |
| optimistic locking | model `version_column` + versioned QuerySet/Tracked writes |
| soft-delete manager behavior | hidden-by-default deleted rows with explicit visibility modes |
| `transaction.atomic` | `AtomicTransaction` |
| nested atomic/savepoint | savepoint stack with callback rollback checkpoints |
| `transaction.on_commit` | post-commit callbacks discarded with rolled-back savepoints |
| `select_for_update` | transaction-only lock modes with wait/no-wait/skip-locked |
| multi-database read use | explicit primary/read-replica pools plus `ReadOnlyPoolExecutor` |
| migration dependencies | `MigrationGraph` |
| migration operations | typed reversible `MigrationOperation` plus immutable SQL migration compatibility |

### Deliberate differences

- Query execution is explicit async Rust; relation properties never perform hidden database I/O.
- Tenant scoping is fail-closed and first-class rather than relying only on manager conventions.
- Global tenant bypass requires application authorization, including independent authorization for related models.
- Generic mutation paths cannot rewrite tenant ownership, primary keys, soft-delete markers, timestamps, or optimistic-version fields.
- PostgreSQL RLS support remains an independent database control rather than being replaced by ORM filters.
- Partial serializers produce validated patches instead of partially initialized Rust models.
- Sensitive/write-only serializer input must be transformed by the owning domain service before persistence.
- Read-replica selection is explicit. The ORM does not guess consistency requirements or route writes to replicas.
- SQL generated at runtime crosses SQLx 0.9's `AssertSqlSafe` boundary only inside reviewed compiler/migration modules after identifiers and values have been validated/parameterized.

## DRF mapping

| DRF concept | Rust database-system equivalent |
|---|---|
| serializer field policy | `SerializerPolicy` / `SerializerFieldPolicy` |
| create/update/partial validation | `SerializerMode` and model-derived validation |
| `read_only` / `write_only` | serializer policy flags with framework/sensitive defaults |
| permission/query scoping | application-owned `RestAccessPolicy<T>` |
| filter backend allowlist | `RestQueryPolicy::allow_filter()` |
| search filter | `allow_search()` |
| ordering filter | `allow_ordering()` |
| page-number pagination | `PaginationStyle::PageNumber` |
| limit/offset pagination | `PaginationStyle::LimitOffset` |
| cursor pagination | signed deterministic keyset cursor policy |
| serializer error object | `RestError` with stable code/detail/field errors |

## HTTP/service integration boundary

The database workspace intentionally does not create an Axum/Actix/tonic application, authentication protocol, rate limiter, content-negotiation layer, OpenAPI generator, or business authorization model because the supplied project establishes no single authoritative transport/auth contract.

A production service adapter should:

1. authenticate the caller using the service's authoritative authentication component;
2. resolve tenant context and create an application `RestAccessPolicy`/global-scope capability from authoritative authorization;
3. validate create/update bodies with `SerializerPolicy` and apply required domain transforms to secrets;
4. apply `RestQueryPolicy` to an already authorization-scoped QuerySet;
5. use primary/transaction executors for writes, locks, and read-after-write semantics; use `ReadOnlyPoolExecutor` only for reads allowed to observe replica lag;
6. execute atomic workflows through `AtomicTransaction` and use a durable outbox for cross-service publication;
7. map stable ORM/REST error codes to the public service error contract;
8. emit metrics/tracing without raw SQL values, secrets, or unrestricted request content.

## Remaining deliberate gaps

The archive does not claim the following are complete:

- arbitrary user-defined SQL expressions or unrestricted raw SQL QuerySet escape hatches;
- a general arbitrary correlated-subquery API, recursive/deep relation path compiler, or every PostgreSQL function/window frame;
- implicit Django-style many-to-many manager mutation (`add/remove/set/clear`) for arbitrary key types; the supplied project uses explicit bridge models for authoritative many-to-many persistence;
- a complete Django migration autodetector/state optimizer/squashing engine;
- automatic topology-aware primary/replica/shard/data-residency routing and replica-lag consistency decisions;
- database schema reverse engineering (`inspectdb` equivalent);
- streaming/COPY ingestion APIs;
- framework-specific viewsets/routers/authentication/throttling/OpenAPI;
- production media storage/scanning/transcoding, because no authoritative provider contracts were supplied.

These are documented gaps, not silently simulated features.
