# Lex Framework v1.0.0

Lex is a Rust 2024 application framework with PostgreSQL and SQLite backends, a typed ORM, migrations, REST APIs, authentication, permissions, middleware, media services, secure input handling, and a convention-first CLI. It is inspired by Django and Django REST Framework while retaining Rust-native type safety, explicit async execution, and database-enforced security where the backend supports it.

## Workspace

- `crate/orm` — `#[derive(Model)]` procedural macro that generates schema metadata, typed field/relation descriptors, validation, SQLx row decoding, and a Django-style `Model::objects(pool)` manager entry point.
- `crate/db_controller` — canonical `Entity`, typed `QuerySet`, predicates/lookups, relation loading, repositories, transactions/savepoints, tracked model state, tenant/RLS controls, and transport-neutral DRF-style serializer/filter/pagination policy.
- `crate/models` — current user, tenant, RBAC, API-key, session, membership, and related domain models.
- `crate/migration` — immutable/checksummed SQL migrations plus typed forward/reverse migration operations and a deterministic dependency graph.
- `crate/db` — backend selection, zero-configuration SQLite development, validated PostgreSQL TLS/runtime settings, connection retry/health checks, and controlled migration startup.
- `crate/secure_input` — input normalization/validation/sanitization and context-specific output encoding.
- `crate/media` — framework media storage/scanning contracts with memory and filesystem storage, S3-compatible configuration boundaries, upload validation, and native ClamAV/clamd scanning support.

## ORM capabilities

The primary model API is `ModelManager<T>::queryset()` or generated `T::objects(pool).queryset()`.

Implemented QuerySet behavior includes:

- composable `filter`, `exclude`, AND/OR/NOT predicates and field-to-field comparisons;
- typed values for Boolean, integer, floating-point, decimal, UUID, timestamp, text, JSON, and bytes;
- exact/comparison/null/`IN`/range/text lookups;
- deterministic ordering, distinct, page/limit/offset, first/last/earliest/latest/get/exists/count/values/aggregate/explain;
- create/bulk-create/concurrency-safe unique-field `get_or_create_by`;
- guarded bulk update with immutable tenant/framework-owned fields, automatic version advancement, arithmetic increment, optimistic versioned update, soft delete/restore/hard delete;
- singular relation join loading and batched relation prefetching without hidden N+1 queries;
- tenant scoping that fails closed and an authorization-gated global-scope escape hatch;
- row locks only inside a transaction; transaction-local tenant/RLS context, savepoints, `on_commit`, timeouts, and bounded retry policy;
- explicit dirty-model tracking and transactional `Tracked<T>::save()` with optimistic concurrency when a version column exists.

The legacy `Repository<T>` API is retained for compatibility for global models. It deliberately refuses tenant-aware models because its original contract has no tenant context.

## DRF-aligned boundary

`db_controller::rest` is transport-neutral. It does not invent an Axum/Actix router or application authentication scheme. It provides reusable policy primitives for:

- model-derived serializer field policy;
- create/update/partial-update validation;
- read-only/write-only handling, with encrypted credential material omitted by default;
- allowlisted filtering, search, and ordering;
- page-number, limit/offset, and signed keyset cursor pagination;
- HMAC-SHA256 cursor integrity with bounded token size and previous-key verification for controlled rotation;
- request-size/cardinality guards and stable structured error codes.

See `docs/DJANGO_DRF_ALIGNMENT.md`.

## Toolchain and dependencies

`rust-toolchain.toml` pins Rust `1.97.1`; the workspace uses Edition 2024 and Cargo resolver 3. All direct third-party dependency versions are declared once under `[workspace.dependencies]` and inherited by member crates.

The repaired archive intentionally does not fabricate a new `Cargo.lock`: this execution environment has no Rust toolchain and cannot resolve crates. A Rust-enabled release environment must generate the lockfile, review it, then commit it before production builds.

## Setup

```bash
rustup show
cargo generate-lockfile
lex verify framework
```

For local development you can omit `DATABASE_URL`; the framework defaults to `sqlite://db.sqlite3` and creates the SQLite file automatically. Production requires an explicit database URL.

