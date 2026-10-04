//! SQLite stores exact decimals as text. Never silently substitute lexical comparison or
//! IEEE-754 arithmetic for the numeric contract of PostgreSQL NUMERIC.
use super::*;

pub(super) fn is_exact_decimal(spec: &ColumnSpec) -> bool {
    let kind = spec.sql_type.trim().to_ascii_uppercase();
    kind.starts_with("NUMERIC") || kind.starts_with("DECIMAL")
}

pub(super) fn reject_sqlite_decimal_field<T: Entity>(
    field: &str,
    operation: &str,
) -> Result<(), ModelError> {
    if effective_schema::<T>()
        .get(field)
        .is_some_and(is_exact_decimal)
    {
        return Err(ModelError::InvalidQuery(format!(
            "SQLite exact decimal {operation} is unsupported for {field}; use PostgreSQL or integer minor units"
        )));
    }
    Ok(())
}

pub(crate) fn validate_sqlite_decimal_schema<T: Entity>() -> Result<(), ModelError> {
    let schema = effective_schema::<T>();
    for (field, spec) in &schema {
        if is_exact_decimal(spec) && (spec.unique || spec.primary_key) {
            reject_sqlite_decimal_field::<T>(field, "uniqueness")?;
        }
    }
    for constraint in T::unique_constraints() {
        for field in constraint.columns {
            reject_sqlite_decimal_field::<T>(field, "uniqueness")?;
        }
    }
    for index in T::typed_indexes().into_iter().filter(|index| index.unique) {
        for field in index.columns {
            reject_sqlite_decimal_field::<T>(field, "uniqueness")?;
        }
    }
    Ok(())
}

fn validate_predicate<T: Entity>(predicate: &Predicate) -> Result<(), ModelError> {
    match predicate {
        Predicate::Always(_) | Predicate::IsNull { .. } => Ok(()),
        Predicate::And(predicates) | Predicate::Or(predicates) => {
            for predicate in predicates {
                validate_predicate::<T>(predicate)?;
            }
            Ok(())
        }
        Predicate::Not(predicate) => validate_predicate::<T>(predicate),
        Predicate::FieldComparison { left, right, .. } => {
            reject_sqlite_decimal_field::<T>(left, "comparison")?;
            reject_sqlite_decimal_field::<T>(right, "comparison")
        }
        Predicate::Comparison { field, .. }
        | Predicate::In { field, .. }
        | Predicate::Between { field, .. }
        | Predicate::Text { field, .. }
        | Predicate::DatePart { field, .. }
        | Predicate::JsonContains { field, .. }
        | Predicate::JsonHasKey { field, .. } => {
            reject_sqlite_decimal_field::<T>(field, "comparison")
        }
    }
}

pub(super) fn validate_sqlite_decimal_plan<T: Entity>(
    plan: &QueryPlan<T>,
    check_ordering: bool,
) -> Result<(), ModelError> {
    validate_sqlite_decimal_schema::<T>()?;
    if let Some(predicate) = &plan.predicate {
        validate_predicate::<T>(predicate)?;
    }
    if !check_ordering {
        return Ok(());
    }
    for order in &plan.ordering {
        reject_sqlite_decimal_field::<T>(&order.field, "ordering")?;
    }
    if plan.ordering.is_empty() && plan.use_default_ordering {
        for field in T::default_ordering() {
            reject_sqlite_decimal_field::<T>(field.trim_start_matches('-'), "ordering")?;
        }
    }
    Ok(())
}
