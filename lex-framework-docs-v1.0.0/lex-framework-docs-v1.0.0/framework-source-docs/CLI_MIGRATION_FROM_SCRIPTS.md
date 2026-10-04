# Migration from repository scripts to `lex`

The framework no longer ships operational shell or Python scripts. Use the following CLI equivalents.

| Previous workflow | New framework command |
|---|---|
| install local CLI helper | `cargo install --path crate/lex_cli --force`, then `lex self install` for later refreshes |
| workspace dependency audit | `lex audit workspace` |
| dynamic SQL audit | `lex audit sql` |
| Rust source-integrity audit | `lex audit source` |
| all source audits | `lex audit all` |
| complete framework verification | `lex verify framework` |
| SQLite verification | `lex verify sqlite` |
| provision disposable PostgreSQL | `lex database test-postgres setup` |
| inspect PostgreSQL test configuration | `lex database test-postgres status` |
| PostgreSQL integration verification | `lex verify postgres` |
| framework + SQLite verification | `lex verify all` |
| framework + SQLite + PostgreSQL verification | `lex verify all --postgres` |

## Automation and CI

CI should call `lex` directly after installing the CLI:

```bash
cargo install --path crate/lex_cli --force
lex audit all
lex verify framework
lex verify sqlite
```

For PostgreSQL integration CI, inject a clearly disposable database with `TEST_DATABASE_URL` and `CONFIRM_DISPOSABLE_DATABASE=yes`, or provision one in the host environment with `lex database test-postgres setup`.

Do not add replacement shell wrappers around these commands unless an external CI platform itself requires a minimal launcher. Framework behavior belongs inside the Rust CLI.
