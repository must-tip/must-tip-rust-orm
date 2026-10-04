# Release hardening work log

Status: the recorded hardening and validation pass is complete; the full release objective remains incomplete. See REPORT.md sections M–N for remaining work and release blockers. This document preserves the initial implementation plan and is not release approval.

## Source baseline

The supplied tree is an extracted archive, not a Git checkout. No canonical remote or
release tag can be inferred. There are ten framework crates and one unpublished server.
All packages declare 1.0.0, edition 2024, and Rust 1.97.1. The toolchain file pins
1.97.1; this is also the declared MSRV. There is no lockfile or CI workflow in the archive.

Dependency order (normal dependencies):

1. `db_controller`, `orm`, `secure_input`, `media`, `lex_cli`
2. `models` -> `orm`, `db_controller`
3. `migration` -> `models`, `db_controller`
4. `db` -> `migration`
5. `lex_rest` -> `models`, `db_controller`
6. `lex_framework` -> database, ORM, migration, REST, input, media
7. `lex_framework_server` (not publishable)

`orm` also uses `db_controller` as a development dependency. SQLx always enables both
database backends. Only `lex_rest` exposes optional transport features: `axum` and
`axum-rustls`. The facade does not yet forward those features.

## Architecture and observed gaps

- `Entity` metadata drives ORM queries and schema generation. `Model` generates entity
  metadata and SQLx decoding, but generated paths assume direct consumer dependencies.
- `query.rs` mixes AST, execution, dialect translation, query planning and SQL compilation
  across 4,463 lines. `rest.rs` mixes serializers, cursor signing and query policy across
  1,582 lines. Preserve their public paths during refactoring.
- `db` owns connection configuration and replica selection; `AtomicTransaction` owns
  PostgreSQL transaction-local RLS context and post-commit callbacks.
- `lex_rest` owns HTTP-independent runtime policy; the Axum adapter is the actual HTTP
  boundary. Existing runtime security integration tests call the runtime directly.
- `lex_framework` constructs database-backed session/bearer authentication and signed
  cookies. The application still owns identity verification before issuing credentials.
- RBAC grants currently combine membership, role and permission in one table. Membership
  and role uniqueness and cross-tenant integrity need database constraints and migration.
- CLI scaffolding currently requires the framework source layout, not a clean consumer.
- Media filesystem writes share temporary filenames and confinement is lexical only.
- Documentation is duplicated in `docs`, `docs-site` and the separate docs archive;
  historical implementation notes are not execution evidence.

## Incremental implementation sequence

1. Capture toolchain, formatting, compilation, Clippy, tests and rustdoc baseline logs.
2. Resolve compilation failures before structural changes. Run existing SQLite and guarded
   PostgreSQL suites against disposable databases. Record default and transport features.
3. Add behavioral regressions and fix auth composition, configuration, media, tenant and
   write-policy findings. Normalize RBAC with a data-preserving, fail-closed migration.
4. Separate AST, executors, compilation and query planning; separate serialization,
   filtering/pagination and cursor policy. Keep public re-exports compatible.
5. Add real Axum HTTP tests, macro consumer checks, independent application tests and
   generated project compilation. Test supported feature combinations.
6. Prepare versioned internal dependencies, package metadata/assets, publication graph,
   facade ergonomics and Git/registry consumer parity. Never fabricate a remote URL.
7. Consolidate developer docs, executable quickstart, backend capability/test matrix,
   contributor instructions, deployment assumptions, changelog and CI release gates.
8. Repeat the complete validation and produce the requested A–N release report with
   explicit executed, failed, blocked and unimplemented statuses.

## Evidence

Baseline command results live in `baseline.json` with per-command logs. Required tests
must remain enabled; existing opt-in destructive PostgreSQL tests must explicitly run
with a disposable database. No publication or upstream push is authorized.
