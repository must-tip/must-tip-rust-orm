# SQLite Backend Implementation

This repository now supports PostgreSQL and SQLite through the same framework workspace.

## Default development behavior

When `APP_PROFILE` is not `prod`/`production` and neither `DATABASE_URL` nor the CLI database URL is supplied, the database crate resolves to:

```text
sqlite://db.sqlite3
```

The SQLite connection is opened with `create_if_missing`, so the database file is created automatically on first connection.

Production remains fail-closed: production requires an explicit database URL and never silently selects SQLite.

## Added backend capabilities

- `db::DatabaseBackend::{Auto, Postgres, Sqlite}` and URL-based backend resolution.
- `db::DatabasePool::{Postgres, Sqlite}` and dual-backend health/close behavior.
- SQLite connect options with foreign keys, WAL for file-backed databases, busy timeout, and bounded pooling.
- Single-connection enforcement for `sqlite::memory:` databases.
- `db_controller::DatabaseDialect` and row-associated `QueryExecutor`.
- `SqlitePoolExecutor` and `SqliteTransactionExecutor`.
- SQLite QuerySet execution for portable CRUD/filter/update/delete/count/value operations.
- Explicit errors for PostgreSQL-only operations rather than unsafe approximations.
- SQLite-compatible Entity DDL and index rendering.
- Exact `NUMERIC`/Decimal persistence through text affinity to avoid IEEE-754 precision loss.
- Inline SQLite foreign-key generation for singular relations.
- `SqliteModelManager` and `#[derive(Model)]` helpers `objects_sqlite` / `objects_sqlite_auto`.
- `SqliteAtomicTransaction` with savepoints and post-commit callbacks.
- Checksum-tracked SQLite migration application/rollback/reset/reapply/status/pending support.
- Migration CLI backend dispatch based on database URL.
- Development and backend-specific documentation.
- SQLite integration tests and source-policy validation script.

## PostgreSQL-only capabilities by design

Some features depend on PostgreSQL semantics and are rejected explicitly on SQLite instead of being emulated incorrectly, including PostgreSQL RLS/GUC tenancy enforcement, row-level `FOR UPDATE` locking variants, PostgreSQL advisory migration locking, read replicas, PostgreSQL JSON containment/operators, PostgreSQL JSON EXPLAIN output, and PostgreSQL-only aggregate functions.

Application-level tenant scoping through QuerySet remains available, but database-enforced RLS remains PostgreSQL-specific.

## Validation

Run:

```bash
lex audit workspace
lex audit sql
lex audit source
lex verify sqlite
```

The first three source-policy checks were run successfully when this replacement archive was produced. A Rust compiler was not available in the artifact-generation environment, so `cargo test` must be run on a Rust-enabled machine/VPS before release.
