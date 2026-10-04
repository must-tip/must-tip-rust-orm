# db_controller

Canonical persistence primitives for the workspace: model fields, `Entity` metadata, repository operations, guarded dynamic query builders, and reusable database behavior traits.

Use `ModelManager<T>` for typed model persistence. The lower-level query builders preserve the uploaded string-valued API for compatibility and validate/quote all dynamic identifiers.

See the workspace `docs/` directory for architecture, security, and operational contracts.
