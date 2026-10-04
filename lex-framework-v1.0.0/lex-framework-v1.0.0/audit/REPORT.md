# Lex production hardening and release audit

**Status: hardening implemented; release approval remains withheld.** This report distinguishes executed checks, source review, unsupported behavior and remaining coverage. It does not certify the user's entire definition of done. The supplied tree is an extracted distribution without Git provenance or a canonical hosted URL. No crates were uploaded.

## A. Architecture

Ten publishable framework packages and one unpublished server form the workspace. All use version 1.0.0, edition 2024, and Rust 1.97.1. The pin and declared MSRV are unchanged. The main Cargo dependency is lex-framework; application imports use lex_framework. The facade forwards axum and axum-rustls; both SQLx databases are compiled together. cookie-integration and postgres-integration are explicit test gates, not backend product features.

The controller owns typed fields, Entity metadata, QuerySet execution, transaction primitives and database REST policies. The Model derive generates metadata/decoding. Built-in models own identities; migration owns reviewed SQL-file execution and typed planning. db owns settings/pools/replica selection. lex-rest owns request authentication, permissions, views, routing and optional Axum. The facade constructs configured auth/permission services. Media and secure input expose provider/context boundaries. lex-cli embeds independent project/app scaffolding; generated workspaces consume public packages rather than framework internals.

See [the architecture guide](../docs/concepts/architecture.md), [initial inventory](inventory.json) and [baseline plan](PLAN.md). The initial plan intentionally describes the source before fixes.

## B. Code changes

- `crate/db_controller/src/query/` and `rest/`: responsibility-based modules, preserved public reexports, scoped mutation safety, exact eager relation hydration, atomic SQLite bulk operations and documented terminal behavior. SQLite exact-decimal numeric operations return explicit unsupported errors instead of lexical comparisons/rounded arithmetic.
- `crate/db_controller/src/model_state.rs`: shared tracked persistence with SQLite save support, dirty/no-op behavior, protected identity fields and stale-version conflicts.
- `crate/orm/src/lib.rs`: facade/direct/renamed dependency resolution, correct qualified trait expansion, relation/composite metadata generation, qualified nullable types, UUID/decimal SQLite decoding and user-facing diagnostics.
- `crate/models/src/{user,rbac}.rs`, `crate/migration/src/rbac.rs`, `crate/lex_rest/src/rest_framework/permissions/normalized.rs`: normalized entities, constrained install/conservative upgrade, live primary authorization.
- `crate/lex_rest/src/rest_framework/`: fail-closed credential composition, CORS validation/preflight/Vary behavior, HTTP error redaction/content type, serializer write policies and concurrent idempotency Retry-After.
- `crate/lex_framework/src/{lib,auth_runtime}.rs`: validated issuance/TTL, random opaque credentials, secret redaction, feature forwarding and normalized backend composition.
- `crate/db/src/{config,db_config}.rs`: common production-profile detection, secret-safe diagnostics, every-connection SQLite foreign keys, safe memory pool sizing and explicit primary/replica behavior.
- `crate/media/src/`: directory-capability filesystem access, unique atomic upload temporaries, bounded fail-closed scanner protocol.
- `crate/secure_input/src/`: contextual JavaScript/JSON/XML encoding, Unicode-safe rules and corrected behavior tests.
- `crate/lex_cli/src/`, `project/src/bin/lex-migrate.rs`: standalone source selection, parsed TOML editing, UTF-8-safe audit, generated migration runner and non-conflicting DB URL flag.
- All manifests/readmes/licenses, Cargo.lock, toolchain, Compose, CI, docs and independent acceptance runners were added or aligned. Compilation blockers were fixed before structural work.

These changes are reviewed through public behavior tests, not only compiler snapshots. No meaningful failing test was removed or ignored to obtain passing results.

## C. RBAC changes

Previously TenantRolePermission attached a permission and role directly to a membership. Normalized state separates MembershipRole assignments from RolePermission grants. PermissionDefinition retains its existing unique `name` as code for source compatibility. A role belongs to a tenant, and a membership belongs to one user/tenant pair.

Database uniqueness enforces membership(user,tenant), role(tenant,name), assignment(membership,role), grant(role,permission), and permission name. Composite membership/role `(id,tenant_id)` keys let bridge foreign keys enforce tenant consistency. RESTRICT prevents identity/role/permission deletion from silently erasing referenced assignments or grants. Revoke explicitly first. Lookup indexes and constraint names are deterministic.

Fresh PostgreSQL and SQLite installations reject duplicates, invalid references, tenant-crossed assignments and unsafe parent changes. PostgreSQL installation validates the definition of named foreign keys; a same-name weaker constraint is refused. SQLite checks actual composite PRAGMA shapes and requires FK enforcement. Both installers verify complete, nonpartial structural unique keys independently of index names and reject existing bridges/grants that lost uniqueness.

