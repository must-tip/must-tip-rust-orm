# Migration architecture and reversibility

The **SQL-file migration runner** is the primary execution workflow exposed by `migration-cli`, `lex migrate`, and `Database` startup migration settings. It supports PostgreSQL and SQLite runners, ordered history and transactional execution. `MigrationOperation` and `MigrationGraph` are typed generation/planning primitives: render reviewed SQL into files consumed by the runner. They are not a second automatic schema synchronization service.

The public CLI supports `make NAME --up UP.sql --down DOWN.sql`, `migrate`, `rollback`, `status`, `pending`, `diff`, `reapply`, and explicitly confirmed `reset --yes`. Specify a disposable database for testing. Never run reset/reapply against production merely to resolve history confusion. In a repository checkout `lex migrate` runs the workspace migration binary. Standalone generated workspaces include a `lex-migrate` binary in the application server; `lex migrate` invokes that runner and registers the application models before planning. Existing projects can add the runner with `lex create settings`, which preserves existing source unless force is explicitly requested.

Both forward and reverse SQL must be reviewed for the chosen backend. A reverse CREATE TABLE can recover a schema shape, not data lost by DROP TABLE or DROP COLUMN. Indexes/constraints require explicit companion operations. SQLite does not support PostgreSQL ALTER TABLE constraint syntax; use reviewed rebuild operations when required. Schema comparison/diff is a planning aid, not a guarantee of data-safe migration.

Typed graph construction rejects empty nodes, missing dependencies, duplicate IDs, self-dependencies and cycles. Independent nodes are ordered deterministically. File execution history must not be mixed with a separate tool managing the same schema without a documented ownership boundary.

RBAC changes have dedicated transactional installation/conversion APIs described in the RBAC guide. They deliberately refuse corrupt source authorization state. Test reversals using persisted observable state, not only generated SQL strings.
