> **Historical documentation.** These pre-audit notes may describe obsolete APIs or ownership. Use the [current developer documentation](README.md) and executable acceptance examples for the supported contract.

# SQLite Backend Guide

SQLite is a first-class local-development backend. PostgreSQL remains the feature-complete production backend for capabilities that depend on PostgreSQL semantics such as RLS, read replicas, advisory migration locks, row-level `FOR UPDATE` locks, PostgreSQL JSON operators, and schema introspection/diff.

## Zero-configuration development

When `APP_PROFILE` is not `prod`/`production` and `DATABASE_URL` is absent, `db::Settings` selects:

```text
sqlite://db.sqlite3
```

`Database::initialize()` opens the file with create-if-missing enabled, foreign keys enabled, a busy timeout, and WAL mode for file-backed databases. Therefore a new developer does not need PostgreSQL just to start using the ORM.

```rust
use db::{Database, DatabaseBackend};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let database = Database::initialize().await?;
    assert_eq!(database.backend(), DatabaseBackend::Sqlite);
    let pool = database.sqlite_pool().expect("SQLite backend").clone();
    // use pool below
    database.close().await;
    Ok(())
}
```

## Explicit in-memory SQLite

```bash
export DATABASE_URL='sqlite::memory:'
export APP__DATABASE__BACKEND=sqlite
```

For in-memory databases use one connection if multiple pooled connections need to see the same schema/data:

```toml
[database]
max_connections = 1
min_connections = 1
```

## Models and QuerySet

`#[derive(Model)]` now generates both PostgreSQL and SQLite SQLx row decoding plus two manager entry points:

```rust
let pg = Customer::objects(pg_pool).queryset();
let sqlite = Customer::objects_sqlite(sqlite_pool).queryset();
```

For a fresh local database, the generated async helper can create the model table and compatible indexes when absent:

```rust
let customers = Customer::objects_sqlite_auto(sqlite_pool.clone()).await?;
let stored = customers.queryset().create(&customer).await?;
```

The auto helper is intentionally non-destructive. It creates missing tables/indexes but does not alter an existing table to match changed Rust metadata. Use migrations for schema evolution.

## SQLite-compatible schema mapping

The model metadata remains canonical. SQLite DDL maps types by affinity:

| Framework/PostgreSQL type | SQLite storage |
| --- | --- |
| SMALLINT / INTEGER / BIGINT / BIGSERIAL | INTEGER |
| BOOLEAN | INTEGER |
| REAL / DOUBLE PRECISION / NUMERIC | REAL |
| UUID | TEXT |
| TIMESTAMP / TIMESTAMPTZ / DATE / TIME / INTERVAL | TEXT |
| JSON / JSONB | TEXT |
| BYTEA | BLOB |
| INET / CIDR | TEXT |

`BIGSERIAL PRIMARY KEY` becomes `INTEGER PRIMARY KEY AUTOINCREMENT`. `now()` becomes `CURRENT_TIMESTAMP`. PostgreSQL casts on defaults such as `'{}'::jsonb` are removed. Singular foreign keys are rendered inline because SQLite cannot use PostgreSQL-style `ALTER TABLE ... ADD CONSTRAINT` for that operation.

## Transactions

Use `SqliteAtomicTransaction`:

```rust
use db_controller::{SqliteAtomicTransaction, TransactionOptions};

let mut tx = SqliteAtomicTransaction::begin(&sqlite_pool, TransactionOptions::default()).await?;
let rows = tx.queryset::<Customer>().all().await?;
tx.commit().await?;
```

Savepoints and `on_commit` callbacks are supported. PostgreSQL-specific RLS tenant GUCs and `SELECT ... FOR UPDATE` are not emulated.

## Migrations

The migration CLI selects SQLite when the URL begins with `sqlite:`:

```bash
export DATABASE_URL='sqlite://db.sqlite3'

cargo run -p lex-migration --bin migration-cli -- --dir ./migrations status
cargo run -p lex-migration --bin migration-cli -- --dir ./migrations migrate --dry-run
cargo run -p lex-migration --bin migration-cli -- --dir ./migrations migrate
```

SQLite migrations are checksum tracked in `_migrations`, applied transactionally, and support rollback/reset/reapply/pending/status. PostgreSQL advisory locks are not available on SQLite. SQL migration files must contain SQL accepted by the selected backend; backend-specific migrations should be kept separate when syntax differs.

## Query compatibility

Portable QuerySet operations include normal filtering/comparisons, case-insensitive LIKE lookup, ordering, pagination, create/get/count/exists, direct updates, expression updates, optimistic version updates, soft-delete/restore/hard-delete, and SQLite-compatible projections.

The framework fails explicitly instead of pretending equivalent semantics for PostgreSQL-only operations including:

- row-level `FOR UPDATE` / `NOWAIT` / `SKIP LOCKED`;
- PostgreSQL RLS and transaction GUC tenant context;
- PostgreSQL JSON containment/key operators;
- PostgreSQL-only aggregate functions such as `ARRAY_AGG`, `JSONB_AGG`, `BOOL_AND`, `STDDEV_SAMP`, and `VAR_SAMP`;
- PostgreSQL JSON-formatted `EXPLAIN`;
- PostgreSQL schema introspection/diff in the migration CLI.

For production systems that require those guarantees, use PostgreSQL.

## Production safety

Development falls back to SQLite automatically. Production does not. With:

```bash
export APP_PROFILE=production
```

an explicit `DATABASE_URL` is required. This prevents a missing production secret from silently creating `db.sqlite3` on a server.
