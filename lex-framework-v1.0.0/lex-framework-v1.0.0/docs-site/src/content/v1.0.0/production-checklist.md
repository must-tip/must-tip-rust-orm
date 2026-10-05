# Release procedure and publication order

All Lex crates currently use 1.0.0 and exact internal version requirements. Preserve the version in path+version dependencies; Cargo strips paths in registry packages. Rust library names are retained, while distribution packages now use the `lex-` prefix. Applications using old package names must update their dependency declarations; their Rust imports remain unchanged. This is a release candidate, not a claim of published 1.0 stability.

Normal dependency order (independent leaves may be published in any order):

1. `lex-db-controller`, `lex-media`, `lex-secure-input`, `lex-cli`
2. `lex-orm` (its dev dependency requires `lex-db-controller` for package verification)
3. `lex-models`
4. `lex-migration`
5. `lex-db` and `lex-rest`
6. `lex-framework`

`lex-rest`'s dev migration dependency also requires the earlier migration publication. `project` and the acceptance application are unpublished. All supported crate manifests inherit the canonical repository URL https://github.com/must-tip/must-tip-rust-orm. The read-only registry lookup on 2026-10-04 found no entries for these ten prefixed names; that does not reserve them or prove maintainer ownership. Generic names including db, media, migration, models and orm already identify unrelated packages. Evidence is in `audit/registry-prefixed.json` and `audit/registry.json`.

Package lists and archive verification must include licenses, readmes, macro sources and any runtime-required templates. The CLI embeds templates in Rust strings and does not need repository-only template files. Cargo 1.97.1 supports packaging this unpublished dependency graph together:

```sh
cargo package --workspace --exclude lex_framework_server --all-features --locked
python3 integration/verify_packages.py
```

The second command extracts normalized archives and uses local registry patches to test a version-based consumer. It does not prove downloads from crates.io. Registry ownership and the real hosted source must be resolved before publication.

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
