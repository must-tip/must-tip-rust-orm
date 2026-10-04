> **Historical documentation.** These pre-audit notes may describe obsolete APIs or ownership. Use the [current developer documentation](README.md) and executable acceptance examples for the supported contract.

# Lex CLI Guide

`lex` is the single command surface for Lex framework development and operational workflows. The framework deliberately does not require repository shell or Python scripts.

## Installation

```bash
cargo install --path crate/lex_cli --force
lex --help

# After the first installation, refresh the CLI through itself:
lex self install
```

## Command map

```text
lex create settings
lex create crate <name>
lex refresh
lex routes
lex check
lex doctor
lex audit <all|workspace|sql|source>
lex verify <all|framework|sqlite|postgres>
lex database test-postgres setup
lex database test-postgres status
lex migrate ...
lex start server
lex self install
lex cargo ...
```

### Create project settings

```bash
lex create settings --project-name marketplace
```

This creates the framework project manifest, settings, application directory, migration directory, and generated server crate. Development defaults to SQLite when no database URL is supplied.

### Create an application crate

```bash
lex create crate payments --url /api/v1/payments
```

The generated crate follows Lex conventions:

```text
apps/payments/
├── Cargo.toml
├── lex_app.toml
└── src/
    ├── lib.rs
    ├── models.rs
    ├── views.rs
    └── urls.rs
```

Its model registry and app routes are wired into the project automatically. Developers own the generated Rust source after creation; `lex refresh` will not overwrite it.

### Refresh framework wiring

```bash
lex refresh
```

Refreshes framework-owned central model registration, server application dependencies, and root URL composition.

### Routes

```bash
lex routes
```

Shows mounted application URL prefixes after refreshing the generated router.

### Check and doctor

```bash
lex check
lex doctor
```

`check` validates the Lex project structure. `doctor` additionally runs native audits and Cargo checks. Use `lex doctor --no-cargo` when only structural/source-policy diagnostics are wanted.

## Native audits

Old script-based audits are now Rust CLI features:

```bash
lex audit workspace
lex audit sql
lex audit source
lex audit all
```

`workspace` enforces workspace dependency topology and inheritance policy. `sql` checks reviewed SQLx dynamic-SQL boundaries. `source` rejects forbidden unsafe/TODO/unimplemented patterns and performs lightweight delimiter-integrity checks.

## Verification

SQLite:

```bash
lex verify sqlite
```

Complete framework release gate:

```bash
lex verify framework
```

All non-destructive default gates:

```bash
lex verify all
```

Include PostgreSQL integration tests only after configuring a disposable database:

```bash
lex database test-postgres setup
lex verify all --postgres
```

## PostgreSQL test lifecycle

```bash
lex database test-postgres setup
lex database test-postgres status
lex verify postgres
```

The setup command is intentionally defensive: it refuses protected database names, requires an obviously disposable/test name, creates a restricted login, recreates only the named test database, verifies the login, and persists its generated connection data locally under `.lex/` without printing the password.

## Migrations

```bash
lex migrate
lex migrate status
lex migrate pending
lex migrate rollback
```

The CLI forwards these operations to the framework migration engine while preserving workspace-root discovery.

## Start the server

```bash
lex start server
lex start server --release
```

The CLI refreshes framework-generated app/URL wiring before starting the configured server.

## Advanced Cargo passthrough

```bash
lex cargo test -p lex_app_payments
```

Use this for uncommon custom Cargo work. Stable framework workflows should have dedicated `lex` commands instead.

## Design rule

When a repeated framework workflow becomes necessary, implement it as an `lex` subcommand backed by Rust library code and tests. Do not introduce a new `scripts/` operational dependency.
