use async_trait::async_trait;
use http::Method;
use sha2::{Digest, Sha256};
use std::time::Duration;

use crate::rest_framework::{
    exceptions::{ApiError, ApiResult},
    request::{ApiRequest, RequestContext},
    response::ApiResponse,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdempotencyRequirement {
    Disabled,
    Optional,
    Required,
}

#[derive(Debug, Clone)]
pub struct IdempotencyPolicy {
    pub requirement: IdempotencyRequirement,
    pub ttl: Duration,
}

impl Default for IdempotencyPolicy {
    fn default() -> Self {
        Self {
            requirement: IdempotencyRequirement::Disabled,
            ttl: Duration::from_secs(24 * 60 * 60),
        }
    }
}

impl IdempotencyPolicy {
    pub fn required(ttl: Duration) -> ApiResult<Self> {
        let policy = Self {
            requirement: IdempotencyRequirement::Required,
            ttl,
        };
        policy.validate()?;
        Ok(policy)
    }

    pub fn validate(&self) -> ApiResult<()> {
        if self.ttl < Duration::from_secs(60) || self.ttl > Duration::from_secs(7 * 24 * 60 * 60) {
            return Err(ApiError::configuration(
                "idempotency TTL must be within 60 seconds..=7 days",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum IdempotencyReservation {
    Started { token: String },
    Replay(ApiResponse),
    Conflict,
    InProgress { retry_after: Duration },
}

/// Distributed stores must make `reserve` atomic for `(namespace, key_hash)`. `complete` must only
/// succeed for the reservation token returned by `reserve`, preventing an expired worker from
/// overwriting a newer request's result.
#[async_trait]
pub trait IdempotencyStore: Send + Sync {
    async fn reserve(
        &self,
        namespace: &str,
        key_hash: &str,
        request_fingerprint: &str,
        ttl: Duration,
    ) -> ApiResult<IdempotencyReservation>;

    async fn complete(
        &self,
        namespace: &str,
        key_hash: &str,
        reservation_token: &str,
        response: &ApiResponse,
        ttl: Duration,
    ) -> ApiResult<()>;

    async fn abandon(
        &self,
        namespace: &str,
        key_hash: &str,
        reservation_token: &str,
    ) -> ApiResult<()>;
}

#[derive(Debug, Clone)]
pub struct IdempotencyInput {
    pub namespace: String,
    pub key_hash: String,
    pub fingerprint: String,
}

pub fn prepare_idempotency(
    request: &ApiRequest,
    context: &RequestContext,
    namespace: &str,
    policy: &IdempotencyPolicy,
) -> ApiResult<Option<IdempotencyInput>> {
    policy.validate()?;
    if !is_mutation(request.method()) {
        return Ok(None);
    }
    if matches!(policy.requirement, IdempotencyRequirement::Disabled) {
        return Ok(None);
    }
    if namespace.is_empty() || namespace.len() > 128 {
        return Err(ApiError::configuration("invalid idempotency namespace"));
    }
    let raw_key = request
        .headers()
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok());
    let Some(raw_key) = raw_key else {
        return if matches!(policy.requirement, IdempotencyRequirement::Required) {
            Err(ApiError::bad_request(
                "idempotency_key_required",
                "Idempotency-Key header is required",
            ))
        } else {
            Ok(None)
        };
    };
    if !(8..=255).contains(&raw_key.len())
        || !raw_key.is_ascii()
        || raw_key.chars().any(char::is_control)
    {
        return Err(ApiError::bad_request(
            "invalid_idempotency_key",
            "Idempotency-Key must contain 8..=255 visible ASCII bytes",
        ));
    }

    let mut key_hasher = Sha256::new();
    key_hasher.update(namespace.as_bytes());
    key_hasher.update([0]);
    key_hasher.update(raw_key.as_bytes());
    let key_hash = lowercase_hex(&key_hasher.finalize());

    let mut request_hasher = Sha256::new();
    request_hasher.update(request.method().as_str().as_bytes());
    request_hasher.update([0]);
    request_hasher.update(request.path().as_bytes());
    request_hasher.update([0]);
    if let Some(query) = request.uri().query() {
        request_hasher.update(query.as_bytes());
    }
    request_hasher.update([0]);
    request_hasher.update(context.principal().subject().as_bytes());
    request_hasher.update([0]);
    if let Some(tenant_id) = context.tenant_id() {
        request_hasher.update(tenant_id.as_bytes());
    }
    request_hasher.update([0]);
    if let Some(content_type) = request.headers().get(http::header::CONTENT_TYPE) {
        request_hasher.update(content_type.as_bytes());
    }
    request_hasher.update([0]);
    request_hasher.update(request.body());
    let fingerprint = lowercase_hex(&request_hasher.finalize());

    Ok(Some(IdempotencyInput {
        namespace: namespace.to_owned(),
        key_hash,
        fingerprint,
    }))
}

fn is_mutation(method: &Method) -> bool {
    method == Method::POST
        || method == Method::PUT
        || method == Method::PATCH
        || method == Method::DELETE
}

fn lowercase_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rest_framework::request::{Principal, TransportSecurity};
    use http::{HeaderMap, HeaderValue};

    #[test]
    fn idempotency_fingerprint_binds_principal_and_payload() {
        let mut headers = HeaderMap::new();
        headers.insert("idempotency-key", HeaderValue::from_static("request-123"));
        let request = ApiRequest::new(
            Method::POST,
            "/payments".parse().expect("uri"),
            headers,
            br#"{"amount":100}"#.to_vec(),
            None,
            TransportSecurity::Secure,
            1024,
        )
        .expect("request");
        let context = RequestContext::new(
            Principal::user(uuid::Uuid::new_v4(), "test", Vec::new()).expect("principal"),
            None,
            None,
            TransportSecurity::Secure,
            None,
            None,
        )
        .expect("context");
        let input = prepare_idempotency(
            &request,
            &context,
            "payments.create",
            &IdempotencyPolicy::required(Duration::from_secs(3600)).expect("policy"),
        )
        .expect("idempotency")
        .expect("input");
        assert_eq!(input.key_hash.len(), 64);
        assert_eq!(input.fingerprint.len(), 64);
        assert!(!input.key_hash.contains("request-123"));
    }

    #[test]
    fn idempotency_fingerprint_binds_query_string() {
        let mut headers = HeaderMap::new();
        headers.insert("idempotency-key", HeaderValue::from_static("request-456"));
        let request_a = ApiRequest::new(
            Method::POST,
            "/payments?capture=true".parse().expect("uri"),
            headers.clone(),
            br#"{"amount":100}"#.to_vec(),
            None,
            TransportSecurity::Secure,
            1024,
        )
        .expect("request");
        let request_b = ApiRequest::new(
            Method::POST,
            "/payments?capture=false".parse().expect("uri"),
            headers,
            br#"{"amount":100}"#.to_vec(),
            None,
            TransportSecurity::Secure,
            1024,
        )
        .expect("request");
        let context = RequestContext::new(
            Principal::user(uuid::Uuid::new_v4(), "test", Vec::new()).expect("principal"),
            None,
            None,
            TransportSecurity::Secure,
            None,
            None,
        )
        .expect("context");
        let policy = IdempotencyPolicy::required(Duration::from_secs(3600)).expect("policy");
        let a = prepare_idempotency(&request_a, &context, "payments.create", &policy)
            .expect("idempotency")
            .expect("input");
        let b = prepare_idempotency(&request_b, &context, "payments.create", &policy)
            .expect("idempotency")
            .expect("input");
        assert_ne!(a.fingerprint, b.fingerprint);
    }
}
