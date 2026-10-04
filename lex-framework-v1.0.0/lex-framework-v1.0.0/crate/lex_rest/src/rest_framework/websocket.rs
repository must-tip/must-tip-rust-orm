use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, time::Duration};

use crate::rest_framework::{
    exceptions::{ApiError, ApiResult},
    permissions::PermissionExpr,
    request::{ApiRequest, RequestContext, TransportSecurity},
    throttling::ThrottlePolicy,
    views::ViewServices,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebSocketMessage {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Close(Option<WebSocketCloseFrame>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSocketCloseFrame {
    pub code: u16,
    pub reason: String,
}

#[async_trait]
pub trait WebSocketSession: Send {
    async fn receive(&mut self) -> ApiResult<Option<WebSocketMessage>>;
    async fn send(&mut self, message: WebSocketMessage) -> ApiResult<()>;
    async fn close(&mut self, frame: Option<WebSocketCloseFrame>) -> ApiResult<()>;
    fn selected_subprotocol(&self) -> Option<&str>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketSecurityPolicy {
    pub require_secure_transport: bool,
    pub allowed_origins: BTreeSet<String>,
    pub allow_missing_origin: bool,
    pub max_message_bytes: usize,
    pub idle_timeout: Duration,
    pub heartbeat_interval: Duration,
}

impl Default for WebSocketSecurityPolicy {
    fn default() -> Self {
        Self {
            require_secure_transport: true,
            allowed_origins: BTreeSet::new(),
            allow_missing_origin: false,
            max_message_bytes: 1024 * 1024,
            idle_timeout: Duration::from_secs(120),
            heartbeat_interval: Duration::from_secs(30),
        }
    }
}

impl WebSocketSecurityPolicy {
    pub fn validate(&self) -> ApiResult<()> {
        if self.max_message_bytes == 0 || self.max_message_bytes > 16 * 1024 * 1024 {
            return Err(ApiError::configuration(
                "invalid WebSocket message size limit",
            ));
        }
        if self.idle_timeout < Duration::from_secs(10)
            || self.idle_timeout > Duration::from_secs(3600)
        {
            return Err(ApiError::configuration("invalid WebSocket idle timeout"));
        }
        if self.heartbeat_interval < Duration::from_secs(5)
            || self.heartbeat_interval >= self.idle_timeout
        {
            return Err(ApiError::configuration(
                "invalid WebSocket heartbeat interval",
            ));
        }
        for origin in &self.allowed_origins {
            let canonical = canonical_origin(origin)
                .map_err(|_| ApiError::configuration("invalid allowed WebSocket origin"))?;
            if canonical != *origin {
                return Err(ApiError::configuration(
                    "WebSocket allowed origins must use canonical scheme://host[:port] form",
                ));
            }
            if self.require_secure_transport && !canonical.starts_with("https://") {
                return Err(ApiError::configuration(
                    "secure WebSocket policy requires HTTPS origins",
                ));
            }
        }
        Ok(())
    }

    pub fn enforce_handshake(&self, request: &ApiRequest) -> ApiResult<()> {
        self.validate()?;
        if self.require_secure_transport
            && request.transport_security() != TransportSecurity::Secure
        {
            return Err(ApiError::permission_denied(
                "secure WSS transport is required",
            ));
        }
        let origin = request
            .headers()
            .get(http::header::ORIGIN)
            .and_then(|value| value.to_str().ok());
        match origin {
            Some(origin) => {
                let canonical = canonical_origin(origin)
                    .map_err(|_| ApiError::permission_denied("WebSocket origin is not allowed"))?;
                if self.allowed_origins.contains(&canonical) {
                    Ok(())
                } else {
                    Err(ApiError::permission_denied(
                        "WebSocket origin is not allowed",
                    ))
                }
            }
            None if self.allow_missing_origin => Ok(()),
            None => Err(ApiError::permission_denied(
                "WebSocket Origin header is required",
            )),
        }
    }

    pub fn validate_message(&self, message: &WebSocketMessage) -> ApiResult<()> {
        let bytes = match message {
            WebSocketMessage::Text(value) => value.len(),
            WebSocketMessage::Binary(value)
            | WebSocketMessage::Ping(value)
            | WebSocketMessage::Pong(value) => value.len(),
            WebSocketMessage::Close(Some(frame)) => frame.reason.len(),
            WebSocketMessage::Close(None) => 0,
        };
        if bytes > self.max_message_bytes {
            return Err(ApiError::payload_too_large(self.max_message_bytes));
        }
        Ok(())
    }
}

/// Enforces the configured message-size and idle-timeout policy for every transport adapter.
/// Proactive heartbeat scheduling remains application-owned because canceling an arbitrary
/// adapter's in-flight receive future is not guaranteed to be cancellation-safe.
pub struct SecuredWebSocketSession {
    inner: Box<dyn WebSocketSession>,
    policy: WebSocketSecurityPolicy,
}

impl SecuredWebSocketSession {
    pub fn new(
        inner: Box<dyn WebSocketSession>,
        policy: WebSocketSecurityPolicy,
    ) -> ApiResult<Self> {
        policy.validate()?;
        Ok(Self { inner, policy })
    }
}

#[async_trait]
impl WebSocketSession for SecuredWebSocketSession {
    async fn receive(&mut self) -> ApiResult<Option<WebSocketMessage>> {
        let received = tokio::time::timeout(self.policy.idle_timeout, self.inner.receive())
            .await
            .map_err(|_| ApiError::timeout("WebSocket idle timeout exceeded"))??;
        if let Some(message) = received.as_ref()
            && let Err(error) = self.policy.validate_message(message)
        {
            if let Err(close_error) = self
                .inner
                .close(Some(WebSocketCloseFrame {
                    code: 1009,
                    reason: "message too large".to_owned(),
                }))
                .await
            {
                tracing::warn!(
                    error_code = close_error.code(),
                    "failed to close oversized WebSocket session"
                );
            }
            return Err(error);
        }
        Ok(received)
    }

    async fn send(&mut self, message: WebSocketMessage) -> ApiResult<()> {
        self.policy.validate_message(&message)?;
        self.inner.send(message).await
    }

    async fn close(&mut self, frame: Option<WebSocketCloseFrame>) -> ApiResult<()> {
        if let Some(frame) = frame.as_ref() {
            self.policy
                .validate_message(&WebSocketMessage::Close(Some(frame.clone())))?;
        }
        self.inner.close(frame).await
    }

    fn selected_subprotocol(&self) -> Option<&str> {
        self.inner.selected_subprotocol()
    }
}

fn canonical_origin(origin: &str) -> Result<String, ()> {
    let url = url::Url::parse(origin).map_err(|_| ())?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(());
    }
    Ok(url.origin().ascii_serialization())
}

#[async_trait]
pub trait WebSocketView: Send + Sync {
    fn name(&self) -> &'static str;

    fn permission_policy(&self) -> PermissionExpr {
        PermissionExpr::Authenticated
    }

    fn throttle_policies(&self) -> Vec<ThrottlePolicy> {
        Vec::new()
    }

    fn security_policy(&self) -> WebSocketSecurityPolicy {
        WebSocketSecurityPolicy::default()
    }

    fn subprotocols(&self) -> Vec<String> {
        Vec::new()
    }

    async fn connected(
        &self,
        request: &ApiRequest,
        context: &RequestContext,
        services: &ViewServices,
        session: Box<dyn WebSocketSession>,
    ) -> ApiResult<()>;
}

pub fn validate_subprotocols(protocols: &[String]) -> ApiResult<()> {
    if protocols.len() > 16 {
        return Err(ApiError::configuration("too many WebSocket subprotocols"));
    }
    let mut unique = BTreeSet::new();
    for protocol in protocols {
        if protocol.is_empty()
            || protocol.len() > 128
            || !protocol.bytes().all(|byte| {
                matches!(byte, 0x21..=0x7e)
                    && !matches!(
                        byte,
                        b'(' | b')'
                            | b'<'
                            | b'>'
                            | b'@'
                            | b','
                            | b';'
                            | b':'
                            | b'\\'
                            | b'"'
                            | b'/'
                            | b'['
                            | b']'
                            | b'?'
                            | b'='
                            | b'{'
                            | b'}'
                            | b' '
                            | b'\t'
                    )
            })
        {
            return Err(ApiError::configuration("invalid WebSocket subprotocol"));
        }
        if !unique.insert(protocol) {
            return Err(ApiError::configuration("duplicate WebSocket subprotocol"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{HeaderMap, HeaderValue, Method};

    #[test]
    fn secure_policy_requires_canonical_https_origin() {
        let mut policy = WebSocketSecurityPolicy::default();
        policy
            .allowed_origins
            .insert("https://app.example.com".to_owned());
        assert!(policy.validate().is_ok());
        policy
            .allowed_origins
            .insert("http://app.example.com".to_owned());
        assert!(policy.validate().is_err());
    }

    #[test]
    fn handshake_rejects_unlisted_origin() {
        let mut policy = WebSocketSecurityPolicy::default();
        policy
            .allowed_origins
            .insert("https://app.example.com".to_owned());
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::ORIGIN,
            HeaderValue::from_static("https://evil.example"),
        );
        let request = ApiRequest::new(
            Method::GET,
            "/ws".parse().expect("uri"),
            headers,
            Vec::new(),
            None,
            TransportSecurity::Secure,
            1024,
        )
        .expect("request");
        assert!(policy.enforce_handshake(&request).is_err());
    }
}
