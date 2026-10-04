# Configuration and feature contract

`FrameworkSettings::load` reads Config.toml and environment configuration, validates core database/server settings and production security settings, and composes media/auth/permission policies. APP_PROFILE recognizes prod/production case-insensitively. Production requires DATABASE_URL; development can use the SQLite default. Do not let fallback configuration connect to a production database during tests.

A programmatic SQLite configuration is exercised in `crate/lex_framework/tests/auth_http.rs`: construct Settings, set `database.url = "sqlite::memory:"`, initialize Database, then construct FrameworkSettings/security settings and call `authentication(&database)`. Creating authentication does not create built-in RBAC schema. Install that schema via the reviewed migration API first.

Features on the `lex-framework` package: default is empty; `axum` enables the HTTP adapter, `axum-rustls` also enables the optional TLS serving adapter. These features forward to lex_rest. Database backend selection is runtime configuration: PostgreSQL and SQLite SQLx support are currently compiled together. There are no `sqlite` or `postgres` facade features; do not advertise them. `postgres-integration` features on low-level test crates are test gates requiring explicit disposable database environment variables, not production backend selectors.

Inspect the typed Settings structs/rustdoc for all defaults. CORS development default permits localhost:3000 and does not allow credentials. Cookie and session auth are disabled unless their backend names are listed. Named scope permission backend is the default. Default host is loopback. Production transport requires TLS or explicitly trusted termination. Validate upload limits, signing secret presence, origins and backend names before serving requests.

`cookie-integration` is a test-only facade feature. It requires `LEX_TEST_COOKIE_SECRET` to be set to an explicit disposable test secret. `--all-features` therefore requires that variable for the signed-cookie HTTP tests; never use a deployment secret. Production cookie signing reads the configured secret environment variable instead.

Lex-managed SQLite rejects `database.sqlite_foreign_keys = false`: pooled connections must all enforce referential integrity. Backend-mismatch errors identify the configured/detected backend without echoing database URLs. SQLite startup logs omit the URL as well.
