> **Historical documentation.** These pre-audit notes may describe obsolete APIs or ownership. Use the [current developer documentation](README.md) and executable acceptance examples for the supported contract.

# REST Framework Guide

`lex_rest` is a Django REST Framework-style application layer built around framework-neutral request/response types and optional transport adapters.

## Core pipeline

```text
HTTP adapter
   ↓
ApiRequest
   ↓
RestRuntime
   ├── authentication
   ├── tenant resolution
   ├── permissions
   ├── request middleware
   ├── throttling
   ├── idempotency
   └── audit
   ↓
RouteTable
   ↓
APIView / GenericAPIView / ModelViewSet
   ↓
db_controller QuerySet
```

## Build a runtime

```rust
let runtime = RestRuntimeBuilder::new(Arc::new(permission_backend))
    .authentication(authentication_chain)
    .tenant_resolver(Arc::new(tenant_resolver))
    .require_secure_transport(true)
    .request_timeout(Duration::from_secs(30))?
    .build()?;
```

## Authentication

Implement `AuthenticationBackend` for authoritative mechanisms such as JWT/API key/service identity.

Use `AuthenticationChain::new(...)` to compose them.

An invalid credential must return rejection rather than abstention.

## Permission backend

Implement `PermissionBackend` or use the SQLx-backed database permission backend.

Do not infer permissions from request-controlled fields.

## Generic API view

`GenericAPIView<T>` owns model-level REST policy for list/retrieve/create/update/destroy.

Configure:

- serializer policy;
- query policy;
- access policy;
- model mutation adapter;
- lookup field/path parameter;
- owner field;
- object permission.

## ViewSets

`ModelViewSet<T>` creates collection/detail view behavior and route specifications.

```rust
let core = Arc::new(GenericAPIView::<MyModel>::new(pool.clone()));
let viewset = Arc::new(ModelViewSet::new("my-model", core));
let routes = viewset.route_specs("/api/models", "model")?;
```

## Route table

```rust
let mut table = RouteTable::new();
table.register_many(routes)?;

let openapi = table.build_openapi("My API", "1.0.0")?;
```

## Idempotency

Use `IdempotencyPolicy` with a durable/shared `IdempotencyStore` for multi-instance deployments.

HTTP replay protection does not replace domain-level transactional idempotency.

## Throttling

Use `ThrottleEngine` with a distributed `DistributedThrottleStore`.

## Security middleware

Use request-size/cardinality limits and security headers. Require secure transport in production unless TLS is authoritatively terminated at a trusted adapter/boundary that marks the request secure.

## WebSockets

Use `WebSocketSecurityPolicy`, `WebSocketView`, and `SecuredWebSocketSession` for authenticated/authorized message handling.

## Axum

The project currently includes an Axum adapter. Keep transport conversion in the adapter and business policy in framework/service components.
