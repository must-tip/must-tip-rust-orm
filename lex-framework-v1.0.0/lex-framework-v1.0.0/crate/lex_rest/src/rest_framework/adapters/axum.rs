use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ws::{CloseFrame as AxumCloseFrame, Message, WebSocket, WebSocketUpgrade},
    http::{HeaderMap, Method, Uri},
    response::{IntoResponse, Response},
    routing::any,
};
use std::sync::Arc;

#[cfg(feature = "axum-rustls")]
use std::{net::SocketAddr, path::Path};

use crate::rest_framework::{
    exceptions::{ApiError, ApiResult},
    middleware::RestApplication,
    request::{ApiRequest, TransportSecurity},
    response::{ApiBody, ApiResponse},
    routing::RouteEndpoint,
    websocket::{
        SecuredWebSocketSession, WebSocketCloseFrame, WebSocketMessage, WebSocketSession,
        WebSocketView, validate_subprotocols,
    },
};

#[derive(Clone)]
pub struct AxumAdapter {
    application: Arc<RestApplication>,
    max_body_bytes: usize,
    transport_security: TransportSecurity,
}

impl AxumAdapter {
    pub fn new(application: Arc<RestApplication>) -> Self {
        Self {
            application,
            max_body_bytes: ApiRequest::DEFAULT_MAX_BODY_BYTES,
            transport_security: TransportSecurity::Insecure,
        }
    }

    pub fn max_body_bytes(mut self, max_body_bytes: usize) -> ApiResult<Self> {
        if max_body_bytes == 0 || max_body_bytes > 64 * 1024 * 1024 {
            return Err(ApiError::configuration("invalid Axum body limit"));
        }
        self.max_body_bytes = max_body_bytes;
        Ok(self)
    }

    /// Set how the adapter was reached. Use `Secure` for direct TLS or a trusted TLS-terminating
    /// ingress. Never derive this from an untrusted `X-Forwarded-Proto` header.
    pub fn transport_security(mut self, security: TransportSecurity) -> Self {
        self.transport_security = security;
        self
    }

    pub fn router(&self) -> ApiResult<Router> {
        let app = Arc::clone(&self.application);
        let body_limit = self.max_body_bytes;
        let security = self.transport_security;
        let mut router = Router::new().fallback(move |request: axum::extract::Request| {
            let app = Arc::clone(&app);
            async move { dispatch_http(app, request, body_limit, security).await }
        });

        for route in self.application.routes().routes() {
            let RouteEndpoint::WebSocket(view) = route.endpoint else {
                continue;
            };
            let application = Arc::clone(&self.application);
            let path = route.path;
            let security = self.transport_security;
            let protocols = view.subprotocols();
            validate_subprotocols(&protocols)?;
            router = router.route(
                &path,
                any(
                    move |ws: WebSocketUpgrade, method: Method, headers: HeaderMap, uri: Uri| {
                        let application = Arc::clone(&application);
                        let view = Arc::clone(&view);
                        let protocols = protocols.clone();
                        async move {
                            websocket_upgrade(
                                application,
                                view,
                                ws,
                                WebSocketHandshake {
                                    method,
                                    headers,
                                    uri,
                                    security,
                                    protocols,
                                },
                            )
                            .await
                        }
                    },
                ),
            );
        }
        Ok(router)
    }
}

async fn dispatch_http(
    application: Arc<RestApplication>,
    request: axum::extract::Request,
    max_body_bytes: usize,
    security: TransportSecurity,
) -> Response {
    let (parts, body) = request.into_parts();
    let body = match to_bytes(body, max_body_bytes).await {
        Ok(body) => body.to_vec(),
        Err(_) => {
            return to_axum_response(
                ApiError::payload_too_large(max_body_bytes).to_response(uuid::Uuid::now_v7()),
            );
        }
    };
    let request = match ApiRequest::new(
        parts.method,
        parts.uri,
        parts.headers,
        body,
        None,
        security,
        max_body_bytes,
    ) {
        Ok(request) => request,
        Err(error) => return to_axum_response(error.to_response(uuid::Uuid::now_v7())),
    };
    to_axum_response(application.handle(request).await)
}

struct WebSocketHandshake {
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    security: TransportSecurity,
    protocols: Vec<String>,
}

