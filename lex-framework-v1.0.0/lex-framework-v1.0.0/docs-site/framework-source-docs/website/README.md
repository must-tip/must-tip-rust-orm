> **Historical documentation.** These pre-audit notes may describe obsolete APIs or ownership. Use the [current developer documentation](../README.md) and executable acceptance examples for the supported contract.

# Lex Framework Documentation

Lex is a Rust application framework designed to make large backend projects feel predictable and approachable. Application developers work primarily with `lex_framework`, `lex`, models, views, settings, and app-local URLs. The framework owns database startup, authentication plumbing, permission enforcement, security middleware, routing composition, and common infrastructure.

## Start here

1. [Quick start](QUICKSTART.md)
2. [Views and URLs](VIEWS_AND_URLS.md)
3. [Authentication, sessions, cookies, and bearer tokens](AUTHENTICATION.md)
4. [Permissions](PERMISSIONS.md)
5. [Middleware and HTTP security](MIDDLEWARE.md)
6. [Settings reference](SETTINGS.md)
7. [Media and malware scanning](MEDIA.md)
8. [Recipes](RECIPES.md)

## Core design

```text
Config.toml
   │
   ├── database
   ├── authentication
   ├── permissions
   ├── CORS / CSP / request limits
   └── media
          │
          ▼
     lex_framework
          │
   ┌──────┼─────────┐
   ▼      ▼         ▼
 models  views   middleware
   │      │         │
   └──────┴────┬────┘
               ▼
           application
```

The default developer rules are deliberately small:

- A normal `APIView` is authenticated.
- Use `ViewPermissions` to change access rules.
- `Permissions::named("...")` works immediately with the default scope permission backend.
- Authentication methods are selected in `Config.toml`; application views do not implement authentication parsers.
- Default security middleware comes from settings.
- Custom middleware implements one framework trait.
- SQLite works for development; PostgreSQL remains the richer production backend.
