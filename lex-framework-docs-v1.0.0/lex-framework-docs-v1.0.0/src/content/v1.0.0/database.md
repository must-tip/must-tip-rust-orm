# Databases

Lex v1.0.0 supports **PostgreSQL** as the feature-rich production backend and **SQLite** as a first-class development/lightweight backend.

## Development with SQLite

When development configuration does not provide another database URL, the framework can use:

```text
sqlite://db.sqlite3
```

The local file is created automatically when required.

Use SQLite for local development, examples, tests, small deployments whose concurrency semantics are appropriate, and onboarding.

## PostgreSQL

Use PostgreSQL when you need the framework's PostgreSQL-specific capabilities such as row-level security, read replicas, PostgreSQL advisory migration locks, PostgreSQL row-locking semantics, and PostgreSQL-specific operators/index behavior.

```bash
export DATABASE_URL='postgresql://user:password@localhost/lex'
```

## Backend behavior

The framework intentionally does not pretend every database has identical semantics. Portable ORM operations work across supported backends; PostgreSQL-specific operations remain explicit and fail clearly when requested on SQLite.

## Financial precision on SQLite

Exact decimal values are not mapped through SQLite `REAL`. The framework's SQLite path uses exact decimal string representation so financial values are not silently converted to floating-point approximations.
