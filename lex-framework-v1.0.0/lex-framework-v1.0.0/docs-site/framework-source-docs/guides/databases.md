# Database compatibility and consistency

Use PostgreSQL for database RLS and row locking. SQLite is useful for local development and applications whose concurrency and database-specific requirements fit its model. Backend-specific features fail explicitly where the SQLite executor detects unsupported SQL.

| Capability | PostgreSQL | SQLite | Contract |
|---|---|---|---|
| Basic QuerySet CRUD/filter/order/count | supported | supported | bound values; validated identifiers |
| Tenant predicates | supported | supported | tenant-aware models require explicit scope |
| Normalized RBAC constraints | supported | supported | reviewed RBAC installer; FK enforcement required |
| UUID storage | native UUID | canonical UUID text | generated row decoding matches the ORM's binding |
| JSON field data | JSON/JSONB | text JSON | JSONB operators are not generally portable |
| Decimal storage/retrieval | NUMERIC | exact text | values preserve precision; SQLite numeric SQL below is rejected |
| Decimal comparisons/order/arithmetic/aggregates | supported | rejected except count/null checks | SQLite TEXT/REAL cannot provide the exact NUMERIC contract |
| Decimal unique keys/relation keys/distinct projections | supported | rejected by ORM policy | use PostgreSQL or integer minor units |
| Transactions/savepoints | supported | supported | backend-specific options differ |
| RLS | supported | unavailable | QuerySet scoping alone on SQLite |
| FOR UPDATE/SHARE, NOWAIT, SKIP LOCKED | supported | rejected | SQLite has different database write locking |
| PostgreSQL array/JSONB aggregates | supported where exposed | rejected | no silent approximation |
| Read replicas | explicit PostgreSQL pools | unavailable | eventual consistency, not read-after-write guarantee |
| ModelViewSet database mutation adapter | PostgreSQL | no built-in SQLite adapter | custom public APIView can use SQLite QuerySet |
| Eager relation JSON hydration | supported | supported | decimal/JSON/boolean fidelity tested; binary columns require prefetch_related |
| CASE/window/annotation/subquery AST | no supported typed QuerySet API | no supported typed QuerySet API | do not infer support from SQL backend capability |
| Automatic schema alteration | reviewed migrations | reviewed migrations | auto-init only creates missing objects |

`Database::initialize_with` validates the URL/backend configuration. SQLite `:memory:` is reduced to one pooled connection to keep one logical database. Lex-managed pools require `database.sqlite_foreign_keys = true`; false is rejected before connecting. Enforcement is configured on every connection, not by a single PRAGMA on the pool. The file-backed test holds all four connections simultaneously and verifies that each rejects orphan rows. Use `SqliteConnectOptions::foreign_keys(true)` on every custom pool connection. The normalized installer refuses disabled enforcement and checks composite foreign-key shapes. File-backed SQLite has persistence and locking behavior that memory-only tests cannot fully represent.

ModelManager/QuerySet uses the pool passed to it; it does not secretly select replicas. `postgres_write_pool()` is the primary; `postgres_read_pool()` explicitly selects an available configured replica, round-robin, falling back to primary when none are available. Transactions and authorization should use primary. Asynchronous replicas can return stale rows after writes. Startup failures for replicas are fatal only when `require_all_read_replicas` is configured.

For PostgreSQL RLS use `TenantRlsPolicy`, enable/force it in migrations, and call `set_transaction_tenant` inside each transaction. The tenant GUC is `app.tenant_id`, set transaction-locally. Use a non-superuser, non-BYPASSRLS application role; database owners require FORCE RLS. Missing context matches no rows. Never set tenant session state on a pooled connection outside a transaction. The RLS integration test proves commit/rollback reset on the same reused connection.

Atomic SQLite bulk operations use SQLite JSON functions and UPDATE FROM. The standard SQLx SQLite build is exercised here; a custom externally supplied SQLite library must support those SQL capabilities. Missing capabilities return database errors and must not be treated as a successful partial batch.

SQLite decimal limitations are explicit InvalidQuery errors before SQL execution: comparisons (including equality/membership/ranges), decimal ordering, non-count aggregates, arithmetic updates, decimal uniqueness and relation keys, and distinct decimal projections. Exact values can still be inserted, assigned, copied and retrieved by nondecimal identifiers; null checks and counts are supported. Count/existence ignore ordering as usual. Raw SQL bypasses ORM policy and is the caller's responsibility. Automatic SQLite model initialization also rejects disabled foreign-key pools instead of enabling only one connection.
