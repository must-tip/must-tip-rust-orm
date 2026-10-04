# Sessions, cookies, and tokens

Authentication is framework-owned in v1.0.0. Developers select enabled mechanisms in settings; normal views are authenticated by default.

## Session authentication

```toml
[security.authentication]
backends = ["session"]
```

After your login logic proves a user's identity, issue a session through the framework authentication service:

```rust
let issued = auth_service
    .create_session(user_id, ["profile:read".to_owned()])
    .await?;
```

The framework owns session credential storage, expiry, revocation, scope loading, principal construction, and CSRF validation for unsafe cookie-authenticated requests.

## Bearer authentication

```toml
[security.authentication]
backends = ["bearer"]
```

Issue an opaque bearer token:

```rust
let issued = auth_service
    .create_bearer_token(user_id, ["orders:read".to_owned()])
    .await?;
```

Clients send:

```http
Authorization: Bearer <token>
```

## Hybrid authentication

```toml
[security.authentication]
backends = ["session", "bearer"]
```

This supports browser sessions and API/mobile bearer tokens in the same application.

## Signed cookie identity

The framework can also verify framework-issued signed authentication cookies using a runtime secret configured outside the repository.

## Fail-closed chains

Missing credentials allow the next configured backend to try. Credentials that are present but invalid reject the request; the framework does not silently downgrade to another backend.
