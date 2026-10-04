# Lex Framework

Rust models, PostgreSQL/SQLite queries and migrations, tenant permissions, authentication, REST composition and media policies behind the `lex-framework` facade. This tree is a **1.0.0 release candidate**. The manifest version is not release approval; see the recorded checks and remaining blockers below.

Requires the pinned Rust 1.97.1 toolchain (also the declared MSRV). Stable is checked separately. Start with [installation](docs/getting-started/installation.md), the [executed quickstart](docs/getting-started/quickstart.md), and the [developer documentation](docs/README.md).

```sh
cargo run --manifest-path integration/consumer/Cargo.toml --example first_model
cargo run --manifest-path integration/consumer/Cargo.toml --example first_api
```

The API example uses loopback and temporary SQLite data. Production requires reviewed migrations, authentication and write policies, trusted ingress and appropriate provider implementations. See [database differences](docs/guides/databases.md), [RBAC upgrades](docs/guides/rbac.md), and [deployment](docs/operations/deployment.md).

The primary Cargo package is `lex-framework`; Rust imports are `lex_framework`. Advanced distribution packages use `lex-` names and support explicit dependency aliases. Git branch/revision/tag layouts and normalized package consumers are exercised outside the workspace. This extracted source has no canonical GitHub URL or provenance; hosted release installation remains unverified. Registry publication is not implied.

See [the audit report](audit/REPORT.md), [behavior matrix](audit/TEST_MATRIX.md), [exact command results](audit/release-results.json), [release checklist](docs/operations/release.md), [changelog](CHANGELOG.md), and [contributing](CONTRIBUTING.md). No publication is automated by this audit. Licensed under MIT OR Apache-2.0.
