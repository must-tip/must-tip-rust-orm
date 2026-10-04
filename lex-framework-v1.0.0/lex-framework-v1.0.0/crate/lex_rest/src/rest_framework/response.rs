use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use serde::Serialize;
use serde_json::Value;

use crate::rest_framework::exceptions::{ApiError, ApiResult};

#[derive(Debug, Clone, PartialEq)]
pub enum ApiBody {
    Empty,
    Json(Value),
    Text(String),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApiResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: ApiBody,
}

impl ApiResponse {
    pub fn new(status: StatusCode, body: ApiBody) -> Self {
        Self {
            status,
            headers: HeaderMap::new(),
            body,
        }
    }

    pub fn empty(status: StatusCode) -> Self {
        Self::new(status, ApiBody::Empty)
    }

    pub fn json<T: Serialize>(status: StatusCode, value: &T) -> ApiResult<Self> {
        let value = serde_json::to_value(value)
            .map_err(|_| ApiError::internal("response serialization failed"))?;
        let mut response = Self::new(status, ApiBody::Json(value));
        response.headers.insert(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        Ok(response)
    }

    pub fn text(status: StatusCode, value: impl Into<String>) -> Self {
        let mut response = Self::new(status, ApiBody::Text(value.into()));
        response.headers.insert(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        response
    }

    pub const fn status(&self) -> StatusCode {
        self.status
    }

    pub const fn body(&self) -> &ApiBody {
        &self.body
    }

    pub const fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    pub fn with_header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.headers.insert(name, value);
        self
    }

    pub fn set_request_id(&mut self, request_id: uuid::Uuid) {
        if let Ok(value) = HeaderValue::from_str(&request_id.to_string()) {
            self.headers
                .insert(HeaderName::from_static("x-request-id"), value);
        }
    }

    pub fn into_parts(self) -> (StatusCode, HeaderMap, ApiBody) {
        (self.status, self.headers, self.body)
    }
}
