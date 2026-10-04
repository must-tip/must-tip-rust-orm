# lex_rest REST Framework

`crate/lex_rest` is the application/transport layer for this workspace. It depends on the existing `db_controller` and `models` crates and does not duplicate ORM, serializer, filter, pagination, transaction, tenant, or permission-table definitions.

## Architecture

The request path is:

1. transport adapter constructs a bounded `ApiRequest`;
2. pre-context middleware validates transport/request limits;
3. ordered authentication backends produce one immutable `Principal` or reject presented invalid credentials;
4. the application `TenantResolver` resolves tenant context from authenticated state;
5. immutable `RequestContext` carries request ID, trace ID, principal, tenant, peer address, transport security, and deadline;
6. view-level `PermissionExpr` is enforced;
7. distributed throttle policies are acquired atomically;
8. mutation idempotency reserves an opaque key hash/fingerprint when configured;
9. `APIView` dispatches GET/POST/PUT/PATCH/DELETE, or `ModelViewSet` delegates to the canonical `GenericAPIView<T>`;
10. existing `RestAccessPolicy<T>`, `SerializerPolicy`, `RestQueryPolicy`, QuerySet tenant scope, and object permissions are enforced before persistence;
11. response middleware, audit, request ID, exception mapping, and authentication challenges are applied;
12. the optional Axum adapter converts the framework-neutral response to HTTP.

The ORM remains authoritative for SQL, transactions, optimistic concurrency, row scope, serializer field metadata, filtering, and pagination.

## Modules

- `request` — immutable principal/request context and bounded request parsing.
- `response` — framework-neutral response body/status/header representation.
- `authentication` — ordered backend interface, fail-closed presented-credential behavior, trusted-boundary adapter.
- `permissions` — composable view/object policies and the concrete `DatabasePermissionBackend` backed by existing `User`, `Membership`, `TenantRole`, `TenantRolePermission`, and `PermissionDefinition` models.
- `throttling` — distributed atomic rate-limit storage contract and fail-open/fail-closed policy.
- `exceptions` — stable safe error documents and ORM/REST error mapping.
- `views` — `APIView` dispatch and secure defaults.
- `generic` — `GenericAPIView<T>` plus list/create/retrieve/update/destroy generic views.
- `mixins` — CRUD mixins delegating to `GenericAPIView<T>`.
- `viewsets` — DRF-style `ModelViewSet<T>` collection/detail actions and conventional route generation.
- `routing` — deterministic static/parameter route matching with traversal-safe decoding.
- `middleware` — tenant resolution, request limits, security headers, runtime pipeline, audit/idempotency integration.
- `schema` — OpenAPI 3.2 document metadata for HTTP operations.
- `websocket` — WSS/origin/subprotocol/message-size/idle-timeout contracts and secured sessions.
- `adapters::axum` — optional Axum HTTP/WebSocket adapter and optional direct Rustls WSS serving.

## Permission resolution

`DatabasePermissionBackend` uses the existing authorization schema. A tenant permission is effective only when:

- the principal maps to an active user identity;
- an active `Membership` exists for that user and request tenant;
- the bridge grant belongs to that membership;
- the referenced `TenantRole` still exists in that same tenant;
- the referenced `PermissionDefinition` exists and its name matches the requested permission.

Configurable full-access shortcuts for existing global-admin and tenant-admin model flags are explicit. Service-scope authorization is disabled by default. API-key scopes are an upper bound on the owning user's database permissions by default, so an API key cannot inherit a permission that was not granted to that key. Empty permission conjunctions fail closed. Object permission evaluation rejects cross-tenant resources unless the existing global-admin backend authorizes the override.

No new permission tables are introduced by `lex_rest`.

## Authentication

`lex_rest` intentionally defines authentication interfaces rather than inventing token verification. An owning service must supply its authoritative OAuth/OIDC/JWT/API-key/mTLS/session implementation through `AuthenticationBackend` or a validated `TrustedPrincipalResolver`.

Backends must return:

- `Abstain` only when their credential mechanism is absent;
- `Authenticated(Principal)` only after cryptographic/session verification;
- `Rejected` when credentials for their mechanism were presented but invalid.

A rejected backend stops the chain, preventing fallback from converting invalid credentials into another identity.

## HTTP behavior

`APIView` supports GET, POST, PUT, PATCH, DELETE, HEAD and OPTIONS. Unimplemented methods return 405 with an `Allow` header. HEAD reuses GET semantics and removes the response body. OPTIONS reports enabled methods.

`GenericAPIView<T>` reuses the existing model serializer/query/access policies. PUT uses full-update validation and PATCH uses partial-update validation. CRUD writes use the configured mutation adapter and preserve ORM tenant/optimistic-concurrency controls. The default ORM update adapter performs update, reload, and full model validation in one `AtomicTransaction`; validation failure rolls back before invalid state can commit.

