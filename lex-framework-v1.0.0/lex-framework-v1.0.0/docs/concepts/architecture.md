# Architecture and supported entry points

`lex_framework` is the application facade. `lex_framework::prelude::*` exposes the established model/REST convenience imports; field types and advanced APIs remain discoverable through `lex_framework::db_controller`. `lex_framework::orm::Model` is the derive macro. Application code needs no direct `db_controller` dependency merely to derive a model. Dependency aliases are supported by the derive resolver.

| Cargo package (Rust library) | Responsibility | Consumer role |
|---|---|---|
| lex-framework (`lex_framework`) | settings, authentication construction, facade | primary dependency |
| lex-rest (`lex_rest`) | request runtime, authorization, serializers/views, optional Axum | custom HTTP integration |
| lex-db-controller (`db_controller`) | fields, Entity, QuerySet, transactions, REST database policies | advanced ORM integration |
| lex-orm (`orm`) | Model derive | macro, usually via facade |
| lex-db (`db`) | validated settings, primary/replica pools | advanced database composition |
| lex-migration (`migration`) | SQL-file runner, typed operation generator and graph | migration tooling |
| lex-models (`models`) | built-in identities and RBAC entities | advanced identity integration |
| lex-media (`media`) | storage and malware scanner boundaries | storage providers |
| lex-secure-input (`secure_input`) | validation and context-specific encoders | input/output policies |
| lex-cli (`lex_cli`) | `lex` project/app generator and operational commands | installed executable |

The unpublished `project` server is repository scaffolding. Generated applications are independent workspaces and do not copy framework implementation crates.

The dependency direction is ORM primitives → models → migrations → database/runtime composition. Procedural expansion resolves consumer crate names. Query building is separate from execution: `query/` contains value, predicate, plan, queryset, mutation, relation, compiler, and PostgreSQL/SQLite executor modules. Existing `db_controller::query::*` imports remain supported. `rest/` separates errors, access policy, serializer, filtering, query policy, ordering and signed cursors.

Lex constructs configured session, bearer and signed-cookie authentication. Applications still own login/enrollment workflows, credential delivery, trusted tenant resolution, resource ownership rules, and view write-field allowlists. Those responsibilities must be explicit in an application's design.

Advanced direct dependencies can preserve existing imports with explicit aliases, for example `db_controller = { package = "lex-db-controller", version = "1.0" }` and `orm = { package = "lex-orm", version = "1.0" }` after publication. Without those aliases Cargo normalizes the package keys to `lex_db_controller` and `lex_orm`. The derive resolves that actual key. `integration/verify_macro_consumers.py` independently tests default direct names, aliases and a renamed facade, including relations and exact decimal row decoding.
