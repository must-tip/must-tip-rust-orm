# Authentication simplification

This revision removes application-owned authentication backend wiring.

## Developer contract

1. Choose `session`, `cookie`, `bearer`, or an ordered combination in `Config.toml`.
2. Start the project normally with `lex start server`.
3. Implement `APIView` business logic. Views are authenticated by default.
4. Add `fn access(&self) -> ViewAccess { ViewAccess::Public }` only for intentionally public endpoints.

## Framework responsibilities

- constructs the configured authentication chain;
- creates framework session/token storage for SQLite and PostgreSQL;
- persists only SHA-256 digests of opaque session/bearer credentials;
- checks expiry and revocation;
- enforces CSRF for unsafe session/cookie requests;
- signs/verifies stateless auth cookies with HMAC-SHA256;
- rejects invalid recognized credentials instead of silently falling through;
- exposes `FrameworkAuthService` for issuing/revoking credentials after application login logic verifies identity.

There is no generated `project/src/auth.rs` and no application provider registry.