The PostgreSQL conversion locks source authorization tables, rejects invalid/cross-tenant source grants and differing per-member permission sets for the same role, deduplicates assignment/grant pairs and retains source rows. It refuses merging into populated normalized state. This avoids escalating privileges by taking a union of inconsistent old grants. Application configuration must switch only after reviewed migration. SQLite legacy table rebuild/conversion remains unsupported; fresh constrained installation is supported. See [upgrade instructions](../docs/guides/rbac.md).

## D. Query refactor

The original 4,463-line query module became `mod`, `value`, `predicate`, `ordering`, `policy`, `plan`, `executor`, `postgres`, `sqlite`, `queryset`, `compiler`, `mutation`, `mutation_support`, `relations`, `sqlite_decimal` and `tests`. Representation, SQL compilation, dialect execution and mutation policy have separate responsibilities. Existing db_controller::query public paths remain reexported. The legacy builder remains an advanced interface and does not automatically inherit model tenancy.

The sqlite_decimal policy isolates unsupported numeric operations while preserving exact storage, assignment, null checks and counts. Decimal unique/relation keys and distinct projections are explicitly rejected. Automatic model initialization refuses FK-disabled custom pools. SQLite bulk create/update now execute as single JSON-backed statements, preventing partial persistence when later rows fail. Shared tracked saves preserve transaction executors. Eager relation JSON preserves exact decimals, SQLite booleans and JSON types; binary hydration returns an explicit unsupported error directing callers to prefetch. Typed CASE/window/annotation/subquery APIs were not invented or falsely advertised. Some dialect behavior still uses compatibility translation, so the refactor is not a wholesale replacement compiler.

## E. REST refactor

The original 1,582-line database REST module became `mod`, `error`, `policy`, `serializer`, `filtering`, `cursor`, `query`, `ordering` and `tests`. Public imports remain compatible. Transport/security responsibilities remain in lex-rest, separate from database policy.

Common identity, ownership and privilege fields default read-only; explicit write allowlists are the recommended contract. Models do not imply client write permission. Wrapped database/internal errors are redacted while not-found/conflict statuses remain meaningful. The stable error body is code/detail/request_id with optional fields. SQLite supports custom APIViews using its public QuerySet; the built-in database mutation adapter remains PostgreSQL-specific.

## F. Test coverage

[The full matrix](TEST_MATRIX.md) lists backend, unit, integration, external-consumer, HTTP and documentation evidence with limitations. Important executed stories include:

- Portable queries, hostile bound values, tenant CRUD, soft deletion, exact nullable decimals and stale tracked editors on both databases.
- Protected external HTTP create/retrieve/PUT/PATCH/delete on both databases, forged tenant payload rejection, cross-tenant 404s and live role removal using the same token.
- Actual RBAC duplicate/FK/delete/tenant constraint rejection, legacy conversion preservation/refusal and same-name constraint corruption detection.
- Forced PostgreSQL RLS using a restricted role; missing tenant, wrong tenant, commit/rollback and reused connection state.
- Session/bearer expiry/revocation/scope/CSRF/composition; signed-cookie tampering and CSRF through the router.
- Concurrent idempotency retry, replay/conflict/expiry; per-user throttling/reset/backend failure; CORS and exact request limits.
- File-backed SQLite persistence, rollback/savepoints and simultaneously held pooled connections rejecting orphan references.
- Macro compile-fail fixtures plus default direct, renamed direct and renamed-facade consumer compilation/runtime metadata.
- Filesystem symlink/traversal/collision/concurrent upload, strict scanner responses and contextual output encoding.
- CLI-generated projects, actual migrations, independent application tests and live TCP generated/quickstart servers.

Existing ignored PostgreSQL tests remain ignored by default for environment isolation but are explicitly selected in the release run. Passing compiler/policy units are not presented as proof of real database/server behavior.

## G. Git installation

The independent runner builds against a disposable local Git repository with these exact declaration forms (URL/revision values are printed in its log):

```toml
lex-framework = { package = "lex-framework", git = "file:///tmp/LEX_SNAPSHOT/repository", branch = "main", features = ["axum"] }
lex-framework = { package = "lex-framework", git = "file:///tmp/LEX_SNAPSHOT/repository", rev = "SNAPSHOT_COMMIT", features = ["axum"] }
lex-framework = { package = "lex-framework", git = "file:///tmp/LEX_SNAPSHOT/repository", tag = "v1.0.0", features = ["axum"] }
```

These are fixture forms; no real GitHub URL/tag can be claimed from this archive. For an actual hosted release, provide the canonical URL and run `python3 integration/verify_consumer.py --git URL --tag TAG` or `--rev COMMIT`. Production should pin a real tag or immutable revision. Branch main is development-only. The facade-only application source stays the same across dependency sources.

