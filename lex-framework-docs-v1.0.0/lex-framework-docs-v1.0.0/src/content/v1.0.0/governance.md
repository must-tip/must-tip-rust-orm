# Governance, Risk, Compliance Controls, and Evidence

This document records engineering controls and gaps. It does not assert legal compliance, certification, regulatory approval, penetration-test completion, or independent security assessment.

## Control matrix

| Area | Implemented control | Evidence in source | Remaining operational evidence |
|---|---|---|---|
| Secrets | DB URL excluded from committed config; production `.env` disabled | `crate/db/src/config.rs`, `.gitignore`, `.env.example` | Secret-manager policy and rotation evidence |
| DB transport | PostgreSQL `VerifyFull` default | `crate/db/src/config.rs` | Production CA/SAN connection evidence |
| SQL injection | Parameter binds, bounded identifier/type/default/CHECK validation, typed value/schema checks, audited SQLx 0.9 `AssertSqlSafe` boundary | `crate/db_controller/src/query.rs`, `entity.rs`, ``lex audit sql`` | Compiler + real PostgreSQL tests, SAST review |
| Tenant isolation | Fail-closed tenant QuerySet + legacy repository guard | `query.rs`, `repository.rs` | Service authorization/tenant-resolution tests |
| Privileged global access | Application-provided authorization capability | `GlobalScopeAuthorization` in `query.rs` | Authorization implementation and policy evidence |
| Database tenant enforcement | PostgreSQL RLS SQL and transaction-local tenant context | `security.rs`, `transaction.rs` | Applied policies, least-privilege DB role, cross-tenant tests |
| Concurrency | Row-lock transaction requirement, savepoints, bounded DB retry, protected/incremented version metadata | `query.rs`, `transaction.rs`, `model_state.rs` | Load/concurrency test results |
| REST input/authorization policy | Serializer type/field validation; allowlisted filters/search/order; bounded inputs; application-owned `RestAccessPolicy` | `rest.rs` | Framework-adapter auth tests and gateway rate limits |
| Cursor integrity | HMAC-SHA256 signed/versioned/bounded cursor with key rotation | `rest.rs` | Secret-manager/key-rotation evidence |
| Sensitive model output | Credential hash/token fields excluded; encrypted fields write-only in policy | `crate/models/src/user.rs`, `rest.rs` | External API response tests |
| Passwords | Argon2id PHC hashes and redacted debug | `field.rs` | Authentication rate-limit/account policy evidence |
| Migration integrity | UP+DOWN SHA-256, drift rejection, tx advisory lock | `crate/migration` | Staging/production execution records |
| Migration dependency safety | Deterministic typed graph, missing/cycle rejection | `graph.rs`, `operation.rs` | Generated-plan review |
| Unsafe memory | Workspace forbids unsafe Rust | root `Cargo.toml` | Successful compiler/lint run |
| Dependency governance | One direct-version source at workspace root; workspace/dynamic-SQL/source audits in release gate | root/member `Cargo.toml`, ``lex audit workspace``, ``lex audit sql``, ``lex audit source`` | Reviewed lockfile + advisory/license/source scans |

## Risk register

| Risk | Severity | Status / mitigation |
|---|---|---|
| No compiler/test execution in this repair environment | High | Open until `lex verify framework` passes with Rust 1.97.1 |
| PostgreSQL behavior not executed here | High | Open until `lex verify postgres` passes against disposable target-version PostgreSQL |
| No concrete Axum/Actix/tonic adapter or application auth/permission implementation | High for an API service | Intentional boundary; implement in the owning service using its established auth contracts |
| Global tenant authorization policy is application supplied | High if implemented weakly | ORM requires a capability; service must bind it to authoritative authorization and audit it |
| RLS is generated but production DB role/policy application is deployment-owned | High for sensitive multi-tenancy | Apply reviewed RLS and test using least-privilege production-equivalent role |
| Legacy `ManyToMany<T>` stores integer IDs regardless target key type | Medium | Preserved API compatibility; use relation metadata/`Prefetched` for hydration until breaking redesign is approved |
| Implicit primary key on `TenantRolePermission` is not represented in Rust struct | Medium | Preserved; explicit/composite key decision requires domain approval |
| `TenantRolePermission` has no tenant column in its public model contract | High for direct cross-tenant access | Do not expose generic tenant-scoped access to this bridge as proof of tenant isolation. Restrict it behind tenant-authorized domain queries/joins until a domain-approved tenant key/composite FK schema change is designed and migrated. |
| Typed destructive migration operations can have dependent objects not encoded in one operation | High for rollback | Require explicit companion operations or paired reviewed SQL for indexes/FKs/checks/triggers/views and test the complete reverse plan before production use. |
| Typed migrations coexist with SQL migrations and are not a full Django autodetector | Medium | Use immutable SQL for unsupported operations; evolve one engine deliberately after compiler/integration evidence |
| No arbitrary deep relation/subquery/expression compiler | Medium | One-hop typed relation `EXISTS`, aggregates, CASE, and basic ranking windows exist; use reviewed service-owned SQL only for requirements beyond the supported AST |
| No automatic topology-aware shard/consistency router | Medium/High at scale | Explicit primary/read-replica pools exist; automatic sharding, residency routing, lag-based consistency, and failover decisions require deployment contracts |
| `encrypted` metadata is not transparent field encryption | High if misunderstood | Treat as sensitivity/write-only metadata; add envelope encryption only with explicit KMS/key lifecycle contract |
| Media crate lacks storage/scanning/transcoding implementation | High for media workloads | Do not deploy it as a complete media service |
| No vulnerability/license audit executed here | Medium | Run organizational supply-chain tools after lockfile generation |

