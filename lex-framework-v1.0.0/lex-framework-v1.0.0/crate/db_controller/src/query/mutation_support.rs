//! Internal mutation validation and deadline handling.

use super::*;

pub(super) fn validate_update_values<T: Entity>(
    values: &Map<String, Value>,
) -> Result<Vec<String>, ModelError> {
    if values.is_empty() {
        return Err(ModelError::Validation("update has no fields".into()));
    }
    let schema = T::schema();
    let tenant_column = T::tenant_column();
    let version_column = T::version_column();
    let mut keys = values.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    for key in &keys {
        let spec = schema
            .get(key)
            .ok_or_else(|| ModelError::InvalidQuery(format!("unknown update field: {key}")))?;
        let framework_owned = key.as_str() == T::primary_key()
            || key == "created_at"
            || key == "updated_at"
            || T::soft_delete_column().is_some_and(|column| column == key.as_str())
            || tenant_column.is_some_and(|column| column == key.as_str())
            || version_column.is_some_and(|column| column == key.as_str());
        if framework_owned {
            return Err(ModelError::InvalidQuery(format!(
                "framework-owned field {key} cannot be changed through generic update()"
            )));
        }
        let value = values.get(key).ok_or_else(|| {
            ModelError::Other(format!("update field {key} disappeared during validation"))
        })?;
        if value.is_null() {
            if !spec.nullable {
                return Err(ModelError::Validation(format!(
                    "field {key} cannot be null"
                )));
            }
        } else {
            let _ = DbValue::from_json_for_column(spec, value)?;
        }
    }
    Ok(keys)
}

pub(super) fn validate_expression_update_field<'a, T: Entity>(
    schema: &'a HashMap<String, ColumnSpec>,
    field: &str,
) -> Result<&'a ColumnSpec, ModelError> {
    let spec = schema
        .get(field)
        .ok_or_else(|| ModelError::InvalidQuery(format!("unknown update field: {field}")))?;
    let framework_owned = field == T::primary_key()
        || field == "created_at"
        || field == "updated_at"
        || T::soft_delete_column().is_some_and(|column| column == field)
        || T::tenant_column().is_some_and(|column| column == field)
        || T::version_column().is_some_and(|column| column == field);
    if framework_owned {
        return Err(ModelError::InvalidQuery(format!(
            "framework-owned field {field} cannot be changed through update expressions"
        )));
    }
    Ok(spec)
}

pub(super) fn db_value_is_zero(value: &DbValue) -> bool {
    match value {
        DbValue::I16(value) => *value == 0,
        DbValue::I32(value) => *value == 0,
        DbValue::I64(value) => *value == 0,
        DbValue::F32(value) => *value == 0.0,
        DbValue::F64(value) => *value == 0.0,
        DbValue::Decimal(value) => *value == Decimal::ZERO,
        _ => false,
    }
}

pub(super) fn canonical_json_key(value: &Value) -> Result<String, ModelError> {
    serde_json::to_string(value).map_err(ModelError::from)
}

pub(super) async fn run_with_timeout<T>(
    timeout: Option<Duration>,
    future: impl std::future::Future<Output = Result<T, SqlxError>>,
) -> Result<T, ModelError> {
    if let Some(timeout) = timeout {
        return match tokio::time::timeout(timeout, future).await {
            Ok(result) => result.map_err(ModelError::from),
            Err(_) => Err(ModelError::QueryTimeout {
                millis: timeout.as_millis().min(u128::from(u64::MAX)) as u64,
            }),
        };
    }
    future.await.map_err(ModelError::from)
}
