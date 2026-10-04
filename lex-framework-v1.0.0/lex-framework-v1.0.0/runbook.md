# Lex Framework Runbook

The `lex` executable is the single supported developer and operations interface for framework workflows. Repository shell/Python scripts are intentionally not part of the framework contract.

## Install the CLI

From the framework source tree:

```bash
cargo install --path crate/lex_cli --force
lex --help

# After the first installation, refresh the CLI through itself:
lex self install
```

A published release can later be installed with `cargo install lex_cli` once the crate is published.

## Create or repair project scaffolding

```bash
lex create settings --project-name my_platform
lex create crate accounts --url /api/v1/accounts
lex create crate billing --url /api/v1/billing
lex refresh
lex check
```

`lex refresh` regenerates only framework-owned app/URL wiring. It does not overwrite developer-owned model, view, or URL source files.

## Run locally

SQLite is the development default when `DATABASE_URL` is not configured:

```bash
lex migrate
lex start server
```

Use PostgreSQL explicitly when required:

```bash
export DATABASE_URL='postgresql://USER:PASSWORD@HOST:5432/DATABASE'
lex migrate
lex start server
```

## Audits

```bash
lex audit all
lex audit workspace
lex audit sql
lex audit source
```

These checks are implemented inside the Rust CLI. Python is not required.

## Structural and compiler diagnostics

```bash
lex check
lex doctor
```

`lex doctor --no-cargo` runs structural and native source-policy validation only. `lex doctor` additionally runs `cargo check --workspace --all-targets`.

## SQLite verification

```bash
lex verify sqlite
```

This runs the SQLite database, ORM, and migration regression gates.

## Framework release verification

```bash
lex verify framework
```

This runs framework-owned audits plus formatting, Cargo checks, Clippy, workspace tests, documentation tests, release builds, TLS-feature verification, and dependency-duplicate inspection.

## Disposable PostgreSQL integration database

Never run destructive integration tests against production.

Create a dedicated test database:

```bash
lex database test-postgres setup
```

Custom values:

```bash
lex database test-postgres setup \
  --database lex_framework_test \
  --user lex_test_user \
  --host 127.0.0.1 \
  --port 5432
```

`lex` validates that the database name is clearly disposable, refuses protected PostgreSQL databases, creates/rotates a restricted role, recreates the test database, verifies login, and stores the generated connection information in `.lex/test-postgres.toml` with mode `0600` on Unix. The CLI does not print the password.

Inspect non-secret metadata:

```bash
lex database test-postgres status
```

Then run PostgreSQL integration verification:

```bash
lex verify postgres
```

Or provide an external disposable target explicitly:

```bash
export TEST_DATABASE_URL='postgresql://.../clearly_disposable_test_db'
export CONFIRM_DISPOSABLE_DATABASE=yes
lex verify postgres
```

## Full verification

```bash
lex verify all
```

By default this performs framework + SQLite verification and skips destructive PostgreSQL tests. To include the configured disposable PostgreSQL integration database:

```bash
lex verify all --postgres
```

## Migrations

```bash
lex migrate
lex migrate status
lex migrate pending
lex migrate rollback
```

Arguments after `lex migrate` are passed to the framework migration engine; developers never need to invoke the migration binary directly.

## Advanced Cargo escape hatch

Framework workflows should use dedicated `lex` commands. For unusual development work, the CLI offers a workspace-root-aware Cargo passthrough:

```bash
lex cargo test -p my_crate
lex cargo tree --duplicates --locked
```

This is an escape hatch, not a replacement for `lex verify`, `lex doctor`, or framework-specific commands.

## Production startup

Production must explicitly configure its database and trusted TLS boundary. Do not rely on the development SQLite fallback.

Typical sequence:

```bash
lex check
lex verify framework
lex migrate
APP_PROFILE=production LEX_TRUSTED_TLS_TERMINATION=true lex start server --release
```

Only set `LEX_TRUSTED_TLS_TERMINATION=true` when the process is actually behind a trusted TLS ingress/proxy or another reviewed TLS termination boundary.
