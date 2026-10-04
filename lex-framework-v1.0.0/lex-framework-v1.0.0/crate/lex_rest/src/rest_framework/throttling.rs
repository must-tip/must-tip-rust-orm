use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::rest_framework::{
    exceptions::{ApiError, ApiResult},
    request::RequestContext,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThrottleKey {
    Principal,
    Tenant,
    PrincipalAndTenant,
    PeerIp,
    ScopeOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThrottlePolicy {
    pub scope: String,
    pub limit: u64,
    pub window: Duration,
    pub key: ThrottleKey,
    pub fail_closed: bool,
}

impl ThrottlePolicy {
    pub fn validate(&self) -> ApiResult<()> {
        if self.scope.is_empty()
            || self.scope.len() > 128
            || !self
                .scope
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':'))
        {
            return Err(ApiError::configuration("invalid throttle scope"));
        }
        if self.limit == 0 || self.limit > 10_000_000 {
            return Err(ApiError::configuration("invalid throttle limit"));
        }
        if self.window.is_zero() || self.window > Duration::from_secs(86_400) {
            return Err(ApiError::configuration("invalid throttle window"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThrottleLease {
    pub allowed: bool,
    pub remaining: u64,
    pub reset_after: Duration,
}

/// Distributed implementations MUST atomically increment/check a key and set/retain its expiry
/// in one server-side operation. A read-then-write implementation is not compliant because it
/// permits bursts under concurrency.
#[async_trait]
pub trait DistributedThrottleStore: Send + Sync {
    async fn acquire(
        &self,
        key_hash: &str,
        limit: u64,
        window: Duration,
        now_unix_ms: u64,
    ) -> ApiResult<ThrottleLease>;
}

#[derive(Clone)]
pub struct ThrottleEngine {
    store: Arc<dyn DistributedThrottleStore>,
}

impl ThrottleEngine {
    pub fn new(store: Arc<dyn DistributedThrottleStore>) -> Self {
        Self { store }
    }

    pub async fn check(
        &self,
        policy: &ThrottlePolicy,
        context: &RequestContext,
    ) -> ApiResult<ThrottleLease> {
        policy.validate()?;
        let raw_key = match policy.key {
            ThrottleKey::Principal => context.principal().subject().to_owned(),
            ThrottleKey::Tenant => context
                .tenant_id()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "no-tenant".to_owned()),
            ThrottleKey::PrincipalAndTenant => format!(
                "{}:{}",
                context.principal().subject(),
                context
                    .tenant_id()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "no-tenant".to_owned())
            ),
            ThrottleKey::PeerIp => context
                .peer_ip()
                .map(|value| value.to_string())
                .ok_or_else(|| {
                    ApiError::configuration(
                        "peer-IP throttle requires a trusted peer IP from the transport adapter",
                    )
                })?,
            ThrottleKey::ScopeOnly => "global".to_owned(),
        };
        let key_hash = throttle_key_hash(&policy.scope, &raw_key);
        let now_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ApiError::internal("system clock is before UNIX epoch"))?
            .as_millis()
            .try_into()
            .map_err(|_| ApiError::internal("system clock overflow"))?;
        match self
            .store
            .acquire(&key_hash, policy.limit, policy.window, now_unix_ms)
            .await
        {
            Ok(lease) => {
                if lease.allowed {
                    Ok(lease)
                } else {
                    Err(ApiError::throttled(lease.reset_after.as_secs().max(1)))
                }
            }
            Err(error) if policy.fail_closed => Err(error),
            Err(error) => {
                tracing::error!(error_code = error.code(), scope = %policy.scope, "throttle store unavailable; fail-open policy applied");
                Ok(ThrottleLease {
                    allowed: true,
                    remaining: 0,
                    reset_after: policy.window,
                })
            }
        }
    }
}

fn throttle_key_hash(scope: &str, raw_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(scope.as_bytes());
    hasher.update([0]);
    hasher.update(raw_key.as_bytes());
    let digest = hasher.finalize();
    let mut output = String::with_capacity("mtrest:v1:".len() + digest.len() * 2);
    output.push_str("mtrest:v1:");
    use std::fmt::Write as _;
    for byte in digest {
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

#[cfg(test)]
pub(crate) mod test_store {
    use super::*;
    use std::{collections::HashMap, sync::Mutex};

    #[derive(Default)]
    pub struct MemoryThrottleStore {
        values: Mutex<HashMap<String, (u64, u64)>>,
    }

    #[async_trait]
    impl DistributedThrottleStore for MemoryThrottleStore {
        async fn acquire(
            &self,
            key_hash: &str,
            limit: u64,
            window: Duration,
            now_unix_ms: u64,
        ) -> ApiResult<ThrottleLease> {
            let window_ms = u64::try_from(window.as_millis())
                .map_err(|_| ApiError::internal("window overflow"))?;
            let mut values = self.values.lock().expect("throttle store");
            let entry = values
                .entry(key_hash.to_owned())
                .or_insert((0, now_unix_ms.saturating_add(window_ms)));
            if now_unix_ms >= entry.1 {
                *entry = (0, now_unix_ms.saturating_add(window_ms));
            }
            entry.0 = entry.0.saturating_add(1);
            let allowed = entry.0 <= limit;
            Ok(ThrottleLease {
                allowed,
                remaining: limit.saturating_sub(entry.0),
                reset_after: Duration::from_millis(entry.1.saturating_sub(now_unix_ms)),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{test_store::MemoryThrottleStore, *};
    use crate::rest_framework::request::{Principal, RequestContext, TransportSecurity};

    #[tokio::test]
    async fn peer_ip_throttle_fails_closed_without_trusted_peer_ip() {
        let store = Arc::new(MemoryThrottleStore::default());
        let engine = ThrottleEngine::new(store);
        let principal =
            Principal::user(uuid::Uuid::new_v4(), "test", Vec::new()).expect("principal");
        let context =
            RequestContext::new(principal, None, None, TransportSecurity::Secure, None, None)
                .expect("context");
        let policy = ThrottlePolicy {
            scope: "peer.read".to_owned(),
            limit: 10,
            window: Duration::from_secs(60),
            key: ThrottleKey::PeerIp,
            fail_closed: false,
        };

        let error = engine
            .check(&policy, &context)
            .await
            .expect_err("missing trusted peer IP must not become a global shared bucket");
        assert_eq!(error.code(), "configuration_error");
    }

    #[tokio::test]
    async fn atomic_store_contract_enforces_limit() {
        let store = Arc::new(MemoryThrottleStore::default());
        let engine = ThrottleEngine::new(store);
        let principal =
            Principal::user(uuid::Uuid::new_v4(), "test", Vec::new()).expect("principal");
        let context =
            RequestContext::new(principal, None, None, TransportSecurity::Secure, None, None)
                .expect("context");
        let policy = ThrottlePolicy {
            scope: "user.read".to_owned(),
            limit: 2,
            window: Duration::from_secs(60),
            key: ThrottleKey::Principal,
            fail_closed: true,
        };
        engine.check(&policy, &context).await.expect("first");
        engine.check(&policy, &context).await.expect("second");
        assert!(engine.check(&policy, &context).await.is_err());
    }
}
