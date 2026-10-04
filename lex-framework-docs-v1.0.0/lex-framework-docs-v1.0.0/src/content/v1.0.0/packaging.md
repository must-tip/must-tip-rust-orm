# Installing and packaging Lex

The workspace now exposes `lex_framework` as the facade crate. Application crates may depend on the facade instead of importing every internal crate directly.

During workspace development:

```toml
[dependencies]
lex_framework = { path = "../path/to/crate/lex_framework" }
```

When published to a registry, use the published version instead. `lex_framework` re-exports the stable framework-facing database, REST, and media modules while keeping internal crate boundaries available for advanced use.

Install the project-management CLI with Cargo:

```bash
cargo install --path crate/lex_cli --force
```

After `lex` exists, normal framework operations are CLI commands (`lex create settings`, `lex create crate`, `lex doctor`, `lex verify`, `lex migrate`, `lex start server`). No operational shell-script layer is required.

Before registry publication, set repository/homepage/version metadata, run `cargo package` for each public crate, and publish dependency crates before the facade and CLI.
