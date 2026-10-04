pub mod apply;
pub mod cli;
pub mod command_handlers;
pub mod diff;
pub mod graph;
pub mod operation;
pub mod rbac;
pub mod scanner;
pub mod sqlite_command_handlers;

pub use cli::run_migration_cli;
pub use command_handlers::{
    handle_make_migration, handle_migrate, handle_migrate_tree, handle_reapply, handle_reset,
    handle_rollback, handle_show_migrations, handle_show_pending, pending_migrations,
    pending_migrations_tree, write_migration_file,
};
pub use graph::{MigrationGraph, MigrationNode};
pub use operation::{IndexMethod, MigrationOperation, operations_for_model};

pub use sqlite_command_handlers::{
    handle_migrate_sqlite, handle_migrate_tree_sqlite, handle_reapply_sqlite, handle_reset_sqlite,
    handle_rollback_sqlite, handle_show_migrations_sqlite, pending_migrations_sqlite,
    pending_migrations_tree_sqlite,
};
