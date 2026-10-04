# Middleware and HTTP security

Most applications do not need to register security middleware manually. `FrameworkSettings::rest_middleware()` builds the standard stack from `Config.toml`.

## Built-in middleware from settings

The generated stack includes configured:

- request URI/header limits;
- security response headers;
- Content-Security-Policy;
- Permissions-Policy;
- HSTS on secure transport;
- CORS.

Example:

```toml
[security.headers]
enabled = true
csp = "default-src 'self'; object-src 'none'; frame-ancestors 'none'"
permissions_policy = "camera=(), microphone=(), geolocation=()"
hsts = "max-age=31536000; includeSubDomains"

[security.cors]
enabled = true
allowed_origins = ["https://app.example.com"]
allowed_methods = ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"]
allowed_headers = ["Authorization", "Content-Type", "X-CSRF-Token"]
allow_credentials = true
```

## Custom middleware

Use the framework alias rather than depending on REST internals:

```rust
use async_trait::async_trait;
use lex_framework::middleware::Middleware;
use lex_framework::views::{ApiRequest, ApiResponse, ApiResult, RequestContext};

pub struct ResponseMarker;

#[async_trait]
impl Middleware for ResponseMarker {
    async fn before(&self, _request: &ApiRequest) -> ApiResult<()> {
        Ok(())
    }

    async fn after(
        &self,
        _request: &ApiRequest,
        _context: &RequestContext,
        response: &mut ApiResponse,
    ) -> ApiResult<()> {
        response.headers_mut().insert(
            "x-example".parse().expect("header name"),
            "enabled".parse().expect("header value"),
        );
        Ok(())
    }
}
```

`before` runs before handler execution. `after` runs once an authenticated request context exists. `after_pre_context` is available for response behavior even when authentication/context construction failed.

## Recommended rule

Prefer settings for standard security behavior. Write custom middleware only for cross-cutting application behavior such as correlation metadata, custom compliance headers, or domain-specific request checks.
