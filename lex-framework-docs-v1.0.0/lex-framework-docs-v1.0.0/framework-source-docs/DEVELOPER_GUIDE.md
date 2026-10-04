
> **New simplified application API:** For website-style documentation and current examples of framework-owned authentication, declarative view permissions, and middleware, start with [`docs/website/README.md`](website/README.md).

# Lex Rust Framework — Developer Guide

This guide is the canonical developer-facing manual for the current workspace. It explains how the framework is structured, how to define models, query PostgreSQL, use transactions and migrations, build REST APIs with `lex_rest`, validate untrusted input, test integrations, and operate the framework safely.

> **Current scope:** the framework is PostgreSQL-first. It does not currently provide a generic SQLite/MySQL backend. Production media storage/scanning/transcoding is also intentionally left as an integration boundary.

## 1. What this framework provides

The workspace combines several crates into one backend framework:

| Crate | Purpose |
|---|---|
| `orm` | `#[derive(Model)]` procedural macro for model metadata, validation, typed field descriptors, SQLx row decoding, indexes, constraints, relations, and `Model::objects(pool)` |
| `db_controller` | Core ORM: `Entity`, typed fields, `QuerySet`, predicates, relations, repositories, transactions, model state, tenant/RLS controls, serializer/query policies |
| `db` | PostgreSQL settings, TLS policy, connection pooling, retries, replicas, health checks, optional migration startup |
| `migration` | Immutable/checksummed SQL migrations, migration CLI, typed migration operations, dependency graph, schema diff support |
| `models` | Example/domain models for users, tenants, memberships, roles, permissions, sessions/API keys |
| `lex_rest` | DRF-style request pipeline, authentication, permissions, generic views, viewsets, routing, middleware, idempotency, throttling, audit, WebSockets, OpenAPI metadata, Axum adapter |
| `secure_input` | Normalization, validation, sanitization, output encoding, composable input pipelines |
| `media` | Media storage/scanning layer with memory and filesystem backends, S3-compatible configuration boundaries, and ClamAV/clamd scanning support |

The architectural rule is simple:

```text
models / service code
        ↓
      orm macro
        ↓
   db_controller
        ↓
       sqlx
        ↓
    PostgreSQL
```

HTTP/API code lives above persistence:

```text
lex_rest
   ↓
db_controller
   ↓
PostgreSQL
```

## 2. Toolchain and workspace setup

The workspace targets:

- Rust 1.97.1
- Edition 2024
- Cargo resolver 3
- PostgreSQL through SQLx 0.9

A normal developer setup is:

```bash
rustup show
cargo generate-lockfile
cargo check --workspace
cargo test --workspace
```

The repository also provides:

```bash
lex verify framework
```

For PostgreSQL integration tests, use only a disposable database:

```bash
export TEST_DATABASE_URL='postgres://USER:PASSWORD@HOST:5432/disposable_test_db'
export CONFIRM_DISPOSABLE_DATABASE=yes
lex verify postgres
```

Never point destructive integration tests at production or valuable data.

## 3. Configuration

The `db` crate exposes `Settings`, `DatabaseSettings`, `ServerSettings`, and `Database`.

The required database secret is supplied through:

```bash
export DATABASE_URL='postgres://user:password@host:5432/database'
```

Non-secret nested configuration uses `APP__...` keys. Examples:

```bash
export APP__DATABASE__MAX_CONNECTIONS=20
export APP__DATABASE__MIN_CONNECTIONS=1
export APP__DATABASE__CONNECT_TIMEOUT_SECS=10
export APP__DATABASE__CONNECT_RETRIES=5
export APP__DATABASE__REQUIRE_SSL=true
export APP__DATABASE__RUN_MIGRATIONS=false
export APP__DATABASE__MIGRATIONS_DIR='./migrations'
export APP__SERVER__HOST='127.0.0.1'
export APP__SERVER__PORT=8080
```

Read replicas are provided through `DATABASE_READ_REPLICA_URLS_JSON`.

In production:

```bash
export APP_PROFILE=production
```

The settings loader disables `.env` loading for the production profile.

### Initialize the database

```rust
use db::Database;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let database = Database::initialize().await?;

    database.health_check_all().await?;

    let write_pool = database.write_pool();
    let read_pool = database.read_pool();

    // Use pools in application services.

    database.close().await;
    Ok(())
}
```

`write_pool()` is always the primary. `read_pool()` uses an initialized replica in round-robin order, or falls back to primary if no replica is available and configuration allows fallback.

