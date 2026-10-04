#![cfg(feature = "axum")]
//! Process-local fixtures exercise the public store contracts, not distributed durability.
use async_trait::async_trait;
use axum::{Router, body::Body};
use http::{Method, Request, StatusCode};
use lex_rest::rest_framework::{
    adapters::axum::AxumAdapter,
    authentication::{AuthenticationBackend, AuthenticationChain, AuthenticationOutcome},
    exceptions::{ApiError, ApiResult},
    idempotency::{IdempotencyPolicy, IdempotencyReservation, IdempotencyStore},
    middleware::{RestApplication, RestRuntimeBuilder},
    permissions::DenyAllPermissionBackend,
    request::{ApiRequest, Principal, RequestContext},
    response::ApiResponse,
    routing::{RouteSpec, RouteTable},
    throttling::{
        DistributedThrottleStore, ThrottleEngine, ThrottleKey, ThrottleLease, ThrottlePolicy,
    },
    views::{APIView, ViewAccess, ViewServices},
};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Notify};
use tower::ServiceExt;

struct Entry {
    fingerprint: String,
    token: String,
    response: Option<ApiResponse>,
    expires: Instant,
}
#[derive(Default)]
struct LocalIdempotency(Mutex<HashMap<(String, String), Entry>>);
#[async_trait]
impl IdempotencyStore for LocalIdempotency {
    async fn reserve(
        &self,
        namespace: &str,
        key: &str,
        fingerprint: &str,
        ttl: Duration,
    ) -> ApiResult<IdempotencyReservation> {
        let mut entries = self.0.lock().await;
        let key = (namespace.to_owned(), key.to_owned());
        if entries
            .get(&key)
            .is_some_and(|entry| entry.expires <= Instant::now())
        {
            entries.remove(&key);
        }
        if let Some(entry) = entries.get(&key) {
            return Ok(if entry.fingerprint != fingerprint {
                IdempotencyReservation::Conflict
            } else if let Some(response) = &entry.response {
                IdempotencyReservation::Replay(response.clone())
            } else {
                IdempotencyReservation::InProgress {
                    retry_after: Duration::from_secs(1),
                }
            });
        }
        let token = uuid::Uuid::new_v4().to_string();
        entries.insert(
            key,
            Entry {
                fingerprint: fingerprint.into(),
                token: token.clone(),
                response: None,
                expires: Instant::now() + ttl,
            },
        );
        Ok(IdempotencyReservation::Started { token })
    }
    async fn complete(
        &self,
        namespace: &str,
        key: &str,
        token: &str,
        response: &ApiResponse,
        ttl: Duration,
    ) -> ApiResult<()> {
        let mut entries = self.0.lock().await;
        let entry = entries
            .get_mut(&(namespace.into(), key.into()))
            .ok_or_else(|| ApiError::internal("missing test reservation"))?;
        if entry.token != token {
            return Err(ApiError::internal("stale test reservation"));
        }
        entry.response = Some(response.clone());
        entry.expires = Instant::now() + ttl;
        Ok(())
    }
    async fn abandon(&self, namespace: &str, key: &str, token: &str) -> ApiResult<()> {
        let mut entries = self.0.lock().await;
        let key = (namespace.into(), key.into());
        if entries.get(&key).is_some_and(|entry| entry.token == token) {
            entries.remove(&key);
        }
        Ok(())
    }
}
#[derive(Default)]
struct LocalThrottle {
    entries: Mutex<HashMap<String, (u64, u64)>>,
    offset_ms: AtomicU64,
}
#[async_trait]
impl DistributedThrottleStore for LocalThrottle {
    async fn acquire(
        &self,
        key: &str,
        limit: u64,
        window: Duration,
        now: u64,
    ) -> ApiResult<ThrottleLease> {
        let milliseconds = u64::try_from(window.as_millis()).unwrap().max(1);
        let now = now + self.offset_ms.load(Ordering::SeqCst);
        let mut entries = self.entries.lock().await;
        let (count, expires) = entries.entry(key.into()).or_insert((0, now + milliseconds));
        if now >= *expires {
            *count = 0;
            *expires = now + milliseconds;
        }
        *count += 1;
        Ok(ThrottleLease {
            allowed: *count <= limit,
            remaining: limit.saturating_sub(*count),
            reset_after: Duration::from_millis(*expires - now),
        })
    }
}
struct TestIdentity;
#[async_trait]
impl AuthenticationBackend for TestIdentity {
    fn name(&self) -> &'static str {
        "test-only"
    }
    async fn authenticate(&self, request: &ApiRequest) -> ApiResult<AuthenticationOutcome> {
        match request.headers().get("x-test-user") {
            None => Ok(AuthenticationOutcome::Abstain),
            Some(value) => Ok(AuthenticationOutcome::Authenticated(Principal::user(
                value
                    .to_str()
                    .ok()
                    .and_then(|value| value.parse().ok())
                    .ok_or_else(|| ApiError::authentication("invalid test identity"))?,
                "test-only",
                [],
            )?)),
        }
    }
}
struct Mutation {
    count: Arc<AtomicUsize>,
    required: bool,
    gate: Option<(Arc<Notify>, Arc<Notify>)>,
}
#[async_trait]
impl APIView for Mutation {
    fn name(&self) -> &'static str {
        "mutation"
    }
    fn access(&self) -> ViewAccess {
        ViewAccess::Public
    }
    fn allowed_methods(&self) -> Vec<Method> {
        vec![Method::POST]
    }
    fn idempotency_policy(&self, _: &Method) -> IdempotencyPolicy {
        if self.required {
            IdempotencyPolicy::required(Duration::from_secs(60)).unwrap()
        } else {
            IdempotencyPolicy::default()
        }
    }
    async fn post(
        &self,
        request: &ApiRequest,
        _: &RequestContext,
        _: &ViewServices,
    ) -> ApiResult<ApiResponse> {
        let payload = request.json()?;
        let id = self.count.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some((entered, release)) = &self.gate {
            entered.notify_one();
            release.notified().await;
        }
        ApiResponse::json(
            StatusCode::CREATED,
            &serde_json::json!({"id":id,"payload":payload}),
        )
    }
}
fn router(
    view: Mutation,
    idempotency: Option<Arc<LocalIdempotency>>,
    throttle: Option<Arc<LocalThrottle>>,
) -> Router {
    let mut routes = RouteTable::new();
    routes
        .register(RouteSpec::http("/mutations", "mutations", Arc::new(view)))
        .unwrap();
    let mut builder = RestRuntimeBuilder::new(Arc::new(DenyAllPermissionBackend))
        .require_secure_transport(false)
        .authentication(AuthenticationChain::new(vec![Arc::new(TestIdentity)]).unwrap());
    if let Some(store) = idempotency {
        builder = builder.idempotency_store(store);
    }
    if let Some(store) = throttle {
        builder = builder.throttling(
            ThrottleEngine::new(store),
            vec![ThrottlePolicy {
                scope: "mutations".into(),
                limit: 2,
                window: Duration::from_secs(60),
                key: ThrottleKey::Principal,
                fail_closed: true,
            }],
        );
    }
    AxumAdapter::new(Arc::new(RestApplication::new(
        routes,
        builder.build().unwrap(),
    )))
    .router()
    .unwrap()
}
async fn send(
    router: &Router,
    key: Option<&str>,
    body: &str,
    user: Option<uuid::Uuid>,
) -> axum::response::Response {
    let mut request = Request::builder()
        .method("POST")
        .uri("/mutations")
        .header("content-type", "application/json");
    if let Some(key) = key {
        request = request.header("idempotency-key", key);
    }
    if let Some(user) = user {
        request = request.header("x-test-user", user.to_string());
    }
    router
        .clone()
        .oneshot(request.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap()
}
#[tokio::test]
async fn retries_replay_the_original_mutation_and_changed_payloads_conflict() {
    let count = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(LocalIdempotency::default());
    let router = router(
        Mutation {
            count: count.clone(),
            required: true,
            gate: None,
        },
        Some(store.clone()),
        None,
    );
    assert_eq!(
        send(&router, None, "{}", None).await.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        send(&router, Some("request-123"), "{}", None)
            .await
            .status(),
        StatusCode::CREATED
    );
    let retry = send(&router, Some("request-123"), "{}", None).await;
    assert_eq!(retry.status(), StatusCode::CREATED);
    assert_eq!(retry.headers()["idempotency-replayed"], "true");
    assert_eq!(
        send(&router, Some("request-123"), "{\"changed\":true}", None)
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    // Simulate store expiry without a wall-clock sleep; the framework must request a new lease.
    for entry in store.0.lock().await.values_mut() {
        entry.expires = Instant::now();
    }
    assert_eq!(
        send(&router, Some("request-123"), "{}", None)
            .await
            .status(),
        StatusCode::CREATED
    );
    assert_eq!(count.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn simultaneous_duplicate_requests_execute_the_mutation_once() {
    let count = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let router = router(
        Mutation {
            count: count.clone(),
            required: true,
            gate: Some((entered.clone(), release.clone())),
        },
        Some(Arc::new(LocalIdempotency::default())),
        None,
    );
    let first_router = router.clone();
    let first =
        tokio::spawn(async move { send(&first_router, Some("request-123"), "{}", None).await });
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    let duplicate = send(&router, Some("request-123"), "{}", None).await;
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);
    assert!(duplicate.headers().contains_key("retry-after"));
    release.notify_one();
    assert_eq!(first.await.unwrap().status(), StatusCode::CREATED);
    assert_eq!(
        send(&router, Some("request-123"), "{}", None)
            .await
            .status(),
        StatusCode::CREATED
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn throttling_keeps_users_separate_from_the_shared_anonymous_bucket_and_resets_expired_windows()
 {
    let count = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(LocalThrottle::default());
    let router = router(
        Mutation {
            count: count.clone(),
            required: false,
            gate: None,
        },
        None,
        Some(store.clone()),
    );
    let alice = uuid::Uuid::new_v4();
    let bob = uuid::Uuid::new_v4();
    for user in [Some(alice), Some(bob), None] {
        for _ in 0..2 {
            assert_eq!(
                send(&router, None, "{}", user).await.status(),
                StatusCode::CREATED
            );
        }
        let denied = send(&router, None, "{}", user).await;
        assert_eq!(denied.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(denied.headers().contains_key("retry-after"));
    }
    assert_eq!(count.load(Ordering::SeqCst), 6);
    store.offset_ms.store(60_001, Ordering::SeqCst);
    assert_eq!(
        send(&router, None, "{}", Some(alice)).await.status(),
        StatusCode::CREATED
    );
}

#[tokio::test]
async fn an_idempotency_key_cannot_replay_another_users_mutation() {
    let count = Arc::new(AtomicUsize::new(0));
    let router = router(
        Mutation {
            count: count.clone(),
            required: true,
            gate: None,
        },
        Some(Arc::new(LocalIdempotency::default())),
        None,
    );
    let alice = uuid::Uuid::new_v4();
    let bob = uuid::Uuid::new_v4();
    assert_eq!(
        send(&router, Some("request-123"), "{}", Some(alice))
            .await
            .status(),
        StatusCode::CREATED
    );
    let denied = send(&router, Some("request-123"), "{}", Some(bob)).await;
    assert_eq!(denied.status(), StatusCode::CONFLICT);
    assert!(!denied.headers().contains_key("idempotency-replayed"));
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn missing_idempotency_store_fails_before_a_required_mutation_runs() {
    let count = Arc::new(AtomicUsize::new(0));
    let router = router(
        Mutation {
            count: count.clone(),
            required: true,
            gate: None,
        },
        None,
        None,
    );
    assert_eq!(
        send(&router, Some("request-123"), "{}", None)
            .await
            .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(count.load(Ordering::SeqCst), 0);
}
