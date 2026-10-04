use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::rest_framework::{exceptions::ApiResult, request::RequestContext};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    Success,
    ClientError,
    Denied,
    Throttled,
    ServerError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub request_id: Uuid,
    pub trace_id: Option<String>,
    pub principal_subject: String,
    pub tenant_id: Option<Uuid>,
    pub route_name: String,
    pub method: String,
    pub resource_type: Option<String>,
    pub resource_id: Option<String>,
    pub status: u16,
    pub duration_micros: u64,
    pub outcome: AuditOutcome,
}

impl AuditEvent {
    pub fn from_request(
        context: &RequestContext,
        route_name: impl Into<String>,
        method: impl Into<String>,
        status: u16,
        outcome: AuditOutcome,
    ) -> Self {
        let micros = duration_micros(context.elapsed());
        Self {
            event_id: Uuid::now_v7(),
            occurred_at: Utc::now(),
            request_id: context.request_id(),
            trace_id: context.trace_id().map(str::to_owned),
            principal_subject: context.principal().subject().to_owned(),
            tenant_id: context.tenant_id(),
            route_name: route_name.into(),
            method: method.into(),
            resource_type: None,
            resource_id: None,
            status,
            duration_micros: micros,
            outcome,
        }
    }

    pub fn before_context(
        request_id: Uuid,
        trace_id: Option<String>,
        route_name: impl Into<String>,
        method: impl Into<String>,
        status: u16,
        duration: Duration,
        outcome: AuditOutcome,
    ) -> Self {
        Self {
            event_id: Uuid::now_v7(),
            occurred_at: Utc::now(),
            request_id,
            trace_id,
            principal_subject: "unauthenticated".to_owned(),
            tenant_id: None,
            route_name: route_name.into(),
            method: method.into(),
            resource_type: None,
            resource_id: None,
            status,
            duration_micros: duration_micros(duration),
            outcome,
        }
    }
}

fn duration_micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

#[async_trait]
pub trait AuditSink: Send + Sync {
    async fn record(&self, event: AuditEvent) -> ApiResult<()>;
}

#[derive(Debug, Default)]
pub struct TracingAuditSink;

#[async_trait]
impl AuditSink for TracingAuditSink {
    async fn record(&self, event: AuditEvent) -> ApiResult<()> {
        tracing::info!(
            event_id = %event.event_id,
            request_id = %event.request_id,
            trace_id = event.trace_id.as_deref(),
            principal = %event.principal_subject,
            tenant_id = ?event.tenant_id,
            route = %event.route_name,
            method = %event.method,
            status = event.status,
            duration_micros = event.duration_micros,
            outcome = ?event.outcome,
            "api_audit"
        );
        Ok(())
    }
}