Use the primary for:

- writes;
- migrations;
- row locks;
- transactions;
- read-after-write consistency;
- business commands requiring the latest authoritative state.

## 4. Defining models

The normal model experience uses `#[derive(Model)]` from the `orm` crate.

```rust
use db_controller::{BooleanField, DateTimeField, TextField, UUIDField};
use orm::Model;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "customers")]
#[timestamps]
#[model(ordering = "-created_at")]
pub struct Customer {
    #[field(primary_key)]
    pub id: UUIDField,

    #[field(not_empty, max_length = 120)]
    pub name: TextField,

    #[field(unique, index, not_empty, max_length = 255)]
    pub email: TextField,

    #[field(default = false)]
    pub verified: BooleanField,

    #[field(soft_delete)]
    pub deleted_at: Option<DateTimeField>,
}
```

The macro generates the `Entity` implementation, model schema metadata, typed field descriptors, SQLx row decoding support, validation, relation metadata, index/constraint metadata, and `Customer::objects(pool)`.

### Table attributes

Use:

```rust
#[table(name = "customers")]
```

If no table name is supplied, the macro derives one from the struct name.

### Model attributes

Supported model options are:

```rust
#[model(ordering = "-created_at,name")]
#[model(tenant = "tenant_id")]
#[model(soft_delete = "deleted_at")]
#[model(version = "version")]
#[model(global)]
```

A model cannot be both `global` and explicitly tenant-scoped.

### Field attributes

Current field options include:

```text
primary_key
unique
index
nullable
soft_delete
encrypted
not_empty
bool_true
min = ...
max = ...
min_length = ...
max_length = ...
default = ...
regex = ...
depends_on = ...
```

Examples:

```rust
#[field(primary_key)]
pub id: UUIDField,

#[field(not_empty, unique, regex = r"^[^@]+@[^@]+\.[^@]+$", max_length = 255)]
pub email: TextField,

#[field(default = false)]
pub active: BooleanField,

#[field(nullable)]
pub nickname: Option<TextField>,

#[field(soft_delete)]
pub deleted_at: Option<DateTimeField>,
```

### Framework field wrappers

`db_controller` exposes:

- `UUIDField`
- `DateTimeField`
- `DecimalField`
- `PasswordField`
- `JsonField<T>`
- `BooleanField`
- `IntegerField`
- `FloatField`
- `TextField`
- `SlugField`
- `BinaryField`
- `ForeignKey<T>`
- `ManyToMany<T>`

Useful constructors:

```rust
use db_controller::{BooleanField, DateTimeField, JsonField, TextField, UUIDField};

let id = UUIDField::new();
let now = DateTimeField::now();
let name = TextField::new("Alice");
let active = BooleanField::new(true);
let metadata = JsonField::new(serde_json::json!({"source": "api"}));
```

### Passwords

`PasswordField` uses Argon2 and redacts its `Debug` representation.

```rust
use db_controller::PasswordField;

let password = PasswordField::hash("correct horse battery staple")?;
assert!(password.verify("correct horse battery staple")?);
```

Do not persist plaintext passwords into `PasswordField` manually.

### Encrypted fields

`#[field(encrypted)]` marks a field as sensitive in framework metadata and serializer policy. It does **not** automatically invent a key-management system. Service code remains responsible for applying the correct encryption/hashing transformation before persistence.

## 5. Relations

Relations are metadata-driven and are not persisted as ordinary columns.

A singular relation:

```rust
#[field(index)]
pub owner_id: UUIDField,

#[relation(local = "owner_id", model = "User", remote = "id")]
pub owner: Option<User>,
```

A many relation:

```rust
#[relation(model = "Membership", local = "id", remote = "user_id", many)]
#[serde(default)]
pub memberships: ManyToMany<UUIDField>,
```

Supported relation metadata includes:

```text
model
local
remote
many
related_name
on_delete
```

Relation loading is explicit. Accessing a Rust field never performs hidden database I/O.

## 6. Registering models

Typed schema discovery and migration diffing use the model registry.

The supplied `models` crate exposes:

```rust
models::register_all_models();
```

For your own application, register every model that must participate in schema discovery before running migration-diff logic.

An `Entity` can also register itself:

```rust
MyModel::register();
```

## 7. Model validation

Derived models expose validation methods generated by the macro.