```rust
let database = db::Database::initialize().await?;
let sqlite = database.sqlite_pool().expect("default development SQLite").clone();
```

See `docs/SQLITE_GUIDE.md` and run `lex verify sqlite` for the SQLite-specific verification gate.

For destructive database integration tests, point only at a disposable PostgreSQL database:

```bash
export TEST_DATABASE_URL='postgres://.../disposable_test_db'
export CONFIRM_DISPOSABLE_DATABASE=yes
lex verify postgres
```

Never point that test command at production or a database containing valuable data.

## Production safety defaults

- PostgreSQL TLS certificate/hostname verification defaults to `VerifyFull`.
- Credentials come from `DATABASE_URL`; committed configuration contains no database password.
- Automatic migrations are disabled by default.
- Migration UP and DOWN content is checksummed and migrations are transaction/advisory-lock protected. Model-generated typed migrations retain declared indexes, composite uniqueness, CHECK constraints, and singular foreign keys.
- Runtime values are SQLx binds; dynamic identifiers/types/defaults are validated before SQL construction.
- Tenant-aware QuerySets require a tenant scope unless an application authorization object explicitly grants global access.
- PostgreSQL RLS helpers provide an independent defense-in-depth tenant boundary.
- Unscoped destructive QuerySet mutations are rejected unless an explicit all-row mutation capability is requested.
- `select_for_update` is rejected outside an actual transaction.
- Workspace lints forbid unsafe Rust and unfinished `todo!`/`unimplemented!` code.

## Release status

This source tree is a production-hardened candidate, not a certified production release. The current repair environment cannot run `cargo check`, Clippy, tests, or a release build because no Rust compiler is installed and toolchain downloads are blocked. Production promotion requires the compiler-backed and PostgreSQL gates in `docs/VERIFICATION.md`.

## Developer documentation

Start here if you are building with the framework:

- `docs/QUICKSTART.md` — first model, automatic SQLite/PostgreSQL connection, query, transaction, migration
- `docs/SQLITE_GUIDE.md` — zero-configuration SQLite, QuerySet compatibility, transactions, migrations, backend differences
- `docs/DEVELOPER_GUIDE.md` — complete developer handbook
- `docs/PROJECT_STRUCTURE.md` — crate responsibilities and dependency direction
- `docs/ORM_GUIDE.md` — models, QuerySet, relations, tenancy, transactions
- `docs/MIGRATIONS_GUIDE.md` — migration CLI and release workflow
- `docs/REST_FRAMEWORK_GUIDE.md` — `lex_rest` authentication, permissions, views, routing, middleware, WebSockets
- `docs/SECURE_INPUT_GUIDE.md` — validation, sanitization, and context-specific encoding
- `docs/TESTING_AND_VERIFICATION.md` — developer and PostgreSQL verification workflow

Architecture, security, and operations references:

- `docs/ARCHITECTURE.md`
- `docs/API_CONTRACTS.md`
- `docs/DJANGO_DRF_ALIGNMENT.md`
- `docs/LEX_REST.md`
- `docs/SECURITY.md`
- `docs/OPERATIONS.md`
- `docs/GOVERNANCE.md`
- `docs/VERIFICATION.md`

## Lex CLI

The workspace now includes a first-party `lex` developer CLI for project scaffolding and server management.

```bash
cargo install --path crate/lex_cli --force
lex create settings --project-name my_platform
lex create crate billing --url /api/v1/billing
lex doctor
lex migrate
lex start server
```

Generated application crates contain `models.rs`, `views.rs`, and app-local `urls.rs`; the CLI mounts them into the central project route table automatically. See `docs/CLI_GUIDE.md` for the complete workflow.

- `docs/CLI_MIGRATION_FROM_SCRIPTS.md` — migration map from legacy repository scripts to native `lex` commands


## Authentication

Authentication is framework-owned. Choose session, signed-cookie, bearer, or hybrid authentication in `Config.toml`; views are protected by default and use `ViewAccess::Public` only when anonymous access is intentional. See `docs/AUTHENTICATION_GUIDE.md`.