## H. crates.io readiness

Generic packages db/media/migration/models/orm were already unrelated registry entries. Distribution names now consistently use lex- prefixes while dependency aliases preserve Rust imports. Read-only registry checks found all ten proposed prefixed names absent on 2026-10-04; absence is neither reservation nor ownership.

Publish order, including dev dependencies needed for verification:

1. lex-db-controller, lex-media, lex-secure-input, lex-cli
2. lex-orm
3. lex-models
4. lex-migration
5. lex-db and lex-rest
6. lex-framework

Manifests include versions, edition/MSRV, license, description, readme, documentation URL, keywords/categories and exact internal path+version/package dependencies. Repository/homepage metadata awaits a verified canonical URL. All ten are packaged and compiled together using Cargo workspace staging; publish dry runs do not upload. The package verifier checks readmes/licenses/normalized dependencies and builds an external version-based consumer with local registry patches. That proves archive/API layout, not registry downloads or maintainer ownership. No `.cargo_vcs_info.json` provenance is claimed without a source Git checkout. `project` and acceptance apps are unpublished.

## I. Documentation

New/current material covers installation lifecycle, copyable executed model/API examples, architecture/imports, models/queries, transactions, primary migrations, normalized RBAC/upgrades, authentication, contextual security, backend differences, settings/features, CLI, deployment, contribution and release gates. Root/package READMEs and CHANGELOG explain release-candidate status and compatibility. Historical uppercase guides and old website content are marked historical; current site pages mirror the audited guides. Important examples execute outside the workspace and over live HTTP.

Strict rustdoc and the docs site build are separate gates. Rustdoc's clean build does not prove every public API has complete prose or every historical snippet compiles. Remaining public documentation coverage is explicitly recorded. The Vite build reports a large-chunk performance warning rather than a failed build.

## J. Security

Remediations include invalid-credential downgrade prevention, ambiguous credential rejection, tenant object fail-closed checks, normalized composite tenant FKs, safe legacy privilege conversion, no leaked pooled RLS tenant state, every-connection SQLite FKs, atomic batches, serializer mass-assignment controls, safe transport defaults, CORS preflight validation, request limits, error/Debug/URL redaction, unpredictable opaque credentials, filesystem capability confinement, strict scanner responses and inline-script/XML encoding.

Regression failures are retained in before logs. Raw tokens are not stored. Cookie payloads are authenticated rather than encrypted and individual stateless cookies cannot be database-revoked. Trusted TLS termination remains an operator topology assertion. No live production deployment or complete side-channel/logging security certification is claimed. Dependency advisories are checked against the recorded database version, not assumed permanently absent.

## K. Compatibility

Internal module moves retain public reexports. Old Rust imports can be retained with dependency aliases after the distribution-name change. Direct default package keys normalize to lex_db_controller/lex_orm; the derive now resolves actual consumer names. Facade-only application code is unchanged between tested source modes.

Intentional behavior changes: configured database authorization now uses normalized state; ambiguous credentials reject; supplied invalid credentials never silently become anonymous; Axum starts insecure unless explicitly marked; security-sensitive serializer fields default read-only; unsafe CORS settings fail validation; disabled SQLite foreign keys are rejected for managed pools; CLI DB URL shorthand changes from conflicting -d to -u. SQLite decimal comparisons/order/arithmetic/non-count aggregates, uniqueness/relation keys and distinct projections now fail explicitly; use the documented portable alternatives. Binary eager hydration fails explicitly instead of returning malformed values. See CHANGELOG and RBAC upgrade guide before adopting these changes.

## L. Commands executed

The command table below is generated from `release-results.json` after the final run. Every command has a separate log. Disposable database and cookie test variables are documented in CONTRIBUTING. Pinned/MSRV 1.97.1 and stable 1.99.0 are tested separately; PostgreSQL is version 16. Additional before/after threat regressions and docs-site/npm logs remain under audit.

