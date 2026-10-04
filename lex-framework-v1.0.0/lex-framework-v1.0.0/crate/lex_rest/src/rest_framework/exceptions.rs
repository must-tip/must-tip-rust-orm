use db_controller::{ModelError, RestError};
use http::{HeaderValue, StatusCode};
use serde::Serialize;
use std::collections::BTreeMap;
use thiserror::Error;
use uuid::Uuid;

use crate::rest_framework::response::ApiResponse;

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ErrorDocument {
    pub code: String,
    pub detail: String,
    pub request_id: Uuid,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Error, Clone)]
#[error("{code}: {detail}")]
pub struct ApiError {
    status: StatusCode,
    code: String,
    detail: String,
    metadata: Box<ErrorMetadata>,
}

#[derive(Debug, Clone)]
struct ErrorMetadata {
    fields: BTreeMap<String, Vec<String>>,
    retry_after_seconds: Option<u64>,
    allow: Vec<String>,
    log_detail: Option<String>,
}

impl ApiError {
    pub(crate) fn from_storage(error: sqlx::Error) -> Self {
        Self::from(ModelError::Database(error))
    }
    pub fn new(status: StatusCode, code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            detail: detail.into(),
            metadata: Box::new(ErrorMetadata {
                fields: BTreeMap::new(),
                retry_after_seconds: None,
                allow: Vec::new(),
                log_detail: None,
            }),
        }
    }

    pub fn bad_request(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, detail)
    }

    pub fn authentication(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "authentication_failed", detail)
    }

    pub fn permission_denied(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "permission_denied", detail)
    }

    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", "resource not found")
    }

    pub fn conflict(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, code, detail)
    }

    /// Attach a Retry-After delay in seconds to an actionable transient error.
    pub fn with_retry_after(mut self, seconds: u64) -> Self {
        self.metadata.retry_after_seconds = Some(seconds.max(1));
        self
    }

    pub fn timeout(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::GATEWAY_TIMEOUT, "request_timeout", detail)
    }

    pub fn payload_too_large(limit: usize) -> Self {
        Self::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload_too_large",
            format!("request body exceeds the {limit}-byte limit"),
        )
    }

    pub fn unsupported_media_type() -> Self {
        Self::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "unsupported request media type",
        )
    }

    pub fn method_not_allowed(allow: impl IntoIterator<Item = String>) -> Self {
        let mut error = Self::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
            "HTTP method is not allowed for this resource",
        );
        error.metadata.allow = allow.into_iter().collect();
        error
    }

    pub fn throttled(retry_after_seconds: u64) -> Self {
        let mut error = Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            "throttled",
            "request rate limit exceeded",
        );
        error.metadata.retry_after_seconds = Some(retry_after_seconds.max(1));
        error
    }

    pub fn internal(log_detail: impl Into<String>) -> Self {
        let log_detail = log_detail.into();
        let mut error = Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "an internal server error occurred",
        );
        error.metadata.log_detail = Some(log_detail);
        error
    }

    pub fn configuration(log_detail: impl Into<String>) -> Self {
        let mut error = Self::internal(log_detail);
        error.code = "configuration_error".to_owned();
        error
    }

    pub const fn status(&self) -> StatusCode {
        self.status
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn safe_detail(&self) -> &str {
        &self.detail
    }

    pub fn log_detail(&self) -> Option<&str> {
        self.metadata.log_detail.as_deref()
    }

    pub fn with_fields(mut self, fields: BTreeMap<String, Vec<String>>) -> Self {
        self.metadata.fields = fields;
        self
    }

    pub fn to_response(&self, request_id: Uuid) -> ApiResponse {
        if let Some(detail) = self.log_detail() {
            tracing::error!(
                request_id = %request_id,
                error_code = %self.code,
                error_detail = %detail,
                "request failed"
            );
        }
        let document = ErrorDocument {
            code: self.code.clone(),
            detail: self.detail.clone(),
            request_id,
            fields: self.metadata.fields.clone(),
        };
        let mut response = ApiResponse::json(self.status, &document)
            .unwrap_or_else(|_| ApiResponse::empty(StatusCode::INTERNAL_SERVER_ERROR));
        if let Some(seconds) = self.metadata.retry_after_seconds
            && let Ok(value) = HeaderValue::from_str(&seconds.to_string())
        {
            response
                .headers_mut()
                .insert(http::header::RETRY_AFTER, value);
        }
        if !self.metadata.allow.is_empty()
            && let Ok(value) = HeaderValue::from_str(&self.metadata.allow.join(", "))
        {
            response.headers_mut().insert(http::header::ALLOW, value);
        }
        response
    }
}

impl From<RestError> for ApiError {
    fn from(error: RestError) -> Self {
        // Policy/serializer wrappers must preserve the storage error redaction boundary.
        if matches!(
            error.code.as_str(),
            "database_error" | "database" | "migration_error" | "internal_error"
        ) {
            return Self::internal("storage operation failed");
        }
        let mut mapped = Self::bad_request(error.code, error.detail);
        if mapped.code == "not_found" {
            mapped.status = StatusCode::NOT_FOUND;
        }
        if matches!(
            mapped.code.as_str(),
            "conflict" | "concurrent_modification" | "multiple_objects_returned"
        ) {
            mapped.status = StatusCode::CONFLICT;
        }
        if mapped.code == "permission_denied" {
            mapped.status = StatusCode::FORBIDDEN;
        }
        mapped.metadata.fields = error.fields;
        mapped
    }
}

impl From<ModelError> for ApiError {
    fn from(error: ModelError) -> Self {
        match error {
            ModelError::NotFound => Self::not_found(),
            ModelError::MultipleObjectsReturned => Self::conflict(
                "multiple_objects_returned",
                "query returned more than one object",
            ),
            ModelError::TenantScopeRequired { .. } | ModelError::PermissionDenied(_) => {
                Self::permission_denied("resource access is not permitted")
            }
            ModelError::Validation(detail) => Self::bad_request("validation_error", detail),
            ModelError::InvalidQuery(detail) => Self::bad_request("invalid_query", detail),
            ModelError::Conversion(detail) => Self::bad_request("conversion_error", detail),
            ModelError::Conflict(detail) => Self::conflict("conflict", detail),
            ModelError::ConcurrentModification => Self::conflict(
                "concurrent_modification",
                "resource changed concurrently; reload and retry",
            ),
            ModelError::QueryTimeout { .. } => Self::timeout("database query deadline exceeded"),
            ModelError::TransactionRequired => Self::conflict(
                "transaction_required",
                "operation requires an explicit transaction",
            ),
            ModelError::UnsafeMutation(_) => Self::conflict(
                "unsafe_mutation",
                "operation requires a safer execution context",
            ),
            other => Self::internal(format!("model operation failed with code {}", other.code())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_errors_do_not_expose_log_detail() {
        let error = ApiError::internal("password=secret database host=db.internal");
        let response = error.to_response(Uuid::nil());
        let rendered = format!("{:?}", response.body());
        assert!(!rendered.contains("secret"));
        assert!(!rendered.contains("db.internal"));
    }
}
