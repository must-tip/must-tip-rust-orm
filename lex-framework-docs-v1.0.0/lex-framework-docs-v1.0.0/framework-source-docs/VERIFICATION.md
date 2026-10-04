# Verification Status

## Environment limitation

The repair environment does not provide `rustc`, `cargo`, or `rustfmt`, and its network policy blocks downloading a Rust toolchain. Compiler-backed statements are therefore intentionally not presented as completed evidence.

## Static verification performed

The release preparation process performs source/manifests checks that do not require Rust tooling:

- parse the root and member Cargo manifests, `rust-toolchain.toml`, and `Config.toml` as TOML;
- verify every workspace member manifest and local path dependency;
- verify the local workspace dependency graph is acyclic;
- verify member crates inherit direct third-party versions from `[workspace.dependencies]`;
- audit SQLx runtime SQL callsites so dynamic SQL crosses only the reviewed `AssertSqlSafe` boundary;
- scan for byte-identical Rust source files;
- scan for unfinished/unsafe patterns prohibited by project policy;
- syntax-balance Rust delimiters with a lexer that ignores strings/comments;
- count unit/integration tests and inspect guarded database integration coverage.

Static checks do not substitute for the Rust compiler.

## Required compiler-backed gate

Run from the workspace root:

```bash
cargo generate-lockfile
lex verify framework
```

``lex verify framework`` requires:

1. `cargo fmt --all -- --check`
2. `cargo check --workspace --all-targets --locked`
3. `cargo clippy --workspace --all-targets --locked -- -D warnings`
4. `cargo test --workspace --all-targets --locked`
5. `cargo test --workspace --doc --locked`
6. `cargo build --release --workspace --locked`
7. `cargo tree --duplicates --locked` output review

The generated `Cargo.lock` must be reviewed and committed before production.

## Required PostgreSQL gate

Use a disposable PostgreSQL database only:

```bash
export TEST_DATABASE_URL='postgres://.../disposable_test_db'
export CONFIRM_DISPOSABLE_DATABASE=yes
lex verify postgres
```

The integration suite is ignored during ordinary test runs and becomes destructive only after the confirmation environment variable is set. It covers QuerySet CRUD, tenant isolation, typed related EXISTS/NOT EXISTS filtering, relation loading/soft-delete isolation, expression and bulk updates, optimistic concurrency rejection, unique-key upsert behavior, projections, row locking inside `AtomicTransaction`, nested savepoint/on-commit rollback behavior, transactional `Tracked<T>::save()`, refresh/latest loading, soft-delete visibility, the database-backed `lex_rest` role/permission resolver, and rollback of a REST model update when post-update model validation fails.

## Additional production gates

Before production promotion also require, in the deployment organization:

- dependency vulnerability/license/source policy scanning of the reviewed lockfile;
- migration dry-run/plan and rollback review on production-equivalent PostgreSQL;
- RLS tests using the same least-privilege database role used by application traffic;
- service adapter authentication/authorization/error-contract tests;
- load/soak testing for pool saturation, long transactions, lock contention, pagination, and bulk paths;
- backup/restore and disaster-recovery evidence;
- observability/alert validation and secret-redaction review.

Do not promote the archive solely on the static checks recorded here.

## lex_rest verification

``lex verify framework`` separately checks, lints, tests and release-builds `lex_rest` with `axum-rustls` enabled in addition to the default workspace gate. REST tests cover request redaction, permission composition, API-key scope caps, tenant object boundaries, trusted-peer throttle requirements, distributed throttle behavior, idempotency fingerprints, HTTP/WebSocket route traversal/method handling (including GET/CONNECT upgrades), OpenAPI metadata, WSS origin policy and end-to-end runtime security/error behavior. Compiler-backed execution remains mandatory before release.
