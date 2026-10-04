# Your first model and HTTP API

Use Rust 1.97.1. From this checkout, run the independent examples:

```sh
cargo run --manifest-path integration/consumer/Cargo.toml --example first_model
cargo run --manifest-path integration/consumer/Cargo.toml --example first_api
```

The second command listens on http://127.0.0.1:8080. In another terminal:

```sh
curl -i http://127.0.0.1:8080/notes
curl -i -H 'Content-Type: application/json' -d '{"title":"Hello 世界"}' http://127.0.0.1:8080/notes
curl -i http://127.0.0.1:8080/notes
```

GET returns an array, POST returns the created note with status 201, and a client-supplied id returns 400. The example creates its own in-memory SQLite database and discards it on exit. It refuses the production profile and binds loopback. Its public permission is deliberate: add authentication and trusted tenant resolution before exposing an application.

To copy the model example into a new `cargo new` application, use this dependency set. Replace the path with this checkout's absolute directory; see [installation](installation.md) for verified Git and future registry declarations. Only the Lex dependency source changes between those forms.

```toml
[dependencies]
lex-framework = { path = "/ABSOLUTE/LEX/CHECKOUT/crate/lex_framework" }
serde = { version = "1", features = ["derive"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

The precise local dependency path is `/ABSOLUTE/LEX/CHECKOUT/crate/lex_framework`. Put the following tested source in `src/main.rs`:

```rust
use lex_framework::{
    db_controller::{TextField, UUIDField, sqlx},
    prelude::*,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "notes")]
#[model(global)]
struct Note {
    #[field(primary_key)]
    id: UUIDField,
    title: TextField,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    let notes = Note::objects_sqlite_auto(pool).await?;
    let saved = notes
        .queryset()
        .create(&Note {
            id: UUIDField::new(),
            title: TextField::new("Hello 世界"),
        })
        .await?;
    let loaded = notes.queryset().filter(Note::ID.eq(saved.id)).get().await?;
    assert_eq!(loaded.title.as_str(), "Hello 世界");
    Ok(())
}
```

`#[model(global)]` declares deliberate absence of tenant scope. Tenant-owned models must instead declare their tenant field and receive a server-verified TenantContext. A one-connection pool keeps this in-memory example on the same database. `objects_sqlite_auto` creates missing tables for development; it does not upgrade existing schema. Use reviewed [SQL-file migrations](../concepts/migrations.md) for persistent deployments.

The complete [HTTP example](../../integration/consumer/examples/first_api.rs) shows public APIView, Serializer write allowlists, RouteTable, middleware and Axum composition without internal crate imports. `python3 integration/verify_examples.py` copies both examples outside the workspace and exercises real TCP HTTP requests. The [tenant acceptance API](../../integration/consumer/tests/tenant_http.rs) adds bearer authentication, verified membership and normalized database RBAC, including denial after role removal.

Generate a standalone project and app:

```sh
cargo install --path crate/lex_cli --locked
lex create project my_api --framework-path "$PWD"
cd my_api
lex create crate projects
lex refresh
lex check
cargo check --workspace --all-targets
lex start server
```

`/health` is explicitly public. Model registration does not create tables. The generated `lex-migrate` runner uses the supported SQL-file migration system. See [CLI](../reference/cli.md), [models and queries](../concepts/models-and-queries.md), [transactions](../concepts/transactions.md), [RBAC](../guides/rbac.md) and [deployment](../operations/deployment.md) next. `python3 integration/verify_generated.py --postgres` exercises generated application compilation, migrations, queries and security tests on the disposable backends. Protected HTTP CRUD on both backends is exercised by `integration/consumer/tests/protected_crud.rs`, including cross-tenant denial and revocation. Live WebSocket acceptance remains a coverage gap.
