//! Rust-native, Django-style QuerySet primitives.
//!
//! The public builder never interpolates untrusted values into SQL. Identifiers are validated
//! against model metadata and values are bound through SQLx. Tenant-aware models fail closed
//! unless an explicit tenant or privileged global scope is selected.

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use rust_decimal::Decimal;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use sqlx::{
    Error as SqlxError, FromRow, Postgres, Row, Sqlite, Transaction,
    postgres::{PgPool, PgRow},
    sqlite::{SqlitePool, SqliteRow},
    types::Json,
};
use std::{
    collections::{BTreeSet, HashMap},
    marker::PhantomData,
    net::IpAddr,
    time::Duration,
};
use uuid::Uuid;

use crate::{
    BooleanField, ColumnSpec, DateTimeField, DecimalField, Entity, FloatField, IntegerField,
    ModelError, RelationCardinality, RelationSpec, SlugField, TextField, UUIDField,
    effective_schema, quote_identifier,
};

macro_rules! bind_value {
    ($query:expr, $value:expr) => {{
        match $value {
            DbValue::Bool(value) => $query.bind(value),
            DbValue::I16(value) => $query.bind(value),
            DbValue::I32(value) => $query.bind(value),
            DbValue::I64(value) => $query.bind(value),
            DbValue::F32(value) => $query.bind(value),
            DbValue::F64(value) => $query.bind(value),
            DbValue::Decimal(value) => $query.bind(value),
            DbValue::Uuid(value) => $query.bind(value),
            DbValue::DateTime(value) => $query.bind(value),
            DbValue::String(value) => $query.bind(value),
            DbValue::Json(value) => $query.bind(Json(value)),
            DbValue::Bytes(value) => $query.bind(value),
        }
    }};
}

mod compiler;
mod executor;
mod mutation;
mod mutation_support;
mod ordering;
mod plan;
mod policy;
mod postgres;
mod predicate;
mod queryset;
mod relations;
mod sqlite;
mod sqlite_decimal;
mod value;

pub use compiler::dynamic_predicate;
use compiler::*;
pub use executor::*;
use mutation_support::*;
pub use ordering::*;
use plan::*;
pub use policy::*;
pub use postgres::*;
pub use predicate::*;
pub use queryset::*;
pub use relations::*;
pub use sqlite::*;
pub(crate) use sqlite_decimal::validate_sqlite_decimal_schema;
use sqlite_decimal::{is_exact_decimal, reject_sqlite_decimal_field, validate_sqlite_decimal_plan};
pub use value::*;

#[cfg(test)]
mod tests;
