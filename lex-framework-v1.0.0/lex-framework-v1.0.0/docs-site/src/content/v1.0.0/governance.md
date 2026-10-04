> **Historical documentation.** Check the current quickstart and testing matrix before using these examples.

# Release procedure and publication order

All Lex crates currently use 1.0.0 and exact internal version requirements. Preserve the version in path+version dependencies; Cargo strips paths in registry packages. Primary imports must match whether the source is Git or registry. Generic package names require crates.io ownership/availability verification; renaming is a breaking distribution decision, not a cosmetic manifest fix.

Normal dependency order (independent leaves may be published in any order):

1. db_controller, media, secure_input, lex_cli
2. orm (its dev dependency requires db_controller for package verification)
3. models
4. migration
5. db and lex_rest
6. lex_framework

lex_rest's dev migration dependency also requires the earlier migration publication. `project` and the acceptance application are unpublished. Metadata still needs the canonical repository URL. Package lists and archive verification must include licenses, readmes, macro sources and any runtime-required templates. The CLI embeds templates in Rust strings and does not need repository-only template files.

Release gates:

- [ ] canonical GitHub URL, branch, actual tag/revision verified
- [ ] package names owned/available, versions aligned
- [ ] cargo fmt --all -- --check
- [ ] cargo check --workspace --all-targets --all-features
- [ ] cargo clippy --workspace --all-targets --all-features -- -D warnings
- [ ] workspace tests and doctests
- [ ] SQLite consumer/migration/security tests
- [ ] PostgreSQL tests including explicitly selected legacy ignored tests
- [ ] RLS connection reuse and RBAC constraint tests
- [ ] macro compile-pass/fail tests
- [ ] HTTP authentication/security/mutation tests
- [ ] generated application builds and full developer acceptance on both backends
- [ ] isolated hosted Git consumer at the actual release commit/tag
- [ ] cargo package for every public crate, packaged consumer compilation
- [ ] cargo publish --dry-run for every public crate in dependency order
- [ ] rustdoc with denied warnings and docs site production build
- [ ] dependency advisory audit reviewed
- [ ] CHANGELOG and compatibility/migration instructions reviewed
- [ ] no unresolved release-critical security or installation blocker
- [ ] clean release commit, release tag and matching documentation

Only an authorized maintainer runs `cargo login`/`cargo publish`. Never put registry credentials into ordinary tests/CI. Dry runs can fail before dependency crates are published; record that blocker instead of reporting a verified package. Repository build success alone is insufficient for publication approval.
