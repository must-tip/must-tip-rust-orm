# Transactions and concurrency

Lex includes explicit transaction abstractions for PostgreSQL and SQLite.

## PostgreSQL transaction

The PostgreSQL transaction layer supports isolation choices, read-only/deferrable options where valid, savepoints, timeout controls, tenant context, row locking, and bounded retry for retryable database errors.

```rust
let mut tx = AtomicTransaction::begin(&pool, TransactionOptions::default()).await?;

let user = tx.queryset::<User>()
    .filter(User::ID.eq(user_id))
    .get()
    .await?;

tx.commit().await?;
```

## SQLite transaction

SQLite provides its own transaction implementation and savepoint flow. PostgreSQL-only locking semantics are not silently emulated.

## Retry rule

Database retries are only safe when the operation is idempotent with respect to external effects. Do not blindly retry a transaction that also charges a card, sends an email, or calls an external service unless those effects have their own idempotency design.

## Optimistic concurrency

Version columns can be used for optimistic update checks. A stale writer should receive a concurrent-modification failure instead of overwriting a newer state silently.
