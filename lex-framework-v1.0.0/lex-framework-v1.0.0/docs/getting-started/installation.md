# Installation lifecycle

Install Rust **1.97.1** with rustfmt and Clippy. This is both the pinned toolchain and declared minimum; no lower MSRV is promised.

The primary package is `lex-framework`; its Rust import remains `lex_framework`. All distribution packages use the `lex-` prefix to avoid unrelated registry packages with generic names. Before registry publication, applications use a Git dependency on the canonical repository. The canonical repository is https://github.com/must-tip/must-tip-rust-orm, verified from the Git remote and public HEAD on 2026-10-05. Release tags still require explicit publication and verification.

The supported Cargo declarations have the following forms; replace the explicitly named placeholders with verified release metadata:

```toml
[dependencies]
# Recommended for deployment after a real release tag exists:
lex-framework = { git = "https://github.com/must-tip/must-tip-rust-orm", tag = "VERIFIED_RELEASE_TAG", features = ["axum"] }
# Alternatively pin an immutable commit:
# lex-framework = { git = "https://github.com/must-tip/must-tip-rust-orm", rev = "VERIFIED_COMMIT", features = ["axum"] }
# Development only:
# lex-framework = { git = "https://github.com/must-tip/must-tip-rust-orm", branch = "main", features = ["axum"] }
```

The isolated acceptance runner tests branch, commit and tag forms against a local Git snapshot. That proves Cargo Git dependency layout, **not** that a particular hosted GitHub release exists. To verify a published release, run `python3 integration/verify_consumer.py --git URL --tag TAG`.

After all crates are published and verified, the declaration becomes:

```toml
[dependencies]
lex-framework = { version = "1.0", features = ["axum"] }
```

Application imports and model declarations stay identical. Registry availability and package ownership must be checked before publishing these instructions as an available release.

For local checkout development, use `lex create project my_api --framework-path /absolute/path/to/lex`. For hosted consumers use `lex create project my_api --git URL --rev COMMIT` or `--tag TAG`. With no explicit source the generator writes future registry dependencies; those only resolve after publication. Keep `Cargo.lock` for application deployments. Review dependency updates and use `cargo update -p lex-framework` deliberately.
