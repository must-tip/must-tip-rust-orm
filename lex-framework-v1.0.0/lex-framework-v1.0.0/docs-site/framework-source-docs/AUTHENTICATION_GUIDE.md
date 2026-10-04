> **Historical documentation.** These pre-audit notes may describe obsolete APIs or ownership. Use the [current developer documentation](README.md) and executable acceptance examples for the supported contract.

# Authentication Guide

Lex authentication is framework-owned. Application developers select authentication mechanisms in `Config.toml`; they do **not** implement `SessionStore`, bearer-token resolvers, cookie resolvers, or an authentication chain.

## 1. Choose authentication in settings

Browser application using server-side sessions:

```toml
[security.authentication]
backends = ["session"]
```

API/mobile application using opaque bearer tokens:

```toml
[security.authentication]
backends = ["bearer"]
```

Hybrid application:

```toml
[security.authentication]
backends = ["session", "bearer"]
```

Stateless signed-cookie authentication:

```toml
[security.authentication]
backends = ["cookie"]
```

Multiple mechanisms are evaluated in configured order. Credentials that are present but invalid reject the request instead of falling through to another backend.

## 2. Protected views are the default

Every normal `APIView` requires authentication automatically:

```rust
use async_trait::async_trait;
use http::{Method, StatusCode};
use lex_framework::views::{
    ApiRequest, ApiResponse, ApiResult, APIView, RequestContext, ViewServices,
};

pub struct ProfileView;

#[async_trait]
impl APIView for ProfileView {
    fn name(&self) -> &'static str { "profile" }
    fn allowed_methods(&self) -> Vec<Method> { vec![Method::GET] }

    async fn get(
        &self,
        _request: &ApiRequest,
        context: &RequestContext,
        _services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        let user_id = context.principal().user_id();
        ApiResponse::json(StatusCode::OK, &serde_json::json!({"user_id": user_id}))
    }
}
```

There is no authentication boilerplate in the view.

## 3. Public views

Only intentionally anonymous endpoints opt out:

```rust
use lex_framework::views::ViewAccess;

fn access(&self) -> ViewAccess {
    ViewAccess::Public
}
```

Example:

```rust
pub struct LandingView;

#[async_trait]
impl APIView for LandingView {
    fn name(&self) -> &'static str { "landing" }
    fn allowed_methods(&self) -> Vec<Method> { vec![Method::GET] }

    fn access(&self) -> ViewAccess {
        ViewAccess::Public
    }

    async fn get(
        &self,
        _request: &ApiRequest,
        _context: &RequestContext,
        _services: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        ApiResponse::json(StatusCode::OK, &serde_json::json!({"public": true}))
    }
}
```

Public endpoints can still receive an authenticated principal when valid credentials are supplied; they simply do not require one.

## 4. Session authentication

```toml
[security.authentication.session]
cookie_name = "lex_session"
require_secure_transport = false # production must be true
csrf_protection = true
csrf_header = "x-csrf-token"
ttl_secs = 86400
```

Lex automatically creates/uses `_lex_auth_sessions` in the configured SQLite or PostgreSQL database. Raw session credentials are never persisted; only SHA-256 digests are stored.

Unsafe browser requests require the configured CSRF header when session authentication is active.

## 5. Bearer authentication

```toml
[security.authentication.bearer]
scheme = "Bearer"
ttl_secs = 2592000
```

Lex automatically creates/uses `_lex_auth_tokens`. Clients send:

```http
Authorization: Bearer <opaque-token>
```

Only token digests are persisted. Expiry and revocation are checked automatically.

## 6. Signed-cookie authentication

```toml
[security.authentication.cookie]
cookie_name = "lex_auth"
require_secure_transport = false # production must be true
csrf_protection = true
csrf_header = "x-csrf-token"
ttl_secs = 86400
secret_env = "LEX_AUTH_COOKIE_SECRET"
```

Set the signing secret outside `Config.toml`:

```bash
export LEX_AUTH_COOKIE_SECRET='at-least-32-random-bytes-from-a-secret-manager'
```

Lex signs the cookie payload with HMAC-SHA256, validates expiry, and enforces CSRF for unsafe methods.

## 7. Issuing credentials from login/business logic

The framework exposes `FrameworkAuthService`. This is useful when your own login view has already verified a password, passkey, OAuth callback, or another identity proof.

```rust
use lex_framework::authentication::FrameworkAuthService;

let issued = auth.create_session(user_id, ["profile:read".to_owned()]).await?;
let set_cookie = auth.session_cookie(&issued, true);
```

Bearer token:

```rust
let token = auth
    .create_bearer_token(user_id, ["api:read".to_owned()])
    .await?;
```

Signed cookie:

```rust
let cookie = auth.create_cookie(user_id, ["profile:read".to_owned()])?;
```

Credential verification, hashing, expiry, revocation and backend composition remain framework responsibilities.

## 8. Production behavior

Production validation requires secure transport and CSRF for cookie/session authentication. Use TLS directly or set `LEX_TRUSTED_TLS_TERMINATION=true` only when the service is actually behind a trusted TLS ingress/proxy.

Do not place cookie signing secrets, database passwords, or bearer credentials in `Config.toml` or source control.

## 9. Authentication versus authorization

Authentication determines the principal. Authorization remains expressed through `PermissionExpr` when a view needs more than the default authenticated/public distinction.

Examples include tenant membership, admin access, ownership and named permissions. A public view uses `ViewAccess::Public`; a normal view is authenticated by default.