```rust
let customer = Customer {
    id: UUIDField::new(),
    name: TextField::new("Alice"),
    email: TextField::new("alice@example.com"),
    verified: BooleanField::new(false),
    deleted_at: None,
};

customer.validate()?;
customer.full_clean()?;
```

Field-level rules such as `not_empty`, length bounds, regex checks, numeric bounds, and boolean assertions are applied by generated validation code.

## 8. The model manager and QuerySet

A derived model exposes a Django-style manager entry point:

```rust
let manager = Customer::objects(pool.clone());
let queryset = manager.queryset();
```

Equivalent explicit construction is available through `ModelManager<T>`.

QuerySets are lazy query plans. SQL is compiled at the execution boundary.

### Preview generated SQL

```rust
let preview = Customer::objects(pool.clone())
    .queryset()
    .filter(Customer::EMAIL.icontains("@example.com"))
    .preview()?;

println!("{}", preview.sql);
println!("binds={}", preview.parameter_count);
```

Values are bound through SQLx rather than interpolated into SQL strings.

## 9. Filtering and predicates

The derive macro exposes typed field descriptors such as `Customer::EMAIL`.

### Equality and comparison

```rust
let rows = Customer::objects(pool.clone())
    .queryset()
    .filter(Customer::VERIFIED.eq(true))
    .all()
    .await?;
```

Supported comparison helpers:

```text
eq
ne
gt
gte
lt
lte
```

### Null checks

```rust
Customer::DELETED_AT.is_null();
Customer::DELETED_AT.is_not_null();
```

### IN / NOT IN

```rust
let predicate = Customer::ID.is_in(vec![id1, id2, id3]);
```

### Range

```rust
let predicate = Customer::ID.between(lower, upper);
```

### Text lookups

```text
exact
iexact
contains
icontains
starts_with
istarts_with
ends_with
iends_with
```

Example:

```rust
let customers = Customer::objects(pool.clone())
    .queryset()
    .filter(Customer::EMAIL.icontains("@example.com"))
    .all()
    .await?;
```

### Combine predicates

`Predicate` supports AND/OR/NOT composition.

```rust
let predicate = Customer::VERIFIED
    .eq(true)
    .and(Customer::EMAIL.icontains("@example.com"));

let rows = Customer::objects(pool.clone())
    .queryset()
    .filter(predicate)
    .all()
    .await?;
```

### Field-to-field comparison

```rust
use db_controller::ComparisonOperator;

let predicate = Model::LEFT.compare_field(ComparisonOperator::Gt, Model::RIGHT);
```

### JSONB predicates

For JSON fields:

```rust
let contains = Model::METADATA.json_contains(serde_json::json!({"tier": "gold"}));
let has_key = Model::METADATA.json_has_key("tier");
```

These compile to PostgreSQL JSONB behavior and are rejected for non-JSON fields.

## 10. Ordering and pagination

```rust
let rows = Customer::objects(pool.clone())
    .queryset()
    .order_by(Customer::EMAIL.asc())
    .limit(100)
    .offset(0)
    .all()
    .await?;
```

Or page explicitly:

```rust
let rows = Customer::objects(pool.clone())
    .queryset()
    .page(1, 50)?
    .all()
    .await?;
```

`page()` requires `page >= 1` and `page_size` in `1..=10000`.

Other ordering helpers:

```rust
use db_controller::OrderBy;

OrderBy::asc("created_at");
OrderBy::desc("created_at").nulls_last();
```

## 11. Query terminal methods

Common terminals include:

```text
all
first
last
earliest
latest
get
exists
count
values
values_list
aggregate
explain
refresh_from_db
```

Examples:

```rust
let count = Customer::objects(pool.clone())
    .queryset()
    .filter(Customer::VERIFIED.eq(true))
    .count()
    .await?;

let first = Customer::objects(pool.clone())
    .queryset()
    .first()
    .await?;

let one = Customer::objects(pool.clone())
    .queryset()
    .filter(Customer::EMAIL.eq("alice@example.com"))
    .get()
    .await?;
```

`get()` requires exactly one visible row and errors on zero or multiple matches.

## 12. Soft delete

A model with a soft-delete column automatically hides deleted rows from ordinary QuerySets.

Include deleted rows:

```rust
let all_rows = Customer::objects(pool.clone())
    .queryset()
    .with_deleted()
    .all()
    .await?;
```

Only deleted rows:

```rust
let deleted = Customer::objects(pool.clone())
    .queryset()
    .only_deleted()
    .all()
    .await?;
```