`ModelViewSet<T>` exposes the conventional collection/detail action mapping without creating a second persistence path.

## Idempotency

Mutation endpoints may require `Idempotency-Key`. The framework hashes the raw key and fingerprints method, path, query string, principal, tenant, content type and body. Raw idempotency keys are not retained by the framework.

`IdempotencyStore::reserve` must be atomic for `(namespace, key_hash)` and `complete` must enforce the reservation token. A production store must encrypt/protect persisted response bodies according to the application's data-classification policy. Exactly-once business effects across an independent idempotency store and database cannot be guaranteed without a shared transactional design; use the authoritative database/outbox pattern where that guarantee is required.

## Distributed throttling

`DistributedThrottleStore` is an atomic storage contract, not an in-memory production limiter. The owning deployment may implement Redis or another strongly consistent counter backend. The framework never guesses cache endpoints, credentials, cluster behavior, or Lua/function deployment policy.

Policies support principal, tenant, principal+tenant, peer-IP and endpoint scope. Fail-open/fail-closed behavior is explicit per policy. Peer-IP policies require a trusted transport-supplied peer address; missing peer metadata is a configuration error and is never collapsed into a shared anonymous bucket.

## WebSocket / WSS

WebSocket routes run authentication, tenant resolution, permission checks and throttling before upgrade. Routing accepts the standards-compatible HTTP/1.1 `GET` upgrade and HTTP/2+ `CONNECT` form. The optional Axum adapter applies the configured WebSocket message limit at both frame and message boundaries before handing the session to application code. The default security policy:

- requires secure transport (`wss://` in production);
- requires an `Origin` header;
- has an empty allowlist, therefore fails closed until configured;
- accepts only canonical `https://host[:port]` origins when secure transport is required;
- validates subprotocol tokens;
- limits message size;
- applies an idle receive timeout;
- rejects/attempts close code 1009 for oversized inbound messages.

The generic session exposes heartbeat policy but does not cancel arbitrary adapter receive futures to synthesize heartbeats because adapter cancellation-safety is not universally guaranteed. Applications/adapters that require proactive ping scheduling must implement it using their transport's documented cancellation model.

## Axum adapter

Features:

- no adapter: default feature set, framework-neutral core only;
- `axum`: HTTP + WebSocket adapter;
- `axum-rustls`: Axum plus direct Rustls HTTPS/WSS serving.

`AxumAdapter::transport_security` must be set from a trusted deployment boundary. It must never trust arbitrary `X-Forwarded-Proto` input. If TLS terminates at a proxy, that proxy must strip untrusted forwarding headers and the application must receive transport/peer metadata through a trusted adapter configuration.

## Security controls

- no request body or authorization/cookie/API-key values in `Debug`;
- no query-string values in `ApiRequest` debug output;
- bounded body, URI, header, query and WebSocket message sizes;
- secure transport required by default;
- authentication failures can emit backend `WWW-Authenticate` challenges;
- security headers apply to success, matched failures, pre-context failures and routing failures;
- tenant-aware ORM operations remain fail-closed;
- application row/payload policy and principal/object permission policy remain independent checks;
- HMAC-signed ORM cursor pagination remains unchanged and is reused by generic views;
- request trace IDs accept only valid non-zero W3C `traceparent` identifiers;
- audit logs contain identifiers/metadata, not request bodies or credential values.

## Deployment

Run the full release gate before promotion:

```bash
cargo generate-lockfile
lex verify framework
```

The gate separately compiles/tests `lex_rest` with `axum-rustls` enabled because optional adapters are not exercised by a default-feature workspace build.

For disposable PostgreSQL integration tests:

```bash
export TEST_DATABASE_URL='postgres://.../disposable_test_db'
export CONFIRM_DISPOSABLE_DATABASE=yes
lex verify postgres
```

Do not use a production or valuable database.

## Rollback

`lex_rest` adds no database migration or authorization table by itself. A service rollout can therefore roll back to its previous application binary as long as that service has not independently introduced API/schema migrations. Keep route/API versioning stable during rollout and retain the previous image/lockfile. If an application adds migrations alongside REST endpoints, follow the migration-specific reverse/roll-forward plan before binary rollback.

## Remaining integration contracts

These are deliberately application/deployment-owned because the project does not contain an authoritative contract for them:

- concrete OAuth/OIDC/JWT/session/API-key validation;
- Redis/other distributed throttle implementation;
- distributed idempotency storage implementation;
- reverse-proxy trusted peer/proxy-header topology;
- application RBAC/ABAC rules beyond existing permission records;
- transport-specific OpenAPI publication and ingress configuration.

They must not be replaced with guessed credentials, endpoints or security policy inside this crate.
