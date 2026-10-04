//! `lex_rest` is the transport/application layer for the Lex Rust ORM stack.
//!
//! Database persistence remains authoritative in `db_controller`; this crate supplies the
//! Django REST Framework-style request pipeline, permissions, views, routing, WebSockets,
//! idempotency, throttling, audit hooks, and schema metadata.

pub mod rest_framework;

pub use rest_framework::prelude;