Soft delete:

```rust
Customer::objects(pool.clone())
    .queryset()
    .filter(Customer::ID.eq(customer_id))
    .soft_delete()
    .await?;
```

Restore:

```rust
Customer::objects(pool.clone())
    .queryset()
    .only_deleted()
    .filter(Customer::ID.eq(customer_id))
    .restore()
    .await?;
```

Hard delete:

```rust
Customer::objects(pool.clone())
    .queryset()
    .filter(Customer::ID.eq(customer_id))
    .hard_delete()
    .await?;
```

Unscoped bulk mutation is rejected unless the caller explicitly opts in with `allow_all_mutation()`.

## 13. Creating and updating rows

### Create

```rust
let stored = Customer::objects(pool.clone())
    .queryset()
    .create(&customer)
    .await?;
```

### Bulk create

```rust
let inserted = Customer::objects(pool.clone())
    .queryset()
    .bulk_create(&customers)
    .await?;
```

### Update a filtered set

```rust
use serde_json::{Map, json};

let mut values = Map::new();
values.insert("verified".to_owned(), json!(true));

let changed = Customer::objects(pool.clone())
    .queryset()
    .filter(Customer::ID.eq(customer_id))
    .update(values)
    .await?;
```

Framework-owned immutable fields such as primary keys, tenant ownership, soft-delete state, timestamps, and optimistic-version fields are guarded in generic mutation paths.

### Atomic expressions

The typed field API supports set/copy/arithmetic expressions:

```rust
let changed = Account::objects(pool.clone())
    .queryset()
    .filter(Account::ID.eq(account_id))
    .update_expressions(&[
        Account::BALANCE.add(100_i64),
    ])
    .await?;
```

Use database-side arithmetic for counters/balances where lost updates must be avoided.

## 14. Tenant-scoped models

Declare tenant ownership on the model:

```rust
#[derive(Model)]
#[table(name = "tenant_documents")]
#[model(tenant = "tenant_id")]
struct TenantDocument {
    #[field(primary_key)]
    id: UUIDField,

    #[field(index)]
    tenant_id: UUIDField,

    #[field(not_empty)]
    title: TextField,
}
```

A tenant-aware QuerySet must be scoped:

```rust
let documents = TenantDocument::objects(pool.clone())
    .queryset()
    .for_tenant(tenant_id)
    .all()
    .await?;
```

The ORM intentionally fails closed instead of silently returning cross-tenant data.

### Global access

Cross-tenant access requires an application-defined capability implementing `GlobalScopeAuthorization`:

```rust
let queryset = TenantDocument::objects(pool.clone())
    .queryset()
    .global_scope(&authorization)?;
```

The ORM does not hard-code “admin means global.” Your service's authoritative authorization layer must make that decision.

## 15. PostgreSQL row-level security

`db_controller::security` provides RLS helpers including:

- `TenantRlsPolicy`
- `set_transaction_tenant`
- `clear_transaction_tenant`
- `TENANT_GUC`

RLS is a defense-in-depth boundary. ORM tenant predicates do not replace PostgreSQL policy enforcement.

Use tenant context inside a transaction when the database policy expects it.

## 16. Relations: select_related and prefetch_related

Singular relations can be loaded explicitly with `select_related`.

Many relations are loaded through batched prefetching with `prefetch_related`.

The API returns wrapper types:

- `Joined<T, R>`
- `Prefetched<T, R>`

This design avoids hidden N+1 queries. Relation fields never trigger implicit network/database calls when accessed.

For advanced relation filtering, use the framework's typed relation metadata and supported related predicate APIs described in `docs/API_CONTRACTS.md` and `docs/DJANGO_DRF_ALIGNMENT.md`.

## 17. Transactions

Use `AtomicTransaction` for workflows that must share one PostgreSQL transaction.

```rust
use db_controller::{AtomicTransaction, TransactionOptions};

let mut tx = AtomicTransaction::begin(
    &pool,
    TransactionOptions::default(),
).await?;

let rows = tx
    .queryset::<Customer>()
    .filter(Customer::EMAIL.eq("alice@example.com"))
    .all()
    .await?;

// More reads/writes in the same transaction.

tx.commit().await?;
```

### Transaction options

`TransactionOptions` supports transaction-level behavior such as isolation, read-only/deferrable mode, and PostgreSQL timeouts.

### Tenant context in a transaction

