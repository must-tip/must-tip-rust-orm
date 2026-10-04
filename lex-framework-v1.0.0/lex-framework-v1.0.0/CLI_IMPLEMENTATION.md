# Lex CLI Architecture

This revision makes `lex` the single supported command surface for framework development and operational workflows. The repository no longer contains an operational `scripts/` directory.

## Command surface

```text
lex create settings
lex create crate <name> [--url /prefix]
lex refresh
lex routes
lex check
lex doctor [--no-cargo]
lex audit [all|workspace|sql|source]
lex verify [all|framework|sqlite|postgres] [--postgres]
lex database test-postgres setup
lex database test-postgres status
lex migrate [migration args...]
lex start server [--release]
lex self install
lex cargo [cargo args...]
```

## Why the CLI owns these workflows

Framework behavior should be versioned, testable Rust code. Shell and Python helpers previously created a second operational API with different error handling, environment assumptions, and documentation. Their behavior is now implemented under `crate/lex_cli` so the same library code can be unit tested and invoked consistently.

The CLI may launch external tools such as Cargo, PostgreSQL utilities, or systemd where those tools are the actual platform interface. Developers still invoke only `lex` for framework workflows.

## Generated architecture

```text
workspace
├── lex.toml
├── Config.toml
├── .lex/                    # ignored local CLI state/credentials
├── project/
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs
│       ├── settings.rs
│       ├── apps.rs         # generated application/model registration
│       └── urls.rs         # generated root URL composition
├── apps/
│   └── <app>/
│       ├── lex_app.toml
│       └── src/
│           ├── lib.rs
│           ├── models.rs
│           ├── views.rs
│           └── urls.rs
└── migrations/
    └── <app>/
```

## Native policy audits

`lex audit` replaces the former Python audit tools:

- `lex audit workspace` validates Cargo workspace inheritance, local dependency topology, duplicate package names, and dependency cycles.
- `lex audit sql` enforces the reviewed dynamic SQLx boundary.
- `lex audit source` checks prohibited unsafe/unfinished constructs and lightweight source delimiter integrity.
- `lex audit all` runs all three.

Python is not required for these gates.

## Native verification

`lex verify framework` replaces the former generic verification shell workflow and runs:

1. all native audits;
2. Rust/Cargo version reporting;
3. lockfile generation when absent;
4. formatting checks;
5. workspace and TLS-feature Cargo checks;
6. Clippy with warnings denied;
7. workspace, REST/TLS, and documentation tests;
8. release builds;
9. dependency duplicate inspection.

`lex verify sqlite` runs the SQLite database/ORM/migration regression gates.

`lex verify postgres` runs the ignored destructive integration tests only against an explicitly confirmed or CLI-provisioned disposable PostgreSQL target.

`lex verify all` runs framework + SQLite gates and skips PostgreSQL by default. `lex verify all --postgres` includes the configured disposable PostgreSQL test target.

## Disposable PostgreSQL lifecycle

`lex database test-postgres setup` replaces the former PostgreSQL setup shell script. It:

- validates role/database identifiers;
- refuses protected database names;
- requires the database name to clearly identify itself as test/disposable;
- verifies/starts PostgreSQL using available host service tooling;
- creates or rotates a restricted test role;
- recreates only the selected disposable database;
- configures UTC and least-privilege database/schema access;
- generates a random password when one is not supplied;
- verifies login;
- stores the local connection information in `.lex/test-postgres.toml`;
- uses mode `0600` on Unix;
- never prints the password or connection URL.

`lex verify postgres` consumes this local configuration directly; developers do not need to source an environment file.

## CLI installation lifecycle

Initial installation uses Cargo because an uninstalled CLI cannot invoke itself:

```bash
cargo install --path crate/lex_cli --force
```

Once installed, developers can refresh the workspace CLI through the command surface:

```bash
lex self install
```

No installation shell script is maintained.

## Safety rules

- project/app paths cannot escape the workspace;
- app/package names are validated;
- URL mounts reject traversal and malformed prefixes;
- duplicate mounts/packages are rejected;
- generated dependency regions use explicit markers;
- `lex refresh` rewrites generated wiring only;
- developer-owned model/view/URL source is preserved;
- generated permissions fail closed;
- production startup requires an explicit database URL and trusted TLS boundary;
- PostgreSQL destructive tests require a clearly disposable target;
- local test credentials live under ignored `.lex/`, never in committed configuration.

## Validation tests

`crate/lex_cli` tests cover scaffolding, idempotency, URL safety, developer-source preservation, duplicate mount detection, source delimiter checking, PostgreSQL identifier validation, and workspace glob expansion.

The artifact environment for this revision does not include `cargo`/`rustc`. After extraction, run:

```bash
cargo install --path crate/lex_cli --force
lex doctor
lex verify sqlite
lex verify framework
```

Configure a disposable PostgreSQL database and add `lex verify postgres` before production promotion.
