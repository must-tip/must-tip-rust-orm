# Quick start

## Create a project configuration

```bash
lex create settings --project-name shop
```

## Create an application crate

```bash
lex create crate products --url /api/v1/products
```

The generated app contains:

```text
apps/products/
├── Cargo.toml
├── lex_app.toml
└── src/
    ├── lib.rs
    ├── models.rs
    ├── views.rs
    └── urls.rs
```

## Start the server

```bash
lex start server
```

Development defaults to SQLite when no explicit database URL is supplied.

## Validate the project

```bash
lex check
lex doctor
lex verify sqlite
```

## Common imports

Application code should favor the facade crate:

```rust
use lex_framework::prelude::*;
```

For views:

```rust
use lex_framework::{
    permissions::{Permissions, ViewPermissions},
    views::{ApiRequest, ApiResponse, ApiResult, APIView, RequestContext, ViewServices},
};
```

This shields application code from internal crate reorganization.