```rust
tx.set_tenant(tenant_id).await?;
```

Clear it explicitly when needed:

```rust
tx.clear_tenant().await?;
```

### Savepoints

```rust
let mut savepoint = tx.create_savepoint().await?;

// ... work ...

tx.rollback_to(&savepoint).await?;
// or:
tx.release_savepoint(&mut savepoint).await?;
```

### on_commit

Register a callback that should run only after the transaction commits:

```rust
tx.on_commit(|| async move {
    // publish/notify after successful commit
});
```

Callbacks associated with rolled-back savepoint work are discarded.

### Row locking

`select_for_update` is transaction-only:

```rust
use db_controller::{LockWait, RowLock};

let row = tx
    .queryset::<Account>()
    .filter(Account::ID.eq(account_id))
    .select_for_update(RowLock::Update, LockWait::Wait)
    .get()
    .await?;
```

A pooled/non-transactional QuerySet rejects row-lock execution.

## 18. Transaction retry

`retry_transaction` retries bounded PostgreSQL serialization/deadlock failures.

Use it only when the transaction body is safe to retry. External side effects—charging a card, sending an email, publishing an irreversible message—must not be repeated accidentally.

For cross-service effects, prefer a durable outbox written in the same database transaction and published separately.

## 19. Tracked model state and optimistic concurrency

`Tracked<T>` retains the originally loaded state and can save only dirty fields.

When the model declares a `version` column, tracked/versioned writes use optimistic concurrency. A stale version produces a concurrent-modification error instead of silently overwriting another writer.

Use this for editable records where lost updates matter.

## 20. Legacy Repository API

`Repository<T>` remains for compatibility with global models.

For new application code, prefer:

```rust
Model::objects(pool).queryset()
```

Tenant-aware models deliberately refuse the legacy repository API because its original method signatures do not carry tenant authorization.

## 21. Migrations

The migration subsystem supports two compatible models:

1. paired immutable SQL migration files;
2. typed `MigrationOperation` + `MigrationGraph` operations.

### Migration CLI

The binary is:

```bash
cargo run -p migration --bin migration-cli -- --help
```

The CLI accepts `DATABASE_URL` through `--db-url` or the environment.

### Create a paired SQL migration

Prepare reviewed UP and DOWN SQL files, then:

```bash
cargo run -p migration --bin migration-cli -- \
  --dir ./migrations \
  make add_customer_status \
  --up /tmp/add_customer_status.up.sql \
  --down /tmp/add_customer_status.down.sql
```

### Apply migrations

```bash
cargo run -p migration --bin migration-cli -- \
  --db-url "$DATABASE_URL" \
  --dir ./migrations \
  migrate
```

### Dry-run pending SQL

```bash
cargo run -p migration --bin migration-cli -- \
  --db-url "$DATABASE_URL" \
  --dir ./migrations \
  migrate --dry-run
```

### Status and pending

```bash
cargo run -p migration --bin migration-cli -- \
  --db-url "$DATABASE_URL" \
  --dir ./migrations \
  status

cargo run -p migration --bin migration-cli -- \
  --db-url "$DATABASE_URL" \
  --dir ./migrations \
  pending
```

### Rollback / reapply

```bash
cargo run -p migration --bin migration-cli -- \
  --db-url "$DATABASE_URL" \
  --dir ./migrations \
  rollback

cargo run -p migration --bin migration-cli -- \
  --db-url "$DATABASE_URL" \
  --dir ./migrations \
  reapply
```

### Reset

Reset is destructive and requires explicit confirmation:

```bash
cargo run -p migration --bin migration-cli -- \
  --db-url "$DATABASE_URL" \
  --dir ./migrations \
  reset --yes
```

Do not run reset on production data.

### Schema diff

The CLI can compare registered desired models with the current PostgreSQL schema:

```bash
cargo run -p migration --bin migration-cli -- \
  --db-url "$DATABASE_URL" \
  --dir ./migrations \
  diff
```

The current project includes schema diff building blocks but is not a complete Django migration state optimizer/squashing/autodetection engine. Review generated SQL before applying it.

## 22. REST framework overview

`lex_rest` is the transport/application layer. Persistence remains in `db_controller`.

Important modules include:

- `request`
- `response`
- `authentication`
- `permissions`
- `generic`
- `views`
- `viewsets`
- `routing`
- `middleware`
- `throttling`
- `idempotency`
- `audit`
- `schema`
- `websocket`
- `adapters::axum`

