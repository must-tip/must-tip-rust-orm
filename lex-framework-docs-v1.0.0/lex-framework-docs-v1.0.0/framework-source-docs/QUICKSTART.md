# Quick Start

This is the shortest supported path for a developer to use the current framework.

## 1. Install the pinned Rust toolchain

```bash
rustup show
```

The repository pins Rust 1.97.1 in `rust-toolchain.toml`.

## 2. Generate/verify the lockfile

```bash
cargo generate-lockfile
cargo check --workspace
```

## 3. Choose a database

For local development, no database configuration is required. If `DATABASE_URL` is absent, the framework uses and creates:

```text
sqlite://db.sqlite3
```

To use PostgreSQL instead:

```bash
export DATABASE_URL='postgres://user:password@127.0.0.1:5432/my_app'
```

For production, set `APP_PROFILE=production`; the framework then requires an explicit `DATABASE_URL` and will not silently create SQLite.

## 4. Initialize the database runtime

```rust
use db::{Database, DatabaseBackend};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let database = Database::initialize().await?;
    database.health_check_all().await?;

    match database.backend() {
        DatabaseBackend::Sqlite => {
            let pool = database.sqlite_pool().expect("SQLite pool").clone();
            // Fresh-development helper; creates the model table/indexes if missing.
            // let customers = Customer::objects_sqlite_auto(pool).await?;
        }
        DatabaseBackend::Postgres => {
            let pool = database.postgres_write_pool().expect("PostgreSQL pool").clone();
            // let customers = Customer::objects(pool);
        }
        DatabaseBackend::Auto => unreachable!(),
    }

    database.close().await;
    Ok(())
}
```

## 5. Define a model

```rust
use db_controller::{BooleanField, DateTimeField, TextField, UUIDField};
use orm::Model;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "customers")]
#[timestamps]
#[model(ordering = "-created_at")]
struct Customer {
    #[field(primary_key)]
    id: UUIDField,

    #[field(not_empty, max_length = 120)]
    name: TextField,

    #[field(unique, index, not_empty, max_length = 255)]
    email: TextField,

    #[field(default = false)]
    verified: BooleanField,

    #[field(soft_delete)]
    deleted_at: Option<DateTimeField>,
}
```

## 6. Validate it

```rust
let customer = Customer {
    id: UUIDField::new(),
    name: TextField::new("Alice"),
    email: TextField::new("alice@example.com"),
    verified: BooleanField::new(false),
    deleted_at: None,
};

customer.full_clean()?;
```

## 7. Create a row

```rust
// SQLite development:
let manager = Customer::objects_sqlite_auto(sqlite_pool.clone()).await?;
let stored = manager.queryset().create(&customer).await?;

// PostgreSQL:
// let stored = Customer::objects(pg_pool.clone()).queryset().create(&customer).await?;
```

## 8. Query rows

```rust
let rows = Customer::objects_sqlite(sqlite_pool.clone())
    .queryset()
    .filter(Customer::EMAIL.icontains("@example.com"))
    .filter(Customer::VERIFIED.eq(true))
    .order_by(Customer::EMAIL.asc())
    .limit(50)
    .all()
    .await?;
```

## 9. Use a transaction

```rust
use db_controller::{AtomicTransaction, TransactionOptions};

// PostgreSQL: AtomicTransaction
// SQLite: SqliteAtomicTransaction
let mut tx = db_controller::SqliteAtomicTransaction::begin(
    &sqlite_pool,
    TransactionOptions::default(),
).await?;

let customer = tx
    .queryset::<Customer>()
    .filter(Customer::EMAIL.eq("alice@example.com"))
    .get()
    .await?;

// write more state in the same transaction

tx.commit().await?;
```

## 10. Run migrations

```bash
cargo run -p migration --bin migration-cli -- \
  --db-url "${DATABASE_URL:-sqlite://db.sqlite3}" \
  --dir ./migrations \
  status

cargo run -p migration --bin migration-cli -- \
  --db-url "${DATABASE_URL:-sqlite://db.sqlite3}" \
  --dir ./migrations \
  migrate --dry-run
```

Review the dry-run output before applying migrations.

## 11. Run verification

```bash
lex verify framework
```

For destructive database integration tests:

```bash
export TEST_DATABASE_URL='postgres://.../disposable_test_db'
export CONFIRM_DISPOSABLE_DATABASE=yes
lex verify postgres
```

## Next

Read `docs/DEVELOPER_GUIDE.md` for the complete framework manual and `docs/SQLITE_GUIDE.md` for backend-specific SQLite behavior.
