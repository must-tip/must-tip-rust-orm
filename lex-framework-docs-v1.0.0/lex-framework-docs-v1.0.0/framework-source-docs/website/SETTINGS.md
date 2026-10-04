# Settings reference

The primary project configuration is `Config.toml`. Secrets remain in environment variables or system credential injection.

## Database

```toml
[database]
backend = "auto"
max_connections = 10
require_ssl = false
sqlite_create_if_missing = true
sqlite_foreign_keys = true
sqlite_wal = true
```

Development can automatically use `sqlite://db.sqlite3`. Production should use an explicit backend/URL.

## Authentication

```toml
[security.authentication]
backends = ["session", "bearer"]
```

See [Authentication](AUTHENTICATION.md).

## Permissions

```toml
[security.permissions]
backend = "scopes"
```

See [Permissions](PERMISSIONS.md).

## HTTP security

Use `[security.cors]`, `[security.headers]`, and `[security.request_limits]`. See [Middleware](MIDDLEWARE.md).

## Media

Use `[media]`, `[media.s3]`, and `[media.clamav]`. See [Media](MEDIA.md).

## Environment overrides

The framework uses the `APP__...` environment hierarchy for feature settings and the existing database settings loader for core database/server configuration. Keep credentials out of source-controlled TOML.