Import common types from:

```rust
use lex_rest::prelude::*;
```

or module-specific paths under `lex_rest::rest_framework`.

## 23. Request principals and context

`Principal` represents the authenticated identity class. Current principal kinds include user/service/API-key/anonymous concepts.

Examples:

```rust
use lex_rest::rest_framework::request::{Principal, RequestContext, TransportSecurity};

let principal = Principal::user(
    user_id,
    "jwt",
    vec!["users.read".to_owned()],
)?;

let context = RequestContext::new(
    principal,
    Some(tenant_id),
    None,
    TransportSecurity::Secure,
    None,
    None,
)?;
```

`RequestContext` carries authoritative request information such as principal, tenant, tracing/deadline context, and transport security.

## 24. Authentication backends

Implement `AuthenticationBackend` for each authentication mechanism.

Authentication distinguishes:

- abstain: backend is not applicable;
- authenticated: valid identity;
- rejected: credentials were present but invalid.

This prevents an invalid credential from silently falling through to a weaker backend.

Compose backends with:

```rust
let auth = AuthenticationChain::new(vec![
    Arc::new(jwt_backend),
    Arc::new(api_key_backend),
])?;
```

The framework does not invent your JWT issuer, OAuth server, API-key database, or identity provider. The service integrates its authoritative implementation.

## 25. Permissions

Implement `PermissionBackend` or use the supplied database permission backend.

The database permission implementation includes:

- `SqlxPermissionRepository`
- `DatabasePermissionConfig`
- `DatabasePermissionBackend`

Example construction:

```rust
use lex_rest::rest_framework::permissions::{
    DatabasePermissionBackend,
    DatabasePermissionConfig,
    SqlxPermissionRepository,
};
use std::sync::Arc;

let backend = DatabasePermissionBackend::new(
    Arc::new(SqlxPermissionRepository::new(pool.clone())),
    DatabasePermissionConfig {
        global_admin_full_access: false,
        tenant_admin_full_access: false,
        allow_service_scopes: false,
        require_api_key_scope: true,
    },
);
```

Permission expressions are represented by `PermissionExpr` and are used by generic views/viewsets.

## 26. Generic API views

`GenericAPIView<T>` connects model persistence to REST policies.

It supports:

- serializer policy;
- query/filter policy;
- mutation adapter;
- access policy;
- lookup configuration;
- owner field configuration;
- object permissions;
- list/retrieve/create/update/destroy operations.

Construction starts with:

```rust
let core = GenericAPIView::<Tenant>::new(pool.clone())
    .serializer_policy(serializer)
    .query_policy(query_policy);
```

Mutation paths require a `ModelMutationAdapter<T>`. The supplied `OrmModelMutationAdapter` uses a `ModelFactory<T>` to build validated model instances for creates and updates.

The serializer layer deliberately does not hydrate sensitive/write-only plaintext directly into persistence objects. Apply password hashing, KMS encryption, token hashing, or other domain transformations in service-owned mutation code.

## 27. ModelViewSet

`ModelViewSet<T>` builds collection/detail routes around a `GenericAPIView<T>`.

```rust
let core = Arc::new(GenericAPIView::<Tenant>::new(pool.clone()));
let viewset = Arc::new(ModelViewSet::new("tenant", core));

let specs = viewset.route_specs("/api/tenants", "tenant")?;
```

You can configure:

- enabled actions;
- read/write permissions;
- throttles;
- idempotency;
- operation/OpenAPI metadata.

## 28. Routing

Routes are registered in `RouteTable`.

```rust
let mut routes = RouteTable::new();
routes.register_many(specs)?;
```

Manual HTTP route:

```rust
routes.register(RouteSpec::http(
    "/health",
    "health",
    Arc::new(health_view),
))?;
```

The route table resolves method/path pairs and can build OpenAPI schema metadata:

```rust
let schema = routes.build_openapi("Example API", "1.0.0")?;
let document = schema.document();
```

## 29. REST runtime and middleware

Build the runtime with `RestRuntimeBuilder`.

```rust
let runtime = RestRuntimeBuilder::new(Arc::new(permission_backend))
    .authentication(authentication_chain)
    .tenant_resolver(Arc::new(tenant_resolver))
    .require_secure_transport(true)
    .request_timeout(std::time::Duration::from_secs(30))?
    .build()?;

let app = RestApplication::new(routes, runtime);
```

The runtime coordinates:

