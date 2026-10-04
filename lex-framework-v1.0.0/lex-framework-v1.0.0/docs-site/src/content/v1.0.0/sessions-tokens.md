# Sessions, bearer tokens and signed cookies

Lex constructs authentication backends from FrameworkSettings. Applications prove identity (for example by verifying a stored PasswordField), then issue credentials through FrameworkAuthService. Lex does not provide an automatic enrollment/login endpoint or infer authorization from a user-supplied tenant identifier.

Enable mechanisms explicitly:

```toml
[security.authentication]
backends = ["session", "bearer"]
[security.authentication.session]
csrf_protection = true
require_secure_transport = true
```

`FrameworkSettings::authentication(&database).await` returns the authentication chain and credential service. Bind that chain to RestRuntimeBuilder. The public protected endpoint example is compile-tested in `crate/lex_framework/tests/auth_http.rs`; the independent tenant API is in `integration/consumer/tests/tenant_http.rs`.

After verifying an existing user's identity, `service.create_session(user_id, scopes).await` returns a one-time raw token, CSRF token and expiry. `service.session_cookie(&issued, true)` formats a Secure, HttpOnly cookie. Deliver it as Set-Cookie, and deliver the CSRF value through an authenticated channel appropriate to your client. Default session cookie is lex_session; expiry is 86,400 seconds. Unsafe POST/PUT/PATCH/DELETE requests require the matching x-csrf-token. GET is safe without that token. A missing/wrong CSRF value is 403; expired, revoked or invalid session credentials are 401. `revoke_session` invalidates future requests; applications own logout response/cookie clearing. Tokens are hashed in database storage and must never appear in logs.

`create_bearer_token(user_id, scopes).await` returns a one-time opaque bearer token. Clients send `Authorization: Bearer TOKEN`. Default expiry is 2,592,000 seconds. `revoke_bearer_token` invalidates it. Bearer-only authentication has no cookie CSRF requirement. Multiple concurrent sessions/tokens are supported; do not treat issuance as revocation of older credentials.

For stateless signed cookies, enable cookie and configure its secret environment setting:

```toml
[security.authentication]
backends = ["cookie"]
[security.authentication.cookie]
secret_env = "LEX_AUTH_COOKIE_SECRET"
csrf_protection = true
require_secure_transport = true
```

Supply at least 32 unpredictable secret bytes through that environment variable; never put deployment secrets in source. `create_cookie` returns the signed value and CSRF token; `auth_cookie(&issued, true)` formats the cookie. Default name is lex_auth and expiry is 86,400 seconds. HMAC-SHA256 protects subject, scopes and expiry against alteration. Payloads are visible, not encrypted. Expiry is enforced, but individual stateless cookies cannot be revoked through the database service. Use sessions if immediate per-credential revocation is required. Rotating the signing secret invalidates old cookies.

Zero/overflowing lifetimes, disabled issuance mechanisms and missing signing secrets return errors. Production validation requires secure transport and CSRF for session/cookie backends. Do not trust spoofable forwarded headers to establish TLS; see deployment.

With multiple backends every supplied credential is evaluated. A valid early credential cannot mask a later invalid credential; invalid credentials yield 401, and two valid mechanisms yield an ambiguous-credentials 401. Only all-abstaining backends leave a request anonymous. Protected views then challenge it; public views must still reject supplied invalid credentials. Authentication proves a principal, not permission to every resource. Use live database RBAC and server-verified tenant context for SaaS authorization. See security and RBAC next.