<!-- COMMAND_RESULTS -->
| Check | Exact command | Exit | Execution log |
|---|---|---|---|
| toolchain | `rustup show` | 0 | [log](release-toolchain.log) |
| rust-version | `rustc --version` | 0 | [log](release-rust-version.log) |
| cargo-version | `cargo --version` | 0 | [log](release-cargo-version.log) |
| format | `cargo fmt --all -- --check` | 0 | [log](release-format.log) |
| consumer-format | `cargo fmt --manifest-path integration/consumer/Cargo.toml -- --check` | 0 | [log](release-consumer-format.log) |
| check | `cargo check --workspace --all-targets --all-features --locked` | 0 | [log](release-check.log) |
| clippy | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | 0 | [log](release-clippy.log) |
| workspace-tests | `cargo test --workspace --all-targets --all-features --locked` | 0 | [log](release-workspace-tests.log) |
| doctests | `cargo test --workspace --all-features --doc --locked` | 0 | [log](release-doctests.log) |
| postgres-orm | `cargo test -p lex-db-controller --test postgres_orm -- --ignored --test-threads=1` | 0 | [log](release-postgres-orm.log) |
| postgres-permissions | `cargo test -p lex-rest --test postgres_permissions -- --ignored --test-threads=1` | 0 | [log](release-postgres-permissions.log) |
| postgres-mutations | `cargo test -p lex-rest --test postgres_mutation -- --ignored --test-threads=1` | 0 | [log](release-postgres-mutations.log) |
| rustdoc | `cargo doc --workspace --all-features --no-deps --locked` | 0 | [log](release-rustdoc.log) |
| features-default | `cargo check -p lex-framework --no-default-features --locked` | 0 | [log](release-features-default.log) |
| features-axum | `cargo check -p lex-framework --features axum --locked` | 0 | [log](release-features-axum.log) |
| features-rustls | `cargo check -p lex-framework --features axum-rustls --locked` | 0 | [log](release-features-rustls.log) |
| default-tests | `cargo test --workspace --all-targets --locked` | 0 | [log](release-default-tests.log) |
| consumer | `python3 integration/verify_consumer.py --postgres` | 0 | [log](release-consumer.log) |
| macro-consumers | `python3 integration/verify_macro_consumers.py` | 0 | [log](release-macro-consumers.log) |
| quickstart | `python3 integration/verify_examples.py` | 0 | [log](release-quickstart.log) |
| generated | `python3 integration/verify_generated.py --postgres` | 0 | [log](release-generated.log) |
| git-consumer | `python3 integration/verify_consumer.py --local-git` | 0 | [log](release-git-consumer.log) |
| package | `cargo package --workspace --exclude lex_framework_server --all-features --locked` | 0 | [log](release-package.log) |
| packaged-consumer | `python3 integration/verify_packages.py` | 0 | [log](release-packaged-consumer.log) |
| publish-dry-run | `cargo publish --workspace --exclude lex_framework_server --all-features --locked --dry-run` | 0 | [log](release-publish-dry-run.log) |
| stable | `cargo +stable check --workspace --all-targets --all-features --locked` | 0 | [log](release-stable.log) |
| audit | `cargo audit --json` | 0 | [log](release-audit.log) |
| docs-site | `npm run build` | 0 | [log](release-docs-site.log) |
| npm-audit | `npm audit --audit-level=high` | 0 | [log](release-npm-audit.log) |

All 27 runner checks and both documentation checks exited zero on 2026-10-04. The all-feature workspace run reported 223 passed, zero failed and three pre-existing PostgreSQL tests ignored; those three were then explicitly executed successfully in their dedicated gates. Independent and CLI-generated consumers each reported 17 passed, zero failed and zero ignored across PostgreSQL and SQLite. The three local Git dependency styles passed their SQLite consumer suites.

The doctest command passed but discovered zero doctests; this does not establish rustdoc example coverage. Quickstart examples were independently compiled and exercised through real HTTP requests. Strict rustdoc uses `RUSTDOCFLAGS="-D warnings"` in the runner environment. Stable was a build check, not a second complete test run. The docs-site build emitted its existing large JavaScript chunk warning. Dependency audits reported no known Rust vulnerabilities and no npm vulnerabilities at execution time.
<!-- /COMMAND_RESULTS -->

## M. Remaining limitations

SQLite legacy RBAC upgrade/rebuild is not supplied. Full generic-view/viewset/custom-action HTTP matrices, live WebSocket frames/origins/idle/closure behavior, real serialization/deadlock contention, live replica failure/consistency tests, exhaustive relation/index/macro error/field-boundary tests, PostgreSQL session/cookie expiry HTTP stories, provider upload/scanner acceptance and every important public API's complete rustdoc remain incomplete. Property/fuzz infrastructure is not implemented; candidates are documented. S3 runtime is incomplete. Distributed idempotency/throttle storage is an application/provider responsibility; test fixtures do not establish production durability or exactly-once mutations. Platform testing was Linux only. No hidden claim of exhaustive production assurance is made.

## N. Release blockers

1. Canonical GitHub repository, actual source commit/tag, repository metadata and hosted consumer verification are unavailable from the supplied archive.
2. Registry ownership/reservation and real registry publication/download verification remain maintainer steps; package/dry-run success is insufficient.
3. Existing SQLite RBAC users lack a supplied data-preserving upgrade path.
4. Remaining acceptance/security coverage above must be completed or deliberately excluded from the supported release contract, with explicit documentation. The complete requested definition of done is not met.

The repository is materially hardened and the recorded executed gates can be reproduced, but it should not be represented as an approved unrestricted production release.
