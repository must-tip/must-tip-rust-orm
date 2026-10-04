# lex-db

Lex 1.0.0 release candidate: validated configuration and database pools. Requires Rust 1.97.1. Licensed under MIT OR Apache-2.0.

Most applications should depend on the `lex-framework` facade. This package supports advanced integrations; its Rust library name is `db`. After publication, use a dependency alias to retain that import:

```toml
[dependencies]
db = { package = "lex-db", version = "1.0" }
```

Registry publication is not yet certified. For checkout development replace `version` with an absolute path to this crate; for Git use the verified repository and a release tag or immutable revision. See the repository documentation and this crate's rustdoc for supported APIs. Do not infer a GitHub URL from this source archive.
