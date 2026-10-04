# Release behavior matrix

This is an evidence inventory, not a claim of complete coverage. `release-results.json` records the final commands and exit codes; individual logs are authoritative. Prior `*-before.log` files preserve regressions that failed before fixes. PG tests use a disposable PostgreSQL 16 database. SQLite tests include memory and file-backed databases. An entry marked partial means only the stated behavior has execution evidence.

Legend: **yes** = behavior exercised by a passing test; **partial** = stated subset only; **unit** = compiler/policy/logic tests, not proof of actual database execution; **gap** = no authoritative test; **N/A** = unsupported/inapplicable. External tests copy the facade consumer outside the workspace. HTTP means Axum ServiceExt requests unless explicitly marked live TCP. Docs examples are executed only where a runner/example is named.

| Capability | PostgreSQL | SQLite | Unit / integration evidence | External consumer | HTTP | Documentation / limitation |
|---|---|---|---|---|---|---|
| Model derive, metadata, explicit IDs, timestamps | yes | yes | orm/model_derive; ORM suites | yes | indirect | concepts/models-and-queries; implicit ID metadata unit only |
| Qualified/nullable UUID and exact decimal storage/decoding | yes | yes | derive plus consumer row reads | yes | indirect | guides/databases; boundary matrix not exhaustive |
| Exact decimal numeric SQL | supported | explicitly rejected | consumer rejects lexical/rounded operations; storage unchanged | yes | N/A | count/null checks supported; no silent SQLite approximation |
| Unique/composite metadata and DB constraints | yes | yes | normalized_rbac; relation models | yes | partial | guides/rbac; ordinary custom indexes broader DB matrix remains partial |
| Invalid derive declarations | N/A | N/A | trybuild: duplicate PK, unknown option, nullable PK | compile pass facade | N/A | further malformed relation/index/conflict diagnostics need fixtures |
| Alias/facade derive resolution | yes | yes | macro expansion; facade external models | yes | indirect | concepts/architecture; retain library names |
| CRUD, partial field mutation | yes | yes | postgres_orm/sqlite_orm + consumer | yes | protected GET/POST/PUT/PATCH/DELETE both DBs | public custom APIView complete CRUD; built-in generic adapters still partial |
| Bulk create/update atomic failure | happy-path yes | failure/success yes | PG legacy; SQLite batch regressions | yes SQLite | gap | no per-row optimistic editor guarantee |
| Tenant bulk/update/delete predicates | partial | yes | scoped ORM mutation + hostile rows | yes | protected CRUD | cross-tenant bulk matrix on PG needs expansion |
| all/filter/chained/exclude/get/first/last/count/exists | yes | yes | ORM plus portable query stories | yes | list | concepts/models-and-queries |
| Ordering, multiple order fields, limit/offset/page | yes | yes | portable query stories | yes | partial | count/exists ignore slicing; cursor DB traversal partial |
| Projection/values-list/distinct | yes | yes | portable query stories | yes | gap | ordered distinct projection rejects unsafe combination |
| Sum/max and basic nondecimal aggregates | yes | yes | query stories plus compiler units | yes | gap | all aggregate functions/type combinations not exercised |
| SQL injection-looking values remain data | yes | yes | real insert/filter/update/read tests | yes | hostile title protected CRUD | identifiers reject malformed names separately |
| Identifier/type validation | compiler/unit | compiler/unit | query/schema/REST policy units | yes invalid ordering | partial | full property-based suite not implemented |
| Expression updates/increments | yes | unit/partial | PG legacy expression mutation | partial | gap | no CASE/window/annotation/subquery typed AST API |
| select_related/prefetch_related and missing/deleted relations | yes | yes | legacy PG + external relations | yes | gap | decimal/JSON/boolean fidelity; binary eager JSON unsupported |
| Cross-tenant relation references cannot hydrate | yes | yes | external adversarial relation fixture | yes | gap | no automatic relation property IO; full many-to-many/N+1 query counting gap |
| Soft delete/with_deleted/restore/hard delete | yes | yes | ORM + external lifecycle | yes | gap | unique keys remain reserved; exhaustive repeat/uniqueness matrix gap |
| Optimistic stale editor conflict and no-op/identity rules | yes | yes | Tracked consumer stories | yes | gap | concurrency conflict preserved; API write race matrix gap |
| Commit/rollback/savepoints | yes | yes | PG legacy callback/savepoint; SQLite file persistence | yes | PG mutation rollback | concepts/transactions |
| Post-commit callback/rolled-back savepoint callbacks | yes | N/A | PG legacy actual callback effects | gap | gap | PG AtomicTransaction contract |
| Serialization/deadlock retry, isolation/read-only/timeouts | unit/partial | PG options N/A | bounded/options validation | gap | gap | deterministic real contention tests remain required |
| RLS missing/correct/wrong tenant; commit/rollback pool reuse | yes | N/A | postgres_rls, restricted non-bypass role, same reused PID | gap | gap | guides/databases; transaction-local setting only |
| Normalized RBAC uniqueness/FKs/delete restrictions | yes | yes | normalized_rbac actual duplicate/invalid-reference inserts | yes SQLite | live role removal denies | guides/rbac |
| Tenant-crossed role assignment and parent tenant reassignment | yes | yes | normalized_rbac real DB rejection | yes SQLite | resolver mismatch denies | bridge composite FKs, RESTRICT |
| Constraint-shape verification | yes | yes | PG weakened same-name FK rejection; both backends reject missing bridge/grant unique keys; SQLite PRAGMA validation | indirect | N/A | installer is not a complete arbitrary schema-drift auditor |
| Legacy RBAC conversion, duplicate grants, privilege disagreement | yes | unsupported | PG normalized_rbac retains source, rejects conflicting sets/merge | gap | N/A | SQLite reviewed rebuild upgrade is a remaining gap |
| Multiple roles/overlap, membership/role/permission removal matrix | partial | partial | legacy permissions plus normalized live revocation | partial | revoked assignment | not every requested authorization story covered |
| Session/token issuance/hash storage/TTL/revocation | primary store compiled; HTTP SQLite | yes | facade auth runtime and auth_http | bearer tenant API | yes | guides/authentication; raw token not persisted |
| Invalid/expired/revoked/missing/wrong-scope credentials | partial | yes | facade auth_http | partial | yes | PostgreSQL-backed auth lifecycle needs separate HTTP run |
| Valid later auth backend; invalid later; ambiguous credentials | N/A | N/A | credential_composition | partial | yes | all supplied credentials evaluated; no invalid anonymous downgrade |
| Signed cookie tamper/signature/expiry/secret/encoding | N/A | N/A | facade runtime + cookie-integration | gap | tamper/CSRF yes | stateless per-cookie revocation unavailable |
| CSRF GET versus POST/PUT/PATCH/DELETE; wrong/missing token | N/A | yes auth store | auth_http | partial | yes | bearer-only does not require cookie CSRF |
| Password hashing/randomness/redacted Debug | unit/partial | unit/partial | PasswordField and facade auth tests | indirect | gap | dedicated log-capture/side-channel audit incomplete |
| HTTP methods, JSON/content type/error/header contracts | N/A | portable example | http_contract/runtime_security | yes | yes; live TCP GET/POST | generic view/action every-method matrix incomplete |
| Serializer mass assignment and write allowlists | unit | unit | REST policy + mass_assignment_http | tenant/ID protection | yes | application-specific sensitive fields require explicit allowlist |
| Authenticated/default protected/public endpoint semantics | N/A | yes | view policy + auth_http | tenant API and protected CRUD | yes | public explicitly selected |
| Idempotency replay/payload conflict/expiry/concurrent retry | N/A | N/A | http_state fixtures, mutation count | gap | yes | caller-provided durable atomic provider; not exactly-once DB effects |
| Throttle boundary/user isolation/window reset/backend failure | N/A | N/A | http_state deterministic fixtures | gap | yes | anonymous shared bucket; tenant/distributed provider/proxy matrix gap |
| CORS origins/preflight/method/header/credentials/Vary | N/A | N/A | http_boundaries | gap | yes | malformed/unsafe config rejected |
| URI/header-count/single/cumulative/body limit boundaries | N/A | N/A | http_boundaries exact and over-limit | quickstart | yes | parser/server upstream limits remain deployment responsibility |
| Security headers/secure transport | N/A | N/A | runtime_security/http_contract | quickstart | yes | actual proxy/TLS termination deployment not exercised |
| WebSocket policy/subprotocol/message/idle behavior | N/A | N/A | policy units only | gap | live upgrade/frame gap | experimental; production interoperability not certified |
| Migration graph/cycles/order/reversibility rendering | unit | unit | migration graph/operation/scanner units | partial | N/A | SQL rendering is not authoritative DB execution |
| SQL migrations apply/history/repeat/drift/rollback | yes | yes | SQLite drift test + consumer PG migration | yes generated SQLite/PG | N/A | all operations/failed concurrent migrations need expansion |
| File SQLite foreign keys/pool persistence | N/A | yes | file-backed transaction + all four held pool connections reject orphan inserts; disabled enforcement config rejected | yes | custom API | production custom pools must enable FK on every connection |
| Replica selection/fallback/config/redaction | unit | N/A | database configuration/selection units | gap | gap | no live asynchronous-replica consistency/failover certification |
| Memory/file media traversal/symlink/temp collision | N/A | N/A | storage_security actual filesystem | gap | upload endpoint gap | provider size/type/scanner application policy required |
| Scanner clean/malformed/provider error protocol | N/A | N/A | scanner_protocol loopback fixtures | gap | N/A | no live clamd/certified malware corpus; S3 runtime incomplete |
| Output encoding/Unicode/XML/inline-script breakout | N/A | N/A | secure-input unit + output_context | gap | N/A | contextual encoding, not universal sanitization |
| CLI create/app/refresh/config/check/generated compile | yes | yes | CLI units/scaffolding + verify_generated | yes independent generated workspace | live generated route | generated consumer includes protected CRUD on both backends |
| Public facade installation | yes | yes | verify_consumer copied path application | yes | yes | Git snapshot branch/rev/tag; hosted source unknown |
| Registry-style API/package assets | yes compiled | yes consumer | cargo package workspace + verify_packages | local registry patches | SQLite HTTP | real registry upload/ownership/download not established |
| Rust pinned/MSRV and stable | compile/test | compile/test | release logs | pinned consumer | pinned | MSRV = pin 1.97.1; stable check separate |
| Rustdoc/docs-site/dependency advisories | N/A | N/A | strict rustdoc, Vite build, cargo audit | examples executed | live quickstart | important public docs improved, every public item not fully documented |

## Unmet acceptance scope

The user's complete definition of done is not met merely by green executed gates. Remaining work includes actual hosted GitHub/tag verification and repository metadata, a safe SQLite legacy RBAC upgrade, all advertised generic-view/viewset workflows, live WebSocket acceptance, deterministic database contention/retry tests, complete relation/index/macro error matrices, PostgreSQL-backed HTTP authentication stories, replica failover behavior, and provider-specific production acceptance. CASE/annotation/window/subquery capabilities must not be advertised as typed QuerySet APIs when absent. Production S3, durable idempotency/throttle implementations, and malware policy are not supplied by the passing fixtures.

Useful future fuzz targets: cookie/cursor decoding, identifiers, filename boundaries, migration graphs, config/header parsing. Full fuzzing/property-based infrastructure has not been added. Test counts alone do not establish threat coverage.
