# Operations, Deployment, and Rollback

## Build gate

```bash
rustup show
cargo generate-lockfile
lex verify framework
```

Review and commit `Cargo.lock`. Production builds must use `--locked`.

## PostgreSQL integration gate

Use only a disposable database matching the production PostgreSQL major version:

```bash
export TEST_DATABASE_URL='postgres://.../orm_disposable'
export CONFIRM_DISPOSABLE_DATABASE=yes
lex verify postgres
```

The integration suite intentionally drops/recreates its tables and exercises QuerySet CRUD, tenant isolation, related filtering, relation loading, text lookup, expression/bulk updates, optimistic concurrency, unique-key upserts, transaction row locks, nested savepoint/on-commit behavior, tracked save, projection behavior, and soft-delete visibility.

## Database startup

Inject `DATABASE_URL` through the deployment secret mechanism. Keep `database.run_migrations = false` on ordinary replicas. Run migrations as a controlled deployment job or exactly one explicitly designated instance.

The initializer validates pool configuration, retries failed initial connections with bounded delay, verifies connectivity with `SELECT 1`, and closes failed initialization resources.


## Read-replica deployment

Set `DATABASE_READ_REPLICA_URLS_JSON` to a JSON array of PostgreSQL URLs supplied by the deployment secret mechanism. Replica URLs are redacted from `DatabaseSettings` debug output. Configure `APP__DATABASE__REQUIRE_ALL_READ_REPLICAS=true` only when process startup must fail if any declared replica cannot initialize.

Use primary for all writes, migrations, row locks, transactions, read-after-write workflows, security decisions requiring current state, and workflows that cannot tolerate replica lag. Use `ReadOnlyPoolExecutor` for explicitly stale-tolerant QuerySets. Provision the replica database role as read-only independently of the Rust guard.

Health checks validate configured pools, but the library does not infer replication lag, promote replicas, or perform topology failover. Those decisions belong to the database platform and service consistency policy.

## Tenant/RLS deployment

For tenant-owned tables:

1. verify `Entity::tenant_column()` metadata;
2. add reviewed RLS migration SQL produced from `TenantRlsPolicy`;
3. run the application as a least-privilege PostgreSQL role that cannot bypass RLS;
4. set the transaction tenant before sensitive operations;
5. still use QuerySet `.for_tenant(tenant_id)`; do not rely on only one control;
6. integration-test cross-tenant denial and privileged global workflows separately.

Do not use a database superuser for routine application traffic because superusers can bypass intended RLS controls.

## Migration release procedure

1. Snapshot/backup according to the database provider runbook.
2. Review every UP/DOWN SQL pair or rendered typed migration operation.
3. Validate migration dependency graph before execution.
4. Run pending/dry-run and SQL review against staging.
5. Apply to staging and run application plus PostgreSQL integration suites.
6. Measure lock duration/table rewrite impact for large tables.
7. Apply immutable migration artifacts to production through one controlled job.
8. Confirm `_migrations`, application health, latency/error metrics, and tenant/RLS checks.
9. Never edit an applied migration; create a new migration.

The existing SQL migration engine serializes writers with a PostgreSQL advisory lock and rejects checksum drift.

## Transaction operational rules

- Keep transactions short; never perform remote network calls while holding database row locks unless the workflow explicitly requires and bounds them.
- Use row locking for pessimistic concurrency and a version column for optimistic concurrency; choose intentionally per workflow.
- Configure `statement_timeout`, `lock_timeout`, and idle-in-transaction timeout for high-risk operations.
- Retry only operations proven idempotent/replay-safe; the library's transaction retry helper limits retryable SQLSTATEs but cannot prove application side effects are safe.
- Use `on_commit` for effects that must not start before commit; durable cross-service publication should use the service's transactional outbox pattern rather than an in-memory callback. If an `on_commit` callback returns `post_commit_failure`, the database transaction has already committed: do not retry the database command as if it rolled back. Retry or compensate the external effect independently.

## Cursor-key rotation

1. Provision a new independent cursor HMAC secret through the secret manager.
2. Configure it as active and retain the prior key only in the bounded previous-key verification set.
3. Deploy all replicas.
4. After the maximum cursor lifetime/rollout window, remove the prior key.
5. Never log key bytes or cursor decoded payloads containing sensitive business identifiers.

## Rollback

Application rollback and schema rollback are separate decisions. Prefer application rollback without schema rollback when schema compatibility permits. Before a DOWN migration, review data-loss/locking implications and backup state. `reset` is destructive and requires explicit CLI confirmation.

For RLS rollback, removing a policy reduces isolation. Treat that as a security-sensitive change requiring explicit approval; prefer correcting the policy while keeping protection enabled.

## Failure handling

- TLS failure: correct certificates/trust/DNS/credentials; do not default to disabling verification.
- Migration drift: restore exact applied files from source control; never rewrite `_migrations` manually.
- Partial migration: the current transaction rolls back; inspect the cause and ship a new reviewed migration.
- `TenantScopeRequired`: fix caller context; do not substitute global scope unless authorization explicitly permits it.
- `ConcurrentModification`: reload/re-evaluate the business command; do not blindly overwrite a concurrent update.
- `TransactionRequired`: execute row-locking logic inside `AtomicTransaction`.
- `post_commit_failure`: treat the database state as committed. Do not replay the transaction automatically; recover the failed effect separately, preferably from a durable outbox.
- Pool exhaustion: inspect query latency, slow transactions, lock waits, and concurrency before increasing pool size.

## Observability

Database/migration code uses `tracing` where currently instrumented. Configure the subscriber at the service binary boundary. Add service-level metrics for query latency, errors, pool saturation, transaction retries, lock timeouts, migration duration, and tenant-denial events. Never log raw database URLs, password/API-key hashes, tokens, cursor keys, or unrestricted query values.

## lex_rest operations

Build the core and the optional Axum/Rustls adapter in CI. Configure transport security only from a trusted deployment boundary, supply production authentication/throttle/idempotency implementations, configure explicit WSS origin allowlists, and alert on authentication denial, throttling, 5xx, idempotency persistence failures and WebSocket handler failures. See `docs/LEX_REST.md`.
