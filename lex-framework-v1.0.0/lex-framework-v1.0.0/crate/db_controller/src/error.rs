use chrono::ParseError as ChronoParseError;
use serde_json::Error as SerdeError;
use sqlx::Error as SqlxError;
use std::{
    io::Error as IoError,
    num::{ParseFloatError, ParseIntError},
};
use thiserror::Error;
use uuid::Error as UuidError;

pub type RepoResult<T> = Result<T, ModelError>;

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("database error: {0}")]
    Database(#[from] SqlxError),
    #[error("JSON error: {0}")]
    Json(#[from] SerdeError),
    #[error("UUID error: {0}")]
    Uuid(#[from] UuidError),
    #[error("datetime parse error: {0}")]
    DateTimeParse(#[from] ChronoParseError),
    #[error("I/O error: {0}")]
    Io(#[from] IoError),
    #[error("integer parse error: {0}")]
    ParseInt(#[from] ParseIntError),
    #[error("float parse error: {0}")]
    ParseFloat(#[from] ParseFloatError),
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("record not found")]
    NotFound,
    #[error("query returned more than one object")]
    MultipleObjectsReturned,
    #[error("tenant scope is required for model {model}")]
    TenantScopeRequired { model: &'static str },
    #[error("row locking requires an explicit database transaction")]
    TransactionRequired,
    #[error("unsafe bulk mutation rejected: {0}")]
    UnsafeMutation(String),
    #[error("query deadline exceeded after {millis}ms")]
    QueryTimeout { millis: u64 },
    #[error("optimistic concurrency conflict")]
    ConcurrentModification,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("invalid query: {0}")]
    InvalidQuery(String),
    #[error("database transaction committed, but post-commit callback failed: {0}")]
    PostCommit(String),
    #[error("missing required field: {0}")]
    MissingField(&'static str),
    #[error("type conversion failed: {0}")]
    Conversion(String),
    #[error("SQL generation error: {0}")]
    SqlBuildError(String),
    #[error("unexpected error: {0}")]
    Other(String),
}

impl ModelError {
    /// Stable machine-readable error code suitable for REST/gRPC adapters.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Database(_) => "database_error",
            Self::Json(_) => "json_error",
            Self::Uuid(_) | Self::DateTimeParse(_) | Self::ParseInt(_) | Self::ParseFloat(_) => {
                "invalid_value"
            }
            Self::Io(_) => "io_error",
            Self::Validation(_) => "validation_error",
            Self::NotFound => "not_found",
            Self::MultipleObjectsReturned => "multiple_objects_returned",
            Self::TenantScopeRequired { .. } => "tenant_scope_required",
            Self::TransactionRequired => "transaction_required",
            Self::UnsafeMutation(_) => "unsafe_mutation",
            Self::QueryTimeout { .. } => "query_timeout",
            Self::ConcurrentModification => "concurrent_modification",
            Self::Conflict(_) => "conflict",
            Self::PermissionDenied(_) => "permission_denied",
            Self::InvalidQuery(_) => "invalid_query",
            Self::PostCommit(_) => "post_commit_failure",
            Self::MissingField(_) => "missing_field",
            Self::Conversion(_) => "conversion_error",
            Self::SqlBuildError(_) => "sql_build_error",
            Self::Other(_) => "internal_error",
        }
    }
}
