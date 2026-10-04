# Framework-owned authentication implementation

Lex now composes authentication entirely inside `lex_framework`.

- `session`: framework-owned database session table (`_lex_auth_sessions`), opaque credentials, digest-only persistence, expiry/revocation and CSRF enforcement.
- `bearer`: framework-owned opaque token table (`_lex_auth_tokens`), digest-only persistence, expiry/revocation and scopes.
- `cookie`: stateless HMAC-SHA256 signed payload using the runtime secret named by `security.authentication.cookie.secret_env`.
- backend ordering remains fail-closed: invalid credentials for a recognized backend reject rather than silently falling through.

Application projects no longer generate `project/src/auth.rs` and do not register authentication resolver/provider implementations.

`FrameworkSettings::authentication(&Database)` initializes required storage and returns the `AuthenticationChain` plus a `FrameworkAuthService` for issuing/revoking credentials from application login/business logic.

`APIView` is authenticated by default. `ViewAccess::Public` is the explicit anonymous-access opt-out.

This keeps authentication infrastructure in the framework while leaving identity-proof business logic (password verification, passkeys, OAuth callbacks, enterprise SSO, etc.) in application code.
