# Contributing

Use the pinned Rust 1.97.1 toolchain and its rustfmt/Clippy components. Install with `rustup toolchain install 1.97.1 --component rustfmt --component clippy`. The declared MSRV is the same version. Run fmt, workspace checks, Clippy with denied warnings, tests and strict rustdoc. Relevant commands and evidence live in `audit/`.

Start the dedicated test database with `docker compose -f compose.test.yml up -d --wait`. Set TEST_DATABASE_URL to the local test database and CONFIRM_DISPOSABLE_DATABASE=yes. Never supply production credentials. Set `LEX_TEST_COOKIE_SECRET` to an explicit disposable test signing key, then run `cargo test --workspace --all-targets --all-features`, then explicitly run the three existing PostgreSQL tests with `--ignored --test-threads=1`. Default-feature tests remain useful for SQLite-only environments. New PG tests use isolated random schemas; legacy tests require serialization.

Run `python3 integration/verify_consumer.py` for an external path consumer and `--local-git` for Git layout. This is not a hosted GitHub release test; use --git/--tag or --rev once identity is verified. Compile CLI-generated projects outside the repository. Build docs with `npm ci && npm run build` in docs-site.

Keep model metadata/query representation separate from execution/dialects. Preserve public reexports when moving modules. Centralize tenant/security invariants and test them through real persisted state and HTTP services. New migration operations need forward SQL, explicit reversibility limitations, backend behavior, and rollback/failure tests. New macro attributes need clear syn errors and compile-fail fixtures. Do not approve snapshot output without inspecting the diagnostic.

Useful future fuzz targets: signed-cookie decoding, cursor authentication, identifier/type validation, migration graphs, filenames, config deserialization and HTTP header parsing. Property tests should prove a meaningful invariant rather than reproduce an implementation.

Run `python3 integration/verify_examples.py` for the copyable ORM and live HTTP quickstart, `python3 integration/verify_generated.py --postgres` for generated-project acceptance, and `python3 integration/verify_packages.py` after workspace packaging. `audit/TEST_MATRIX.md` distinguishes executed behavior from remaining coverage. `integration/verify_release.py` records exact commands, exit codes and separate logs; its PostgreSQL tests require the disposable database variables above.

Run `python3 integration/verify_macro_consumers.py` for default and renamed direct/facade dependencies; procedural macro changes must preserve all three forms.

For the included Compose database, a complete local run is:

```sh
export TEST_DATABASE_URL='postgres://lex_test:lex_local_test_only@127.0.0.1:55432/lex_disposable'
export CONFIRM_DISPOSABLE_DATABASE=yes
export LEX_TEST_COOKIE_SECRET='lex-disposable-local-cookie-signing-secret-not-for-deployment'
python3 integration/verify_release.py
```

These values are disposable fixtures. The runner never publishes; it performs a workspace publish dry run and records its result. Install cargo-audit 0.22.2 for its advisory gate.

After editing canonical Markdown, run `python3 integration/sync_docs.py` before the docs-site build. The documentation CI verifies copies match those sources.