- authentication;
- tenant resolution;
- authorization;
- middleware;
- request timeouts;
- secure-transport enforcement;
- throttling;
- idempotency;
- audit hooks;
- route dispatch;
- WebSocket authorization.

### Request limits

`RequestLimitsMiddleware` applies body/query/header/cardinality boundaries.

### Security headers

`SecurityHeadersMiddleware` manages security response headers including HSTS/CSP settings. HSTS is intentionally not emitted for an insecure transport request.

## 30. API request and response types

`ApiRequest` is framework-neutral and can be built by an HTTP adapter.

Useful methods include:

```text
new
with_path_params
path
body
json
query_values
query_single_map
path_param
path_params
```

Responses use `ApiResponse`:

```rust
use http::StatusCode;
use lex_rest::rest_framework::response::ApiResponse;

let response = ApiResponse::json(
    StatusCode::OK,
    &serde_json::json!({"ok": true}),
)?;
```

Other constructors include `empty` and `text`.

## 31. Idempotency

`IdempotencyPolicy` defines whether a mutating operation requires an idempotency key and how long results are retained.

The storage layer is abstracted behind `IdempotencyStore`.

This protects HTTP request replay semantics, but it is not a substitute for authoritative transactional business idempotency. Financial effects that span multiple systems should use a database uniqueness/idempotency record and/or transactional outbox pattern.

## 32. Distributed throttling

`ThrottleEngine` uses a `DistributedThrottleStore` contract. The framework does not pretend an in-process map is sufficient for horizontally scaled production throttling.

Configure policies with `ThrottlePolicy` and appropriate keys such as identity/service/API-key/tenant/IP according to your application's security model.

## 33. WebSockets

`lex_rest` exposes:

- `WebSocketView`
- `WebSocketSession`
- `SecuredWebSocketSession`
- `WebSocketSecurityPolicy`
- `WebSocketMessage`

The security policy validates the handshake and message boundaries. Production WSS termination and identity integration are supplied by the service/adapter.

## 34. Axum integration

The current workspace includes an Axum adapter at:

```text
crate/lex_rest/src/rest_framework/adapters/axum.rs
```

Use the adapter when Axum is the selected transport. Keep business authorization, serializer policy, and persistence logic inside the framework/service layers rather than reimplementing them in HTTP handlers.

## 35. Serializer policy

`db_controller::rest::SerializerPolicy::for_model::<T>()` derives default field policy from model metadata.

Important defaults:

- primary keys are read-only;
- encrypted fields are write-only;
- required non-null/no-default fields are required on create;
- relation projection fields are read-only;
- unknown/read-only/null/type-invalid input is rejected;
- model validation runs before a completed model is accepted.

Partial updates return a validated field map instead of constructing an invalid partial Rust model.

## 36. REST query policy

`RestQueryPolicy` is deny-by-default.

A service must explicitly allow:

- filter fields/lookups;
- search fields;
- ordering fields;
- pagination style.

Encrypted fields are rejected from filter/search/order policies.

Supported pagination modes are:

- page number;
- limit/offset;
- signed keyset cursor.

Signed cursors use HMAC-SHA256 and support controlled previous-key verification for rotation. Supply signing keys from your secret-management layer, never from committed source.

## 37. Secure input

`secure_input` separates input normalization/validation from output encoding.

### Validation helpers

Examples include:

```rust
use secure_input::{validate_email, validate_filename, validate_input};

validate_email("alice@example.com")?;
validate_filename("report.pdf")?;
validate_input("safe text")?;
```

The exact re-export surface is defined by `secure_input/src/lib.rs`; module paths can also be used directly.

### Sanitization

Functions include:

```text
normalize_unicode
sanitize_string
sanitize_html
collapse_whitespace
remove_special_chars
sanitize_numeric
escape_html
reject_sqli
reject_base64
reject_script
reject_dangerous_ext
enforce_length
```

Lexical SQL/XSS checks are defense-in-depth only. They do **not** replace SQL bind parameters or context-specific output encoding.

### Output encoding

Use the correct sink encoder:

- HTML whitelist/sanitization;
- JSON string escaping;
- XML text escaping;
- JavaScript string escaping;
- CSS string escaping;
- URL parameter encoding.

Never reuse HTML escaping as a universal encoder for JavaScript, CSS, XML, SQL, or URL contexts.

### Pipelines

The crate provides a composable `Pipeline` and stages for sanitizers, validators, and filters.

