# lex-rest

Lex 1.0.0 release candidate: authentication, permissions, serializers, APIViews and optional Axum transport. Requires Rust 1.97.1. Licensed under MIT OR Apache-2.0.

Most applications should depend on the `lex-framework` facade. This package supports advanced integrations; its Rust library name is `lex_rest`. After publication, use a dependency alias to retain that import:

```toml
[dependencies]
lex_rest = { package = "lex-rest", version = "1.0" }
```

Registry publication is not yet certified. For checkout development replace `version` with an absolute path to this crate; for Git use the verified repository and a release tag or immutable revision. See the repository documentation and this crate's rustdoc for supported APIs. Do not infer a GitHub URL from this source archive.
