# Installation

Lex Framework v1.0.0 is distributed as a Rust workspace today, with `lex_framework` as the application-facing facade and `lex_cli` as the developer CLI.

## Requirements

- Rust toolchain pinned by the repository's `rust-toolchain.toml`.
- Cargo.
- SQLite for the default development path, or PostgreSQL when you select PostgreSQL.

## Install the CLI

```bash
cargo install --path crate/lex_cli --force
```

Verify it:

```bash
lex --help
lex doctor
```

## Use the framework facade

Application crates should prefer the facade crate instead of importing internal crates directly:

```rust
use lex_framework::prelude::*;
```

The framework facade exposes the supported application-facing surface for views, permissions, authentication services, settings, ORM integration, media, and other framework facilities.

## Create a project

```bash
lex create settings --project-name marketplace
lex create crate accounts --url /api/v1/accounts
lex create crate products --url /api/v1/products
lex start server
```

Development defaults can use SQLite automatically. Production is intentionally explicit: configure the production database and security settings instead of relying on development fallbacks.
