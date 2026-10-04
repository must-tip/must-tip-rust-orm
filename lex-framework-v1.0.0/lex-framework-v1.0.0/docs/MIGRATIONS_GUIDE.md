> **Historical documentation.** These pre-audit notes may describe obsolete APIs or ownership. Use the [current developer documentation](README.md) and executable acceptance examples for the supported contract.

# Migrations Guide

The project supports reviewed paired SQL migrations and typed migration operations.

## CLI

```bash
cargo run -p lex-migration --bin migration-cli -- --help
```

### Status

```bash
cargo run -p lex-migration --bin migration-cli -- \
  --db-url "$DATABASE_URL" --dir ./migrations status
```

### Pending

```bash
cargo run -p lex-migration --bin migration-cli -- \
  --db-url "$DATABASE_URL" --dir ./migrations pending
```

### Dry-run apply

```bash
cargo run -p lex-migration --bin migration-cli -- \
  --db-url "$DATABASE_URL" --dir ./migrations migrate --dry-run
```

### Apply

```bash
cargo run -p lex-migration --bin migration-cli -- \
  --db-url "$DATABASE_URL" --dir ./migrations migrate
```

### Roll back latest

```bash
cargo run -p lex-migration --bin migration-cli -- \
  --db-url "$DATABASE_URL" --dir ./migrations rollback
```

### Reapply latest

```bash
cargo run -p lex-migration --bin migration-cli -- \
  --db-url "$DATABASE_URL" --dir ./migrations reapply
```

### Reset

```bash
cargo run -p lex-migration --bin migration-cli -- \
  --db-url "$DATABASE_URL" --dir ./migrations reset --yes
```

Reset is destructive. Never use it on production or valuable data.

## Creating a migration

The current CLI creates an immutable migration from reviewed UP/DOWN SQL inputs:

```bash
cargo run -p lex-migration --bin migration-cli -- \
  --dir ./migrations \
  make add_customer_status \
  --up ./tmp/add_customer_status.up.sql \
  --down ./tmp/add_customer_status.down.sql
```

## Schema diff

Register models, then compare desired metadata with PostgreSQL:

```bash
cargo run -p lex-migration --bin migration-cli -- \
  --db-url "$DATABASE_URL" --dir ./migrations diff
```

Review all emitted SQL. The current framework does not claim a complete Django-style migration optimizer/squasher/autodetector.

## Release rules

- Applied migration content is immutable.
- UP and DOWN content is checksummed.
- Drift is rejected.
- Apply is transaction/advisory-lock protected.
- Destructive operations require explicit rollback review.
- Do not assume a generated `down_sql()` can recreate external/dependent objects that were never represented in metadata.
