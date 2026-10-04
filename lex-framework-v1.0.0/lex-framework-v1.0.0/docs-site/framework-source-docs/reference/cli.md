# Command-line workflow

Install the `lex` binary with `cargo install --path crate/lex_cli --locked` from a checkout. Its distribution package is `lex-cli`; the executable remains `lex`. Git/registry installation requires verified source identity/publication first.

```sh
lex create project my_api --framework-path /absolute/path/to/lex
cd my_api
lex create crate projects --url /api/projects
lex refresh
lex check
cargo check --workspace --all-targets
lex start server
```

Use `--git URL --rev COMMIT` or `--tag TAG` in place of `--framework-path` for immutable hosted dependencies. Omitting a source generates future registry dependencies. It does not make unpublished crates available. Existing project directories are rejected to protect application files; duplicate application creation is also an error. `--root DIR` selects a project or the parent directory for a new project.

`refresh` discovers application manifests, updates workspace membership, registers models and mounts URL routes. Generated wiring is marked by managed sections; keep application logic in its own modules. `check` validates structural wiring; it does not certify compilation or security. `doctor` adds Cargo checks. `routes` lists mounted application routes.

Migration commands are forwarded through the generated `project/src/bin/lex-migrate.rs`, which registers application models and calls Lex's public migration CLI. Use `DATABASE_URL` with a dedicated database. The database URL flag is `-u`/`--db-url`; migration directory is `-d`/`--dir`.

```sh
lex migrate -- --dir ./migrations migrate
lex migrate -- --dir ./migrations status
lex migrate -- --dir ./migrations pending
lex migrate -- --dir ./migrations rollback
```

`make NAME --up FILE --down FILE` writes reviewed SQL pairs; it does not infer arbitrary schema changes. `reset --yes` is destructive. Read [migration semantics](../concepts/migrations.md) before rollback/reset. Repository maintenance commands such as `lex verify` assume the framework workspace's package set; consumer applications should run their own Cargo checks and acceptance tests.

Run `python3 integration/verify_generated.py --postgres` from a checkout to generate a temporary independent application, register public models, execute its ORM/HTTP tests, apply/reapply/rollback a file-backed SQLite migration, and start its real TCP server. PostgreSQL tests additionally require the documented disposable database variables.
