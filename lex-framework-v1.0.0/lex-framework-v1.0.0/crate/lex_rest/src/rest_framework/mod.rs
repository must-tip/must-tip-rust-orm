pub mod audit;
pub mod authentication;
pub mod exceptions;
pub mod generic;
pub mod idempotency;
pub mod middleware;
pub mod mixins;
pub mod permissions;
pub mod request;
pub mod response;
pub mod routing;
pub mod schema;
pub mod throttling;
pub mod views;
pub mod viewsets;
pub mod websocket;

#[cfg(feature = "axum")]
pub mod adapters;

pub mod prelude {
    pub use crate::rest_framework::{
        audit::{AuditEvent, AuditOutcome, AuditSink, TracingAuditSink},
        authentication::{
            AuthenticationBackend, AuthenticationChain, AuthenticationOutcome,
            BearerTokenAuthentication, BearerTokenAuthenticationConfig, BearerTokenResolver,
            CookieAuthentication, CookieAuthenticationConfig, CookieIdentity,
            CookiePrincipalResolver, SessionAuthentication, SessionAuthenticationConfig,
            SessionIdentity, SessionStore, TrustedBoundaryAuthentication, TrustedPrincipalResolver,
        },
        exceptions::{ApiError, ApiResult},
        generic::{
            CreateAPIView, DestroyAPIView, GenericAPIView, ListAPIView, ListCreateAPIView,
            ModelFactory, ModelMutationAdapter, OrmModelMutationAdapter, RestModel,
            RetrieveAPIView, RetrieveDestroyAPIView, RetrieveUpdateAPIView,
            RetrieveUpdateDestroyAPIView, UpdateAPIView,
        },
        idempotency::{
            IdempotencyInput, IdempotencyPolicy, IdempotencyRequirement, IdempotencyReservation,
            IdempotencyStore,
        },
        middleware::{
            CorsMiddleware, NoTenantResolver, RequestLimitsMiddleware, RequestMiddleware,
            RestApplication, RestRuntime, RestRuntimeBuilder, SecurityHeadersMiddleware,
            TenantResolver,
        },
        permissions::{
            DatabasePermissionBackend, DatabasePermissionConfig, DenyAllPermissionBackend,
            PermissionBackend, PermissionDecision, PermissionExpr, PermissionRepository,
            PermissionResource, Permissions, ScopePermissionBackend, ScopePermissionConfig,
            SqlxPermissionRepository,
        },
        request::{ApiRequest, Principal, PrincipalKind, RequestContext, TransportSecurity},
        response::{ApiBody, ApiResponse},
        routing::{RouteEndpoint, RouteSpec, RouteTable},
        schema::{ApiSchema, OperationMetadata, SecuritySchemeMetadata},
        throttling::{
            DistributedThrottleStore, ThrottleEngine, ThrottleKey, ThrottleLease, ThrottlePolicy,
        },
        views::{APIView, ViewAccess, ViewPermissions, ViewServices},
        viewsets::{ModelViewSet, ViewSetActions, ViewSetOperationMetadata},
        websocket::{
            SecuredWebSocketSession, WebSocketCloseFrame, WebSocketMessage,
            WebSocketSecurityPolicy, WebSocketSession, WebSocketView,
        },
    };
}
