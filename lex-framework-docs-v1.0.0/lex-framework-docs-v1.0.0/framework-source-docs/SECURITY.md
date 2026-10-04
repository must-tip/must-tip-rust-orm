# Security Controls

## Database transport and secrets

- `DATABASE_URL` is loaded from environment/CLI only and is not serialized into `Config.toml`.
- Production profile does not auto-load `.env`.
- PostgreSQL TLS defaults to certificate and hostname verification (`VerifyFull`).
- Connection logs do not print credentials.
- Cursor signing secrets are caller supplied and redacted from debug output.

Private/self-signed PKI deployments must install the correct trust anchor and use a hostname present in the server certificate. Do not weaken verification as an operational shortcut.

## SQL construction

- Runtime values use SQLx bind parameters.
- QuerySet validates `DbValue` kinds against model SQL metadata before execution.
- Dynamic table/column identifiers are restricted to safe ASCII identifier characters, bounded to PostgreSQL identifier length, and quoted.
- Generated SQL types use a supported allowlist.
- Generated default expressions reject statement separators, comments, NUL, and newlines.
- Dynamic REST filters are field/lookup allowlisted and type parsed.
- `IN` predicates and request inputs are cardinality/size bounded.
- Destructive QuerySet mutations require a user scope or explicit all-row mutation opt-in.
- Generic updates cannot change tenant ownership, primary keys, soft-delete markers, timestamps, or optimistic-version fields directly; versioned bulk updates advance the version automatically.
- Model CHECK constraints use a conservative expression validator; complex/procedural SQL belongs in explicit reviewed migrations.
- Arbitrary raw hooks are not accepted by QuerySet.

## Tenant isolation

Tenant-aware entities fail closed when no tenant is supplied. The compatibility repository also refuses tenant-aware models because its API cannot carry tenant authorization.

Cross-tenant QuerySet access requires `GlobalScopeAuthorization`; the application authorization layer must implement the decision. The ORM does not infer administrator status.

For sensitive multi-tenant tables, enable PostgreSQL RLS through reviewed migrations. `TenantRlsPolicy` and transaction-local `app.tenant_id` support an independent database enforcement layer. Test RLS using the same non-superuser database role used by production application traffic.

## Concurrency and transaction controls

- Row locks require an active transaction.
- Lock wait mode can be wait/no-wait/skip-locked.
- Transaction-local statement, lock, and idle timeouts are configurable.
- Retry policy is bounded and limited to serialization/deadlock SQLSTATEs.
- Versioned writes provide optimistic concurrency protection.
- `Tracked<T>::save()` uses one transaction for update and refresh.
- Post-commit callbacks execute only after successful commit.

## REST/input boundary

Serializer policy rejects unknown/read-only/null-invalid/type-invalid inputs. Encrypted model fields are write-only by default and are not filter/search/order eligible through REST policy.

Cursor tokens use HMAC-SHA256, a versioned payload, bounded token length, constant-time MAC verification supplied by the HMAC crate, and a bounded previous-key set for rotation. Use independent high-entropy keys from the service secret manager; do not reuse database passwords, JWT signing keys, or application session secrets.

Application HTTP layers must still enforce authentication, resource authorization, CSRF policy where cookies are used, request-body size limits, rate limits, CORS policy, and abuse controls. Those are intentionally not invented inside a database crate.

## Passwords and sensitive values

`PasswordField` stores PHC-formatted Argon2id hashes and redacts `Debug`. Password hashes, API-key hashes, and verification tokens in the supplied models are excluded from ordinary serialization. Treat `encrypted` model metadata as sensitivity metadata; it is not transparent database encryption.

## Input/output security

- filename validation rejects traversal, nested paths, encoded traversal, control characters, and dangerous extensions;
- URL validation accepts HTTP/HTTPS URLs with a host and rejects active/local schemes;
- HTML sanitization uses an allowlist parser;
- JavaScript string/property encoding handles separator/control hazards;
- XML/JSON/CSS/URL encoders are context-specific.

No one sanitizer makes a value safe for every sink. Validate on input and encode for the final output context.

## Unsafe Rust

Workspace lint policy forbids unsafe code. The prior raw-`Arc` reconstruction was removed and the final source audit must continue to reject explicit unsafe blocks.

## Supply chain

Direct dependencies are declared once in the root workspace. Production releases require a reviewed `Cargo.lock`, locked builds, `cargo tree --duplicates` review, and the organization's vulnerability/license/source-policy tooling. No dependency-vulnerability or license-compliance claim is made from this environment because external audit tooling was unavailable.

Related models require their own global-scope authorization before a privileged parent query may bypass tenant scoping for that relation. A parent authorization decision is never inherited implicitly by joined or prefetched models.

## Sensitive request transforms

Serializer validation is not a password/key encryption primitive. Write-only/encrypted request fields are deliberately rejected by generic model deserialization until the owning service applies its authoritative hashing/encryption/KMS transform. Persist only derived password hashes, API-key hashes, ciphertext, or other approved protected representations.

## REST/WSS security boundary

`lex_rest` requires secure transport by default, fails closed on invalid presented credentials, enforces tenant/object permission boundaries, protects framework-owned serializer fields, bounds request/query/header/WebSocket inputs, hashes idempotency/throttle partition keys, applies security response headers, validates WebSocket origins/subprotocols, and audits without request bodies or credential values. Concrete authentication and distributed storage implementations remain service-owned security contracts. See `docs/LEX_REST.md`.


## lex_rest authorization and mutation hardening

- Database-backed API-key principals are limited by both the owning user's effective database permission and the API-key scope; either denial denies the request.
- Empty permission conjunctions/collections fail closed instead of becoming vacuous authorization.
- Object permissions independently reject cross-tenant resources unless the configured global-admin permission backend explicitly authorizes the override.
- Peer-IP throttles require trusted peer metadata from the transport boundary; the framework does not trust arbitrary forwarding headers or invent an `unknown` client bucket.
- The default ORM REST update adapter runs update, reload, optimistic-version check, tenant/RLS context, and full model validation in one `AtomicTransaction`; failed model validation rolls back.
- WebSocket/WSS routes authenticate and authorize before upgrade, require origin policy by default, accept HTTP/1.1 GET and HTTP/2+ CONNECT upgrade forms, and configure transport-level frame/message bounds in the optional Axum adapter.
