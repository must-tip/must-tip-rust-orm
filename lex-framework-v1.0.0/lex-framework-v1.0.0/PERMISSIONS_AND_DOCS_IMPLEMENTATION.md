# Permissions and Developer Experience Revision

This revision simplifies application development without replacing the existing permission engine.

## Added

- `Permissions`: ergonomic constructors for public/authenticated/admin/owner/named/composed permission rules.
- `ViewPermissions`: declarative per-HTTP-method permissions with authenticated-by-default behavior.
- Public `OPTIONS` handling for framework-owned preflight/metadata responses.
- `ViewServices::require()` and `ViewServices::require_object()` for data-dependent authorization inside handlers.
- `RequestContext::user_id()`, `is_authenticated()`, and `has_scope()` convenience methods.
- `ScopePermissionBackend`: zero-wiring permission backend that maps named permissions to authenticated principal scopes.
- `[security.permissions]` settings with `scopes` and PostgreSQL `database` backends.
- Framework-owned permission backend construction from `FrameworkSettings`.
- Generated server and CLI templates now use the selected framework permission backend automatically.
- `lex_framework::permissions` and `lex_framework::middleware` application-facing modules.
- Website-ready documentation under `docs/website/`.

## Compatibility

The existing `PermissionExpr`, `permission_policy`, `DatabasePermissionBackend`, object permission system, viewsets, and lower-level middleware APIs remain available. New application code can use the simpler facade while advanced code can continue using the underlying APIs.

## Validation limitation

This artifact environment does not provide `cargo`, `rustc`, or `rustfmt`; therefore final Rust compiler validation must be run in a Rust-enabled development environment with `lex doctor` and `lex verify framework`.
