//! Typed values and validated conversion without database access.

use super::*;

#[derive(Debug, Clone, PartialEq)]
pub enum DbValue {
    Bool(bool),
    I16(i16),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    Decimal(Decimal),
    Uuid(Uuid),
    DateTime(DateTime<Utc>),
    String(String),
    Json(Value),
    Bytes(Vec<u8>),
}

impl DbValue {
    pub fn parse_for_column(spec: &ColumnSpec, raw: &str) -> Result<Self, ModelError> {
        match spec.sql_type.trim().to_ascii_uppercase().as_str() {
            "BOOLEAN" => raw
                .parse::<bool>()
                .map(Self::Bool)
                .map_err(|_| ModelError::InvalidQuery("expected boolean filter value".into())),
            "SMALLINT" => raw
                .parse::<i16>()
                .map(Self::I16)
                .map_err(|_| ModelError::InvalidQuery("expected SMALLINT filter value".into())),
            "INTEGER" => raw
                .parse::<i32>()
                .map(Self::I32)
                .map_err(|_| ModelError::InvalidQuery("expected INTEGER filter value".into())),
            "BIGINT" | "BIGSERIAL" => raw
                .parse::<i64>()
                .map(Self::I64)
                .map_err(|_| ModelError::InvalidQuery("expected BIGINT filter value".into())),
            "REAL" => raw
                .parse::<f32>()
                .map(Self::F32)
                .map_err(|_| ModelError::InvalidQuery("expected REAL filter value".into())),
            "DOUBLE PRECISION" => raw.parse::<f64>().map(Self::F64).map_err(|_| {
                ModelError::InvalidQuery("expected floating-point filter value".into())
            }),
            "NUMERIC" => raw
                .parse::<Decimal>()
                .map(Self::Decimal)
                .map_err(|_| ModelError::InvalidQuery("expected NUMERIC filter value".into())),
            "UUID" => raw
                .parse::<Uuid>()
                .map(Self::Uuid)
                .map_err(|_| ModelError::InvalidQuery("expected UUID filter value".into())),
            "TIMESTAMP" | "TIMESTAMPTZ" => DateTime::parse_from_rfc3339(raw)
                .map(|value| Self::DateTime(value.with_timezone(&Utc)))
                .map_err(|_| {
                    ModelError::InvalidQuery("expected RFC3339 datetime filter value".into())
                }),
            "DATE" => NaiveDate::parse_from_str(raw, "%Y-%m-%d")
                .map(|value| Self::String(value.format("%Y-%m-%d").to_string()))
                .map_err(|_| {
                    ModelError::InvalidQuery("expected ISO date filter value (YYYY-MM-DD)".into())
                }),
            "TIME" => parse_time_literal(raw).map(Self::String),
            "INET" => validate_network_literal(raw, false).map(Self::String),
            "CIDR" => validate_network_literal(raw, true).map(Self::String),
            "INTERVAL" => validate_interval_literal(raw).map(Self::String),
            "JSON" | "JSONB" => serde_json::from_str::<Value>(raw)
                .map(Self::Json)
                .map_err(|_| ModelError::InvalidQuery("expected JSON filter value".into())),
            "BYTEA" => STANDARD
                .decode(raw)
                .map(Self::Bytes)
                .map_err(|_| ModelError::InvalidQuery("expected base64 BYTEA filter value".into())),
            _ => Ok(Self::String(raw.to_owned())),
        }
    }

    pub(crate) fn from_json_for_column(
        spec: &ColumnSpec,
        value: &Value,
    ) -> Result<Self, ModelError> {
        if value.is_null() {
            return Err(ModelError::InvalidQuery(
                "NULL must use is_null()/is_not_null() rather than a bound value".into(),
            ));
        }
        let raw = match value {
            Value::String(value) => value.clone(),
            Value::Bool(value) => value.to_string(),
            Value::Number(value) => value.to_string(),
            other => serde_json::to_string(other)?,
        };
        Self::parse_for_column(spec, &raw)
    }
}

