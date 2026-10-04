# lex-framework

Lex 1.0.0 release candidate provides a Rust application facade for models, database queries, migrations, authentication, tenant permissions, REST composition and media policies. Requires Rust 1.97.1. Licensed under MIT OR Apache-2.0.

The Cargo package is `lex-framework`; the Rust import is `lex_framework`. The default feature set is empty; enable `axum` for HTTP or `axum-rustls` for the optional TLS adapter. PostgreSQL and SQLite support currently compile together.

After publication:

```toml
[dependencies]
lex-framework = { version = "1.0", features = ["axum"] }
serde = { version = "1", features = ["derive"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

The registry release and hosted Git identity are not yet certified. Checkout users can depend on this crate by absolute path; Git users must use the verified repository URL with an immutable revision or release tag. Application code is identical across those sources.

This model example is executed by the independent consumer acceptance suite:

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

Automatic table creation is a development convenience, not schema migration. In-memory SQLite needs an appropriate connection strategy; this example uses one connection. A global model deliberately has no tenant boundary. Tenant models require explicit server-verified context. Production applications must configure authentication, write-field policy, constrained RBAC migrations, transport and durable idempotency/throttle providers as appropriate. See the repository guides and rustdoc; S3 runtime and complete WebSocket acceptance are not certified.
