> **Historical documentation.** These pre-audit notes may describe obsolete APIs or ownership. Use the [current developer documentation](README.md) and executable acceptance examples for the supported contract.

# Testing and Verification

## Fast local gate

```bash
cargo check --workspace
cargo test --workspace
lex verify framework
```

## PostgreSQL integration gate

Use only an empty/disposable database:

```bash
export TEST_DATABASE_URL='postgres://USER:PASSWORD@HOST:5432/disposable_test_db'
export CONFIRM_DISPOSABLE_DATABASE=yes
lex verify postgres
```

The confirmation variable is intentionally required because integration tests drop/create tables.

## What the current tests cover

- model macro metadata and validation;
- QuerySet SQL/persistence behavior;
- PostgreSQL CRUD/concurrency behavior;
- tenant/RLS contracts;
- transaction behavior;
- REST runtime security headers and transport enforcement;
- permission database integrity;
- mutation rollback on validation failure.

## Release verification

Before a production release:

1. run with the pinned toolchain;
2. ensure `Cargo.lock` is reviewed and committed;
3. pass workspace tests and lints;
4. pass PostgreSQL integration tests;
5. review migrations and dry-run output;
6. validate production configuration/TLS;
7. run service-specific integration/e2e tests;
8. verify monitoring/rollback procedures.

See `docs/VERIFICATION.md` for the authoritative project release gate.
