use async_trait::async_trait;
use http::{HeaderMap, Method, Uri};
use lex_rest::rest_framework::{
    authentication::{AuthenticationBackend, AuthenticationChain, AuthenticationOutcome},
    exceptions::ApiResult,
    request::{ApiRequest, Principal, TransportSecurity},
};
use std::sync::Arc;
use uuid::Uuid;

struct CredentialBackend {
    name: &'static str,
    outcome: AuthenticationOutcome,
}

#[async_trait]
impl AuthenticationBackend for CredentialBackend {
    fn name(&self) -> &'static str {
        self.name
    }
    async fn authenticate(&self, _: &ApiRequest) -> ApiResult<AuthenticationOutcome> {
        Ok(self.outcome.clone())
    }
}

fn request() -> ApiRequest {
    ApiRequest::new(
        Method::GET,
        Uri::from_static("/private"),
        HeaderMap::new(),
        vec![],
        None,
        TransportSecurity::Secure,
        1024,
    )
    .unwrap()
}

fn chain(outcomes: Vec<AuthenticationOutcome>) -> AuthenticationChain {
    AuthenticationChain::new(
        outcomes
            .into_iter()
            .enumerate()
            .map(|(i, outcome)| {
                Arc::new(CredentialBackend {
                    name: if i == 0 { "session" } else { "bearer" },
                    outcome,
                }) as Arc<dyn AuthenticationBackend>
            })
            .collect(),
    )
    .unwrap()
}

fn valid() -> AuthenticationOutcome {
    AuthenticationOutcome::Authenticated(
        Principal::user(Uuid::from_u128(1), "test", vec![]).unwrap(),
    )
}

#[tokio::test]
async fn valid_session_does_not_hide_invalid_bearer_credentials() {
    let auth = chain(vec![
        valid(),
        AuthenticationOutcome::Rejected {
            safe_detail: "invalid bearer",
        },
    ]);
    assert!(auth.authenticate(&request()).await.is_err());
}

#[tokio::test]
async fn two_valid_credentials_are_rejected_as_ambiguous() {
    assert!(
        chain(vec![valid(), valid()])
            .authenticate(&request())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn absent_session_allows_a_valid_bearer_credential() {
    assert!(
        chain(vec![AuthenticationOutcome::Abstain, valid()])
            .authenticate(&request())
            .await
            .unwrap()
            .is_authenticated()
    );
}
