use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Lookup {
    Exact,
    NotEqual,
    IExact,
    Contains,
    IContains,
    StartsWith,
    IStartsWith,
    EndsWith,
    IEndsWith,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
    In,
    Range,
    IsNull,
    Year,
    Quarter,
    Month,
    Week,
    Day,
    Hour,
    Minute,
    Second,
    JsonContains,
    HasKey,
}

impl Lookup {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::NotEqual => "neq",
            Self::IExact => "iexact",
            Self::Contains => "contains",
            Self::IContains => "icontains",
            Self::StartsWith => "startswith",
            Self::IStartsWith => "istartswith",
            Self::EndsWith => "endswith",
            Self::IEndsWith => "iendswith",
            Self::GreaterThan => "gt",
            Self::GreaterThanOrEqual => "gte",
            Self::LessThan => "lt",
            Self::LessThanOrEqual => "lte",
            Self::In => "in",
            Self::Range => "range",
            Self::IsNull => "isnull",
            Self::Year => "year",
            Self::Quarter => "quarter",
            Self::Month => "month",
            Self::Week => "week",
            Self::Day => "day",
            Self::Hour => "hour",
            Self::Minute => "minute",
            Self::Second => "second",
            Self::JsonContains => "jsoncontains",
            Self::HasKey => "haskey",
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        [
            Self::Exact,
            Self::NotEqual,
            Self::IExact,
            Self::Contains,
            Self::IContains,
            Self::StartsWith,
            Self::IStartsWith,
            Self::EndsWith,
            Self::IEndsWith,
            Self::GreaterThan,
            Self::GreaterThanOrEqual,
            Self::LessThan,
            Self::LessThanOrEqual,
            Self::In,
            Self::Range,
            Self::IsNull,
            Self::Year,
            Self::Quarter,
            Self::Month,
            Self::Week,
            Self::Day,
            Self::Hour,
            Self::Minute,
            Self::Second,
            Self::JsonContains,
            Self::HasKey,
        ]
        .into_iter()
        .find(|lookup| lookup.name() == value)
    }
}

pub(super) fn scalar_cursor_value(value: &Value) -> Result<String, RestError> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Number(value) => Ok(value.to_string()),
        Value::Bool(value) => Ok(value.to_string()),
        _ => Err(RestError::new(
            "cursor_error",
            "cursor fields must serialize as scalar values",
        )),
    }
}

pub(super) fn reject_params(
    params: &HashMap<String, String>,
    names: &[&str],
) -> Result<(), RestError> {
    if let Some(name) = names.iter().find(|name| params.contains_key(**name)) {
        return Err(RestError::new(
            "invalid_pagination",
            format!("parameter {name} is incompatible with this pagination style"),
        ));
    }
    Ok(())
}

pub(super) fn is_text_sql_type(sql_type: &str) -> bool {
    matches!(
        sql_type.trim().to_ascii_uppercase().as_str(),
        "TEXT" | "CHARACTER VARYING"
    )
}

pub(super) fn validate_lookup_for_column(
    field: &str,
    sql_type: &str,
    lookup: Lookup,
) -> Result<(), RestError> {
    let normalized = sql_type.trim().to_ascii_uppercase();
    let requires_text = matches!(
        lookup,
        Lookup::IExact
            | Lookup::Contains
            | Lookup::IContains
            | Lookup::StartsWith
            | Lookup::IStartsWith
            | Lookup::EndsWith
            | Lookup::IEndsWith
    );
    let requires_temporal = matches!(
        lookup,
        Lookup::Year
            | Lookup::Quarter
            | Lookup::Month
            | Lookup::Week
            | Lookup::Day
            | Lookup::Hour
            | Lookup::Minute
            | Lookup::Second
    );
    let requires_jsonb = matches!(lookup, Lookup::JsonContains | Lookup::HasKey);

    if requires_text && !is_text_sql_type(&normalized) {
        return Err(RestError::new(
            "invalid_policy",
            format!("lookup {} requires a text field: {field}", lookup.name()),
        ));
    }
    if requires_temporal && !matches!(normalized.as_str(), "DATE" | "TIMESTAMP" | "TIMESTAMPTZ") {
        return Err(RestError::new(
            "invalid_policy",
            format!(
                "lookup {} requires a date/timestamp field: {field}",
                lookup.name()
            ),
        ));
    }
    if requires_jsonb && normalized != "JSONB" {
        return Err(RestError::new(
            "invalid_policy",
            format!("lookup {} requires a JSONB field: {field}", lookup.name()),
        ));
    }
    Ok(())
}

pub(super) fn parse_bounded_u32(
    value: Option<&String>,
    default: u32,
    maximum: u32,
    name: &str,
) -> Result<u32, RestError> {
    let value = match value {
        Some(value) => value.parse::<u32>().map_err(|_| {
            RestError::new("invalid_pagination", format!("{name} must be an integer"))
        })?,
        None => default,
    };
    if value == 0 || value > maximum {
        return Err(RestError::new(
            "invalid_pagination",
            format!("{name} must be 1..={maximum}"),
        ));
    }
    Ok(value)
}

pub(super) fn parse_u64(
    value: Option<&String>,
    default: u64,
    name: &str,
) -> Result<u64, RestError> {
    value.map_or(Ok(default), |value| {
        value
            .parse::<u64>()
            .map_err(|_| RestError::new("invalid_pagination", format!("{name} must be an integer")))
    })
}

// Serde cannot derive this external enum's representation for our cursor token, so keep the wire
// value stable with explicit implementations.