async fn websocket_upgrade(
    application: Arc<RestApplication>,
    view: Arc<dyn WebSocketView>,
    ws: WebSocketUpgrade,
    handshake: WebSocketHandshake,
) -> Response {
    let WebSocketHandshake {
        method,
        headers,
        uri,
        security,
        protocols,
    } = handshake;
    let request = match ApiRequest::new(
        method.clone(),
        uri,
        headers,
        Vec::new(),
        None,
        security,
        1024,
    ) {
        Ok(request) => request,
        Err(error) => return to_axum_response(error.to_response(uuid::Uuid::now_v7())),
    };
    let resolved = match application.routes().resolve(&method, request.path()) {
        Ok(resolved) => resolved,
        Err(error) => return to_axum_response(error.to_response(uuid::Uuid::now_v7())),
    };
    if !matches!(resolved.spec.endpoint, RouteEndpoint::WebSocket(_)) {
        return to_axum_response(ApiError::not_found().to_response(uuid::Uuid::now_v7()));
    }
    let request = request.with_path_params(resolved.path_params);
    let (context, services) = match application
        .runtime()
        .authorize_websocket(view.as_ref(), &request)
        .await
    {
        Ok(value) => value,
        Err(error) => return to_axum_response(error.to_response(uuid::Uuid::now_v7())),
    };

    let policy = view.security_policy();
    if let Err(error) = policy.validate() {
        return to_axum_response(error.to_response(context.request_id()));
    }
    let ws = ws
        .max_message_size(policy.max_message_bytes)
        .max_frame_size(policy.max_message_bytes);
    let ws = if protocols.is_empty() {
        ws
    } else {
        ws.protocols(protocols)
    };
    ws.on_upgrade(move |socket| async move {
        let selected_subprotocol = socket
            .protocol()
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let raw_session = AxumWebSocketSession {
            socket,
            selected_subprotocol,
            max_message_bytes: policy.max_message_bytes,
        };
        let session = match SecuredWebSocketSession::new(Box::new(raw_session), policy.clone()) {
            Ok(session) => session,
            Err(error) => {
                tracing::error!(
                    request_id = %context.request_id(),
                    error_code = error.code(),
                    "invalid WebSocket security policy after upgrade"
                );
                return;
            }
        };
        if let Err(error) = view
            .connected(&request, &context, &services, Box::new(session))
            .await
        {
            tracing::warn!(
                request_id = %context.request_id(),
                error_code = error.code(),
                "WebSocket handler terminated with an error"
            );
        }
    })
    .into_response()
}

fn to_axum_response(response: ApiResponse) -> Response {
    let (status, headers, body) = response.into_parts();
    let body = match body {
        ApiBody::Empty => Body::empty(),
        ApiBody::Json(value) => match serde_json::to_vec(&value) {
            Ok(value) => Body::from(value),
            Err(_) => Body::empty(),
        },
        ApiBody::Text(value) => Body::from(value),
        ApiBody::Bytes(value) => Body::from(value),
    };
    let mut response = Response::new(body);
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

struct AxumWebSocketSession {
    socket: WebSocket,
    selected_subprotocol: Option<String>,
    max_message_bytes: usize,
}

#[async_trait::async_trait]
impl WebSocketSession for AxumWebSocketSession {
    async fn receive(&mut self) -> ApiResult<Option<WebSocketMessage>> {
        let Some(message) = self.socket.recv().await else {
            return Ok(None);
        };
        let message = message
            .map_err(|_| ApiError::bad_request("websocket_error", "WebSocket receive failed"))?;
        let message = match message {
            Message::Text(value) => WebSocketMessage::Text(value.to_string()),
            Message::Binary(value) => WebSocketMessage::Binary(value.to_vec()),
            Message::Ping(value) => WebSocketMessage::Ping(value.to_vec()),
            Message::Pong(value) => WebSocketMessage::Pong(value.to_vec()),
            Message::Close(frame) => {
                WebSocketMessage::Close(frame.map(|frame| WebSocketCloseFrame {
                    code: frame.code,
                    reason: frame.reason.to_string(),
                }))
            }
        };
        let bytes = match &message {
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
        Ok(Some(message))
    }

    async fn send(&mut self, message: WebSocketMessage) -> ApiResult<()> {
        let message = match message {
            WebSocketMessage::Text(value) => Message::Text(value.into()),
            WebSocketMessage::Binary(value) => Message::Binary(value.into()),
            WebSocketMessage::Ping(value) => Message::Ping(value.into()),
            WebSocketMessage::Pong(value) => Message::Pong(value.into()),
            WebSocketMessage::Close(frame) => Message::Close(frame.map(|frame| AxumCloseFrame {
                code: frame.code,
                reason: frame.reason.into(),
            })),
        };
        self.socket
            .send(message)
            .await
            .map_err(|_| ApiError::internal("WebSocket send failed"))
    }

    async fn close(&mut self, frame: Option<WebSocketCloseFrame>) -> ApiResult<()> {
        self.send(WebSocketMessage::Close(frame)).await
    }

    fn selected_subprotocol(&self) -> Option<&str> {
        self.selected_subprotocol.as_deref()
    }
}

#[cfg(feature = "axum-rustls")]
pub async fn serve_wss(
    router: Router,
    address: SocketAddr,
    certificate_pem: impl AsRef<Path>,
    private_key_pem: impl AsRef<Path>,
) -> ApiResult<()> {
    use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
    let certificates = tokio::fs::read(certificate_pem.as_ref())
        .await
        .map_err(|_| ApiError::configuration("cannot read TLS certificate file"))?;
    let key = tokio::fs::read(private_key_pem.as_ref())
        .await
        .map_err(|_| ApiError::configuration("cannot read TLS private key file"))?;
    let certificates = CertificateDer::pem_slice_iter(&certificates)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ApiError::configuration("invalid TLS certificate PEM"))?;
    let key = PrivateKeyDer::from_pem_slice(&key)
        .map_err(|_| ApiError::configuration("invalid TLS private key PEM"))?;
    // Explicit provider selection avoids Rustls panicking when an integrator enables another
    // provider for an unrelated client or server in the same dependency graph.
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| ApiError::configuration("unsupported TLS protocol configuration"))?
    .with_no_client_auth()
    .with_single_cert(certificates, key)
    .map_err(|_| ApiError::configuration("invalid TLS certificate/key configuration"))?;
    let config = axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(config));
    axum_server::bind_rustls(address, config)
        .serve(router.into_make_service())
        .await
        .map_err(|error| ApiError::internal(format!("WSS server failed: {error}")))
}
