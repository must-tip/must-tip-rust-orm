# orm

Procedural-macro crate providing `#[derive(Model)]` for the workspace's `db_controller::Entity` contract.

It generates deterministic schema metadata, model validation, SQLx PostgreSQL row decoding, index DDL, and soft-delete trait implementation when declared. Relation projection fields are not persisted as columns.

Persisted `ChoiceField<T>` is intentionally rejected; use a typed enum with SQLx/Serde representation instead.
