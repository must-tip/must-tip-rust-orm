//! Transport-neutral primitives inspired by Django REST Framework.
//!
//! This module deliberately does not own HTTP routing. It converts already-parsed request
//! parameters and JSON payloads into validated ORM operations, keeping serialization, filtering
//! and pagination policies reusable across Axum, Actix, gRPC gateways and background workers.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use sha2::Sha256;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt,
    sync::Arc,
};

use crate::{
    ComparisonOperator, DbValue, Entity, ModelError, OrderBy, Predicate, QueryExecutor, QuerySet,
    SortDirection, dynamic_predicate, effective_schema,
};

mod error;
pub use error::*;
mod policy;
pub use policy::*;
mod serializer;
pub use serializer::*;
mod filtering;
pub use filtering::*;
mod cursor;
pub use cursor::*;
mod query;
pub use query::*;
mod ordering;
#[cfg(test)]
mod tests;
