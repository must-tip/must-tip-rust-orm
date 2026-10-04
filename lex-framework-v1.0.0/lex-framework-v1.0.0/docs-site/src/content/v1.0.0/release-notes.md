# Changelog

## Unreleased — 1.0.0 release audit

### Added

- Normalized MembershipRole/RolePermission entities, tenant-matched composite FKs, deduplicating constraints, live permission repository for both backends and conservative PostgreSQL legacy conversion.
- Isolated facade-only acceptance application; macro compile-fail diagnostics; HTTP auth/CSRF/error tests; RLS connection-reuse tests; filesystem/scanner threat regressions.
- Standalone CLI project generation with local checkout, pinned Git and future registry dependency sources.
- SQLite tracked saves enforce the same stale-version conflicts and dirty-field behavior as PostgreSQL.
- Versioned path dependencies, package readmes/licenses, current contract documentation and release gates.

### Changed

- Query and REST database-policy internals split into modules while retaining public import paths.
- Facade database permission configuration selects normalized RBAC. Existing applications must convert legacy grants before switching; the legacy repository remains available explicitly.
- Axum transport defaults to Insecure; direct TLS/trusted termination must be declared explicitly.
- Multiple valid authentication mechanisms are rejected; valid early credentials cannot mask invalid later credentials.

### Fixed

- Baseline compile errors in bulk update SQL, CLI generated fixture delimiters/manifest borrowing, and HMAC key initialization.
- Facade-only and aliased derive paths, relation/composite metadata expansion, and SQLite UUID text row decoding.
- Eager relation hydration preserves decimal precision, JSON and SQLite boolean types; binary hydration returns an explicit unsupported error.
- Qualified and nullable exact decimal fields derive and round-trip correctly on both backends.
- SQLite exact-decimal numeric SQL now fails explicitly instead of silently using text comparison or floating-point arithmetic. Exact storage, assignment, null checks and counts remain supported. Decimal unique/relation keys and distinct projections are also rejected.
- SQLite automatic model initialization rejects FK-disabled custom pools.
- SQLite bulk create/update execute atomically; failed batches no longer persist earlier rows.
- Migration CLI database URL now uses `-u` to avoid the conflicting directory `-d` flag.
- XML string encoding and control-character rejection, with round-trip tests.
- CLI UTF-8 audit panics, nested TOML dependency handling and inline workspace member editing.
- Wrapped storage errors retain HTTP redaction and appropriate status semantics.

### Security

- Lex-managed SQLite pools reject disabled foreign keys and enforce them on every connection. Backend mismatch errors and startup logging omit database URLs, including credentials in query parameters.

- Directory capabilities prevent filesystem upload/read/delete symlink escapes; unique temporary files prevent collisions and destructive overwrites of unrelated temporary-named objects.
- Database and issued credential Debug output redacts secrets. Random credentials use UUIDv4 randomness and TTL overflow is rejected.
- Named PostgreSQL RBAC foreign keys and required unique-key shapes on both backends are checked against the actual catalog; weakened existing constraints require explicit repair.
- CORS validates origin/method/header policy, rejects disallowed preflights, and preserves existing Vary headers.
- Missing tenant context denies tenant-object permissions and scope-based tenant membership/admin checks.
- JavaScript string encoding escapes HTML delimiters. ClamAV accepts only exact clean responses and bounds protocol reads.

### Compatibility

Distribution packages now use Lex-prefixed names (`lex-framework`, `lex-db`, `lex-orm`, etc.) because generic registry names were occupied by unrelated projects. Rust library names, facade modules, and existing legacy RBAC models are retained. Individual-crate users can retain dependency aliases, e.g. `db_controller = { package = "lex-db-controller", version = "1" }` after publication. Security-sensitive behavior changes above are intentional. Registry publication and hosted Git identity are not implied by the 1.0.0 manifests. See release gates and the audit report for unresolved coverage/publication limitations.
