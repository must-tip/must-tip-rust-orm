# Transactions and concurrent editors

Transactions preserve a consistent unit of work on one database connection. Pool-based QuerySets are separate operations; constructing several managers from the same pool does not place them in a shared transaction.

PostgreSQL's `AtomicTransaction` provides transaction-local tenant context, supported transaction options, savepoints and post-commit callbacks. Pass its transaction to the public transaction executor/QuerySet API. Callbacks run only after successful commit; callbacks registered after a rolled-back savepoint are discarded. Retry policy is for retryable transaction failures, not arbitrary application errors: external side effects must be safe to repeat or deferred until commit.

SQLite uses SQLx transactions and `SqliteTransactionExecutor`. The file-backed consumer test creates a row inside a transaction, discards a savepoint, commits, discards another transaction, closes the pool and reopens the database to observe only committed data. PostgreSQL-only isolation/locking/RLS options are not silently mapped to SQLite. Refer to the [compatibility table](../guides/databases.md).

```rust,ignore
use lex_framework::db_controller::{QuerySet, SqliteTransactionExecutor};
let mut transaction = pool.begin().await?;
QuerySet::<Article, _>::from_executor(SqliteTransactionExecutor::new(&mut transaction))
    .for_tenant(authorized_tenant_id)
    .create(&article).await?;
transaction.commit().await?;
```

For independently loaded editors, declare an integer version field with `#[model(version = "version")]` and a matching database initial default. Wrap the loaded model in `Tracked::new`. PostgreSQL uses `.save(&manager).await`; SQLite uses `.save_sqlite(&manager).await`. A dirty save checks the original version and advances it after success. An outdated writer receives `ModelError::ConcurrentModification` without overwriting the winner. Reload and resolve the conflict deliberately; blindly retrying the old model defeats concurrency protection.

A clean save performs no database transaction and does not change the version. Tracked saves reject changes to primary key, timestamps, tenant, deletion and version identity fields. Generic set/bulk operations have different semantics and do not provide a per-editor version comparison.

RLS tenant settings are transaction-local. A missing setting matches no tenant rows. Commit and rollback must leave no tenant context on a reused pooled connection; the restricted-role PostgreSQL test verifies the same connection is reused without inheriting the previous tenant. Database owners/superusers can bypass ordinary RLS; production application roles must not carry BYPASSRLS or ownership privileges.
