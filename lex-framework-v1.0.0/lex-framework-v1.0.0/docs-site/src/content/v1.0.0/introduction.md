# Lex developer documentation

Lex is a Rust framework combining model metadata, an ORM, PostgreSQL/SQLite migration runners, transport-neutral REST policies, and an optional Axum adapter. The primary dependency is **`lex-framework`**; Rust imports remain `lex_framework`. Version 1.0.0 and Rust 1.97.1 are the current manifest contract; release validation is recorded in `audit/` rather than inferred from a version number.

Start with [installation](getting-started/installation.md), the [executable quickstart](getting-started/quickstart.md), and [architecture](concepts/architecture.md).

- [Database compatibility](guides/databases.md): portable behavior and PostgreSQL-only operations.
- [Authentication](guides/authentication.md) and [authorization](guides/security.md): credentials, scopes, tenant resolution and safe writes.
- [Normalized RBAC and upgrades](guides/rbac.md): schema constraints and legacy conversion.
- [Migrations](concepts/migrations.md): file execution versus typed SQL generation.
- [Models and queries](concepts/models-and-queries.md), [transactions](concepts/transactions.md), and [CLI](reference/cli.md).
- [REST, uploads and output encoding](guides/application-boundaries.md).
- [Configuration and features](reference/configuration.md).
- [Deployment and reverse proxies](operations/deployment.md).
- [Testing matrix](../audit/TEST_MATRIX.md), [contributing](../CONTRIBUTING.md) and [release procedure](operations/release.md).

Older uppercase guides, `docs/website`, and implementation notes are historical material. The pages linked above describe the audited contract. Historical examples must be checked against the executable consumer before reuse. The docs site contains copies of these current contract pages.
