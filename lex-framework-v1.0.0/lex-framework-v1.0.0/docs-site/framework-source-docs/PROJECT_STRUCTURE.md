> **Historical documentation.** These pre-audit notes may describe obsolete APIs or ownership. Use the [current developer documentation](README.md) and executable acceptance examples for the supported contract.

# Project Structure

## Dependency direction

The intended direction is:

```text
service/application
    ├── lex_rest
    ├── models
    └── secure_input
          ↓
         orm
          ↓
    db_controller
          ↓
         sqlx
          ↓
      PostgreSQL
```

`migration` consumes model/entity metadata for schema work. `db` owns PostgreSQL connection/configuration concerns. `media` is a service integration boundary.

## Crate map

### `crate/orm`
Procedural macro crate. Do not put runtime database connection logic here.

### `crate/db_controller`
Canonical persistence core. New generic ORM behavior belongs here rather than being duplicated in services.

### `crate/db`
Runtime settings, pool initialization, TLS, retries, health checks, read-replica initialization, optional migration startup.

### `crate/migration`
Migration file scanning, checksums, transactional application, dependency graph, typed operations, schema diff CLI.

### `crate/models`
Current shared/domain model definitions and registry helper.

### `crate/lex_rest`
Application/transport layer: request pipeline, auth, permission, routing, generic views, viewsets, middleware, throttling, idempotency, audit, schema, WebSockets, Axum adapter.

### `crate/secure_input`
Boundary validation/sanitization/output encoding.

### `crate/media`
Integration contract/boundary. Do not infer that production object storage exists merely because media field aliases/modules exist.

## Source-of-truth rules

1. Model metadata is the schema/persistence source of truth.
2. `db_controller` owns ORM semantics.
3. `lex_rest` consumes persistence policy; it does not replace it.
4. Authentication/authorization decisions remain service-owned.
5. PostgreSQL RLS remains an independent security boundary.
6. Runtime secrets stay outside committed configuration.
7. Migrations are reviewed release artifacts.
