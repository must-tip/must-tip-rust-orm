//! Locking, aggregate and explicit global-authorization contracts.

use super::*;

/// Application authorization boundary for cross-tenant QuerySet access.
///
/// The ORM deliberately does not invent authentication or RBAC rules. Service code implements
/// this trait using its authoritative caller/permission context; QuerySet only proceeds when that
/// policy explicitly authorizes the model.
pub trait GlobalScopeAuthorization: Send + Sync {
    fn authorize_global_scope(&self, model: &'static str) -> Result<(), ModelError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowLock {
    Update,
    NoKeyUpdate,
    Share,
    KeyShare,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockWait {
    Wait,
    NoWait,
    SkipLocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateFunction {
    Count,
    Sum,
    Avg,
    Min,
    Max,
    StdDev,
    Variance,
    ArrayAgg,
    JsonAgg,
    BoolAnd,
    BoolOr,
}

impl AggregateFunction {
    pub(super) const fn as_sql(self) -> &'static str {
        match self {
            Self::Count => "COUNT",
            Self::Sum => "SUM",
            Self::Avg => "AVG",
            Self::Min => "MIN",
            Self::Max => "MAX",
            Self::StdDev => "STDDEV_SAMP",
            Self::Variance => "VAR_SAMP",
            Self::ArrayAgg => "ARRAY_AGG",
            Self::JsonAgg => "JSONB_AGG",
            Self::BoolAnd => "BOOL_AND",
            Self::BoolOr => "BOOL_OR",
        }
    }
}
