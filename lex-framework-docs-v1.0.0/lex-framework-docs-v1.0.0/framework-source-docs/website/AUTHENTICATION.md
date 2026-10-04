# Authentication, sessions, cookies, and bearer tokens

Authentication is framework-owned. Developers select mechanisms in `Config.toml`; views simply receive an authenticated `RequestContext`.

## Browser session authentication

```toml
[security.authentication]
backends = ["session"]
```

Lex stores an opaque session digest in the active SQLite/PostgreSQL database. The browser receives the opaque session token as a cookie. Unsafe browser methods use CSRF protection.

### Issue a session after login

Your login code proves identity; the framework creates the credential:

```rust
let issued = auth_service
    .create_session(user.id, ["profile:read".to_owned(), "profile:edit".to_owned()])
    .await?;

let set_cookie = auth_service.session_cookie(&issued, true);
```

Return `set_cookie` as the HTTP `Set-Cookie` value and return/expose the CSRF token using your chosen frontend contract.

### Revoke a session

```rust
auth_service.revoke_session(&raw_session_token).await?;
```

## Bearer authentication

```toml
[security.authentication]
backends = ["bearer"]
```

The client sends:

```http
Authorization: Bearer <token>
```

Issue a token:

```rust
let issued = auth_service
    .create_bearer_token(
        user.id,
        ["orders:read".to_owned(), "orders:create".to_owned()],
    )
    .await?;
```

Only a digest is persisted by the framework.

## Hybrid browser + API

```toml
[security.authentication]
backends = ["session", "bearer"]
```

A missing session cookie allows the bearer backend to try. An invalid credential is rejected rather than silently downgraded to another backend.

## Signed cookie authentication

```toml
[security.authentication]
backends = ["cookie"]

[security.authentication.cookie]
secret_env = "LEX_AUTH_COOKIE_SECRET"
```

Provide a runtime secret of at least 32 bytes:

```bash
export LEX_AUTH_COOKIE_SECRET='replace-with-a-long-random-secret'
```

Issue the cookie with:

```rust
let issued = auth_service.create_cookie(user.id, ["profile:read".to_owned()])?;
let set_cookie = auth_service.auth_cookie(&issued, true);
```

## Public endpoints

Authentication can be globally enabled while a specific view remains anonymous-accessible:

```rust
fn permissions(&self) -> ViewPermissions {
    ViewPermissions::new(Permissions::public())
}
```

This is the normal pattern for login and registration endpoints.

## Production rules

For session/cookie authentication in production, secure transport and CSRF remain required. The framework validates these settings at startup rather than silently weakening them.
