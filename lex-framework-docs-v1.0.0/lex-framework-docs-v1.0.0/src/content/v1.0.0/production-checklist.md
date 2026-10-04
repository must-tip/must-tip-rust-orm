# Production checklist

Lex is intentionally strict about the difference between development convenience and production safety.

## Before deployment

- Run `lex doctor`.
- Run `lex verify framework`.
- Run `lex verify sqlite` if SQLite behavior matters to your release.
- Provision a disposable PostgreSQL test database and run `lex verify postgres` for PostgreSQL releases.
- Use an explicit production database configuration.
- Review migrations before applying them.
- Configure secure transport or explicitly trusted TLS termination.
- Keep credentials out of `Config.toml` and source control.
- Use CSRF protection with session/cookie authentication.
- Configure explicit CORS origins for credentialed browser requests.
- Configure CSP, HSTS, Permissions-Policy, request limits, and trusted proxy behavior appropriately.
- Configure persistent media storage and malware scanning when the application accepts uploads.
- Verify observability, rollback procedures, and backups.

## Runtime expectations

Production should fail closed when mandatory security/dependency configuration is missing. Do not convert PostgreSQL-only semantics into weaker silent fallbacks.