impl From<bool> for DbValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}
impl From<BooleanField> for DbValue {
    fn from(value: BooleanField) -> Self {
        Self::Bool(value.0)
    }
}
impl From<i16> for DbValue {
    fn from(value: i16) -> Self {
        Self::I16(value)
    }
}
impl From<i32> for DbValue {
    fn from(value: i32) -> Self {
        Self::I32(value)
    }
}
impl From<IntegerField> for DbValue {
    fn from(value: IntegerField) -> Self {
        Self::I32(value.0)
    }
}
impl From<i64> for DbValue {
    fn from(value: i64) -> Self {
        Self::I64(value)
    }
}
impl From<f32> for DbValue {
    fn from(value: f32) -> Self {
        Self::F32(value)
    }
}
impl From<f64> for DbValue {
    fn from(value: f64) -> Self {
        Self::F64(value)
    }
}
impl From<FloatField> for DbValue {
    fn from(value: FloatField) -> Self {
        Self::F64(value.0)
    }
}
impl From<String> for DbValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}
impl From<&str> for DbValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}
impl From<TextField> for DbValue {
    fn from(value: TextField) -> Self {
        Self::String(value.0)
    }
}
impl From<SlugField> for DbValue {
    fn from(value: SlugField) -> Self {
        Self::String(value.0)
    }
}
impl From<Uuid> for DbValue {
    fn from(value: Uuid) -> Self {
        Self::Uuid(value)
    }
}
impl From<UUIDField> for DbValue {
    fn from(value: UUIDField) -> Self {
        Self::Uuid(value.0)
    }
}
impl From<DateTime<Utc>> for DbValue {
    fn from(value: DateTime<Utc>) -> Self {
        Self::DateTime(value)
    }
}
impl From<DateTimeField> for DbValue {
    fn from(value: DateTimeField) -> Self {
        Self::DateTime(value.0)
    }
}
impl From<Decimal> for DbValue {
    fn from(value: Decimal) -> Self {
        Self::Decimal(value)
    }
}
impl From<DecimalField> for DbValue {
    fn from(value: DecimalField) -> Self {
        Self::Decimal(value.0)
    }
}
impl From<Value> for DbValue {
    fn from(value: Value) -> Self {
        Self::Json(value)
    }
}
impl From<Vec<u8>> for DbValue {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value)
    }
}
pub(super) fn parse_time_literal(raw: &str) -> Result<String, ModelError> {
    let parsed = NaiveTime::parse_from_str(raw, "%H:%M:%S%.f")
        .or_else(|_| NaiveTime::parse_from_str(raw, "%H:%M"))
        .map_err(|_| {
            ModelError::InvalidQuery(
                "expected ISO time filter value (HH:MM or HH:MM:SS[.fraction])".into(),
            )
        })?;
    Ok(parsed.format("%H:%M:%S%.f").to_string())
}

pub(super) fn validate_network_literal(
    raw: &str,
    require_prefix: bool,
) -> Result<String, ModelError> {
    if raw.is_empty() || raw.len() > 64 || raw.chars().any(char::is_control) {
        return Err(ModelError::InvalidQuery(
            "invalid network filter value".into(),
        ));
    }
    let (address, prefix) = match raw.split_once('/') {
        Some((address, prefix))
            if !address.is_empty() && !prefix.is_empty() && !prefix.contains('/') =>
        {
            (address, Some(prefix))
        }
        Some(_) => return Err(ModelError::InvalidQuery("invalid network prefix".into())),
        None if require_prefix => {
            return Err(ModelError::InvalidQuery(
                "CIDR filters require an explicit prefix length".into(),
            ));
        }
        None => (raw, None),
    };
    let address = address
        .parse::<IpAddr>()
        .map_err(|_| ModelError::InvalidQuery("invalid IP address filter value".into()))?;
    if let Some(prefix) = prefix {
        let prefix = prefix
            .parse::<u8>()
            .map_err(|_| ModelError::InvalidQuery("invalid network prefix length".into()))?;
        let max = if address.is_ipv4() { 32 } else { 128 };
        if prefix > max {
            return Err(ModelError::InvalidQuery(
                "network prefix exceeds address width".into(),
            ));
        }
    }
    Ok(raw.to_owned())
}

pub(super) fn validate_interval_literal(raw: &str) -> Result<String, ModelError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 256 || trimmed.chars().any(char::is_control) {
        return Err(ModelError::InvalidQuery(
            "invalid INTERVAL filter value".into(),
        ));
    }
    Ok(trimmed.to_owned())
}