## Known gaps

- Compiler-backed verification and PostgreSQL execution evidence are not available in this environment.
- The transport-neutral DRF layer intentionally does not implement HTTP routers/viewsets, service authentication/authorization, throttling, content negotiation, or OpenAPI because no authoritative application web/auth contract was supplied.
- The query compiler supports typed one-hop relation `EXISTS`/`NOT EXISTS`, filtered aggregates, `CASE/WHEN`, and basic deterministic ranking windows, but does not claim arbitrary correlated subqueries, recursive relation paths, custom window frames, or every PostgreSQL expression/function.
- Explicit read-replica pools are implemented. Automatic shard routing, residency policy, replica-lag consistency decisions, and topology-aware failover are not claimed without deployment contracts.
- `TenantRolePermission` needs a domain-approved tenant-key strategy before generic direct access can be treated as independently tenant-isolated.
- Legacy `ManyToMany<T>` ID storage remains compatibility debt.
- Full migration autodetection/state optimization, schema reverse engineering, COPY/streaming APIs, and production media processing are not claimed.

## Evidence produced in this change

- typed QuerySet/expression/lookup/query-executor infrastructure, composite unique upserts, relation `EXISTS` filters, annotation/window/CASE projections, and source tests;
- tenant fail-closed query semantics, authorization-gated global scope, and RLS helpers;
- transaction/savepoint/row-lock/retry/post-commit primitives;
- optimistic concurrency and transactional dirty-model save;
- metadata-driven relation join/prefetch primitives;
- transport-neutral serializer/filter/search/order/page/cursor policy, `RestAccessPolicy`, HMAC cursor tests, and overposting guards;
- typed forward/reverse migration operations (including model indexes/composite uniqueness/CHECK/FKs) and deterministic dependency graph;
- destructive PostgreSQL integration suite guarded by explicit disposable-database confirmation;
- explicit primary/read-replica connection pools plus read-only executor guards;
- centralized direct dependencies, Rust toolchain pin, SQLx dynamic-SQL audit, and release scripts.

## Evidence not produced in this environment

- successful `cargo fmt`, `cargo check`, Clippy, unit/integration tests, or release build;
- PostgreSQL integration test output;
- load/soak/failover/chaos tests;
- SAST/DAST/fuzzing results;
- dependency vulnerability/license/source-policy results;
- penetration test or independent code audit;
- regulatory/legal assessment.

## Change governance

Treat model schema, migration SQL/operations, RLS/global-scope policy, TLS policy, password/key handling, serializer exposure, and dependency changes as security-sensitive. Require peer review, immutable migration history, locked reproducible builds, staging validation, production rollback decision, and retained evidence for every release.

## lex_rest risk/evidence update

- **Authentication integration risk:** concrete OAuth/OIDC/JWT/API-key/session verification is intentionally not invented; production evidence must identify the service-owned backend, issuer/key/session validation controls, rotation and revocation behavior.
- **Distributed-control risk:** production throttle and idempotency implementations require atomic external storage contracts and operational evidence for availability, encryption, retention and fail-open/fail-closed policy.
- **Proxy trust risk:** deployments terminating TLS upstream must document trusted proxy boundaries and forwarding-header sanitization; `lex_rest` does not trust client proxy headers automatically.
- **WebSocket risk:** WSS origin allowlists are fail-closed by default; deployments must provide approved origins, connection/message limits and disconnect/abuse monitoring.
- **Verification evidence:** source-policy audits pass in this environment; compiler/Clippy/test/release/PostgreSQL evidence must be produced by Rust-enabled CI before production promotion.
