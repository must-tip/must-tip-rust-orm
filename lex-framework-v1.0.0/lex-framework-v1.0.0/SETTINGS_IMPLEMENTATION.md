# Settings and framework packaging implementation

This revision makes `Config.toml` the central safe configuration surface for database, server, media and HTTP security behavior while keeping secret values in environment/system credential injection.

Highlights:

- `lex_framework` facade crate for application developers.
- Typed `FrameworkSettings` that composes core database/server settings with media and security settings.
- Memory and filesystem media storage implementations.
- S3-compatible provider configuration suitable for AWS S3, Backblaze B2 S3 API and MinIO without coupling the framework core to one SDK.
- Native ClamAV `clamd` INSTREAM scanner.
- Configurable CSP, Permissions-Policy, HSTS, CORS and request header/URI limits.
- Production validation rejects memory media storage, invalid credentialed wildcard CORS and missing S3 credentials when S3 is selected.
- `lex create settings` generates the same documented settings surface for new projects.
- Framework facade re-exports ORM, database controller, migrations, REST, media and secure-input APIs.

See `docs/SETTINGS_AND_MEDIA.md` and `docs/PACKAGING.md`.