This is useful at API boundaries where a field needs deterministic normalization followed by validation.

## 38. Media

`media` is intentionally an integration boundary, not a fake production media framework.

The current project does **not** claim to provide authoritative implementations for:

- object storage;
- antivirus/malware scanning;
- image metadata stripping;
- transcoding;
- CDN publication.

A consuming service must provide those concrete integrations before treating media upload as production complete.

## 39. Error handling

Persistence uses `ModelError` and `RepoResult<T>`.

REST policy uses `RestError`.

`lex_rest` has its own API exception/error types.

Use stable error codes for application behavior. Do not expose raw SQL or database diagnostics to external clients.

At the transport boundary:

```text
internal error
    ↓ map
stable service error code
    ↓
public API response
```

Log enough context for operators, but never log passwords, API keys, secret plaintext, or unrestricted sensitive request bodies.

## 40. Testing

Run ordinary workspace tests:

```bash
cargo test --workspace
```

Run format/lint/build gates as configured by the repository:

```bash
lex verify framework
```

PostgreSQL integration tests are intentionally `#[ignore]` and require explicit disposable-database confirmation:

```bash
export TEST_DATABASE_URL='postgres://.../disposable_test_db'
export CONFIRM_DISPOSABLE_DATABASE=yes
lex verify postgres
```

The test suite includes behavior around:

- model derive metadata/validation;
- PostgreSQL ORM operations;
- transactions and concurrency;
- tenant scope/RLS behavior;
- REST runtime security;
- PostgreSQL-backed permissions;
- mutation rollback.

## 41. Production checklist for application developers

Before deploying a service built on the framework:

1. Generate and commit a reviewed `Cargo.lock`.
2. Run `lex verify framework` with the pinned Rust toolchain.
3. Run PostgreSQL integration tests against a disposable database.
4. Keep `DATABASE_URL` out of committed config.
5. Use TLS-verified PostgreSQL in production.
6. Disable automatic migrations unless your release process explicitly controls them.
7. Review migration UP and DOWN SQL.
8. Use tenant-scoped QuerySets for tenant-owned data.
9. Enable PostgreSQL RLS where tenant isolation is a security boundary.
10. Use `AtomicTransaction` for multi-write workflows.
11. Use row locks/version checks where concurrent mutation matters.
12. Transform sensitive serializer inputs before persistence.
13. Use distributed idempotency/throttling stores in multi-instance deployments.
14. Require secure transport in the REST runtime.
15. Configure authentication and permission backends from authoritative identity data.
16. Avoid logging raw secrets or SQL values.
17. Do not treat the media boundary as production storage until a real provider is integrated.

See `docs/OPERATIONS.md`, `docs/SECURITY.md`, and `docs/VERIFICATION.md` for the release gate.

## 42. Known deliberate gaps

The current framework does not claim complete support for:

- SQLite/MySQL backends;
- unrestricted raw SQL QuerySet escape hatches;
- every PostgreSQL expression/window/frame type;
- arbitrary-depth relation path compilation;
- implicit Django-style many-to-many mutation managers for arbitrary key types;
- a complete Django migration state optimizer/squashing engine;
- automatic primary/replica/shard/residency routing decisions;
- schema reverse engineering comparable to `inspectdb`;
- streaming/COPY ingestion;
- production media storage/scanning/transcoding without service-owned providers.

Treat these as explicit boundaries rather than silently simulating them.

## 43. Where to read next

- `docs/QUICKSTART.md` — shortest path from checkout to first model/query/API
- `docs/PROJECT_STRUCTURE.md` — crate responsibilities and dependency direction
- `docs/ORM_GUIDE.md` — model and QuerySet-focused reference
- `docs/MIGRATIONS_GUIDE.md` — migration CLI and release workflow
- `docs/REST_FRAMEWORK_GUIDE.md` — `lex_rest` application/API guide
- `docs/SECURE_INPUT_GUIDE.md` — validation/sanitization/encoding rules
- `docs/TESTING_AND_VERIFICATION.md` — local and PostgreSQL verification
- `docs/ARCHITECTURE.md` — architectural contracts
- `docs/API_CONTRACTS.md` — stable API/data contracts
- `docs/DJANGO_DRF_ALIGNMENT.md` — mapping from Django/DRF concepts
- `docs/SECURITY.md` — security controls
- `docs/OPERATIONS.md` — deployment and rollback
- `docs/VERIFICATION.md` — current verification requirements
