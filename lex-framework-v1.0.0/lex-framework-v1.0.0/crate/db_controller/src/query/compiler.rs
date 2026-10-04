//! Validated SQL compilation; all data values remain bound parameters.

use super::*;

pub(super) fn build_select_sql<T: Entity>(
    plan: &QueryPlan<T>,
    start_index: usize,
) -> Result<(String, Vec<DbValue>), ModelError> {
    let table = ident(T::table_name())?;
    let columns = select_columns::<T>(Some("t0"))?;
    let distinct = if plan.distinct { "DISTINCT " } else { "" };
    let (where_sql, params) = build_where::<T>(plan, "t0", start_index)?;
    let ordering = build_ordering::<T>(plan, "t0")?;
    let pagination = build_pagination(plan)?;
    let lock = build_lock(plan)?;
    Ok((
        format!(
            "SELECT {distinct}{columns} FROM {table} AS t0 WHERE {where_sql}{ordering}{pagination}{lock}"
        ),
        params,
    ))
}

pub(super) fn build_where<T: Entity>(
    plan: &QueryPlan<T>,
    alias: &str,
    start_index: usize,
) -> Result<(String, Vec<DbValue>), ModelError> {
    if plan.force_empty {
        return Ok(("FALSE".to_owned(), Vec::new()));
    }

    let mut predicates = Vec::new();
    if let Some(predicate) = &plan.predicate {
        predicates.push(predicate.clone());
    }

    if let Some(column) = T::soft_delete_column() {
        if plan.only_deleted {
            predicates.push(Predicate::IsNull {
                field: column.to_owned(),
                is_null: false,
            });
        } else if !plan.include_deleted {
            predicates.push(Predicate::IsNull {
                field: column.to_owned(),
                is_null: true,
            });
        }
    } else if plan.only_deleted {
        return Err(ModelError::InvalidQuery(
            "only_deleted() used on a model without soft-delete metadata".into(),
        ));
    }

    if let Some(column) = T::tenant_column() {
        if let Some(tenant) = &plan.tenant {
            predicates.push(Predicate::Comparison {
                field: column.to_owned(),
                operator: ComparisonOperator::Eq,
                value: tenant.clone(),
            });
        } else if !plan.global_scope {
            return Err(ModelError::TenantScopeRequired {
                model: T::table_name(),
            });
        }
    }

    if predicates.is_empty() {
        return Ok(("TRUE".to_owned(), Vec::new()));
    }

    compile_predicate::<T>(&Predicate::And(predicates), alias, start_index)
}

pub(super) fn compile_predicate<T: Entity>(
    predicate: &Predicate,
    alias: &str,
    start_index: usize,
) -> Result<(String, Vec<DbValue>), ModelError> {
    let mut params = Vec::new();
    let sql = compile_predicate_inner::<T>(predicate, alias, start_index, &mut params)?;
    Ok((sql, params))
}

pub(super) fn compile_predicate_inner<T: Entity>(
    predicate: &Predicate,
    alias: &str,
    start_index: usize,
    params: &mut Vec<DbValue>,
) -> Result<String, ModelError> {
    match predicate {
        Predicate::Always(value) => Ok(if *value { "TRUE" } else { "FALSE" }.to_owned()),
        Predicate::Comparison {
            field,
            operator,
            value,
        } => {
            validate_field::<T>(field)?;
            validate_bound_value::<T>(field, value)?;
            let column = qualified_ident(alias, field)?;
            let index = start_index + params.len();
            params.push(value.clone());
            let placeholder = placeholder_for_field::<T>(field, index)?;
            Ok(format!("{column} {} {placeholder}", operator.as_sql()))
        }
        Predicate::FieldComparison {
            left,
            operator,
            right,
        } => {
            validate_field::<T>(left)?;
            validate_field::<T>(right)?;
            validate_field_comparison::<T>(left, right)?;
            let left = qualified_ident(alias, left)?;
            let right = qualified_ident(alias, right)?;
            Ok(format!("{left} {} {right}", operator.as_sql()))
        }
        Predicate::IsNull { field, is_null } => {
            validate_field::<T>(field)?;
            let column = qualified_ident(alias, field)?;
            Ok(format!(
                "{column} IS {}NULL",
                if *is_null { "" } else { "NOT " }
            ))
        }
        Predicate::In {
            field,
            values,
            negated,
        } => {
            validate_field::<T>(field)?;
            if values.len() > 10_000 {
                return Err(ModelError::InvalidQuery(
                    "IN predicates accept at most 10000 values".into(),
                ));
            }
            if values.is_empty() {
                return Ok(if *negated { "TRUE" } else { "FALSE" }.to_owned());
            }
            let column = qualified_ident(alias, field)?;
            let mut placeholders = Vec::with_capacity(values.len());
            for value in values {
                validate_bound_value::<T>(field, value)?;
                let index = start_index + params.len();
                params.push(value.clone());
                placeholders.push(placeholder_for_field::<T>(field, index)?);
            }
            Ok(format!(
                "{column} {}IN ({})",
                if *negated { "NOT " } else { "" },
                placeholders.join(", ")
            ))
        }
        Predicate::Between {
            field,
            lower,
            upper,
        } => {
            validate_field::<T>(field)?;
            validate_bound_value::<T>(field, lower)?;
            validate_bound_value::<T>(field, upper)?;
            let column = qualified_ident(alias, field)?;
            let lower_index = start_index + params.len();
            params.push(lower.clone());
            let upper_index = start_index + params.len();
            params.push(upper.clone());
            let lower_placeholder = placeholder_for_field::<T>(field, lower_index)?;
            let upper_placeholder = placeholder_for_field::<T>(field, upper_index)?;
            Ok(format!(
                "{column} BETWEEN {lower_placeholder} AND {upper_placeholder}"
            ))
        }
        Predicate::Text {
            field,
            lookup,
            value,
        } => {
            validate_field::<T>(field)?;
            validate_text_field::<T>(field)?;
            let column = qualified_ident(alias, field)?;
            let index = start_index + params.len();
            let escaped = escape_like(value);
            let (operator, pattern) = match lookup {
                TextLookup::Exact => ("LIKE", escaped),
                TextLookup::IExact => ("ILIKE", escaped),
                TextLookup::Contains => ("LIKE", format!("%{escaped}%")),
                TextLookup::IContains => ("ILIKE", format!("%{escaped}%")),
                TextLookup::StartsWith => ("LIKE", format!("{escaped}%")),
                TextLookup::IStartsWith => ("ILIKE", format!("{escaped}%")),
                TextLookup::EndsWith => ("LIKE", format!("%{escaped}")),
                TextLookup::IEndsWith => ("ILIKE", format!("%{escaped}")),
            };
            params.push(DbValue::String(pattern));
            // PostgreSQL requires ESCAPE to resolve to exactly one character. An explicit
            // escape-string literal is stable regardless of the standard_conforming_strings
            // server setting: E'\\\\' evaluates to one backslash.
            Ok(format!("{column} {operator} ${index} ESCAPE E'\\\\'"))
        }
        Predicate::DatePart {
            field,
            part,
            operator,
            value,
        } => {
            validate_date_part::<T>(field, *part, *value)?;
            let column = qualified_ident(alias, field)?;
            let index = start_index + params.len();
            params.push(DbValue::I32(*value));
            Ok(format!(
                "EXTRACT({} FROM {column}) {} ${index}",
                part.as_sql(),
                operator.as_sql()
            ))
        }
        Predicate::JsonContains { field, value } => {
            validate_json_field::<T>(field)?;
            let column = qualified_ident(alias, field)?;
            let index = start_index + params.len();
            params.push(DbValue::Json(value.clone()));
            Ok(format!("{column} @> ${index}::jsonb"))
        }
        Predicate::JsonHasKey { field, key } => {
            validate_json_field::<T>(field)?;
            if key.is_empty() || key.len() > 1_024 {
                return Err(ModelError::InvalidQuery(
                    "JSON key lookups require a 1..=1024 byte key".into(),
                ));
            }
            let column = qualified_ident(alias, field)?;
            let index = start_index + params.len();
            params.push(DbValue::String(key.clone()));
            Ok(format!("{column} ? ${index}"))
        }
        Predicate::And(values) => {
            if values.is_empty() {
                return Ok("TRUE".to_owned());
            }
            let mut compiled = Vec::with_capacity(values.len());
            for value in values {
                compiled.push(compile_predicate_inner::<T>(
                    value,
                    alias,
                    start_index,
                    params,
                )?);
            }
            Ok(format!("({})", compiled.join(" AND ")))
        }
        Predicate::Or(values) => {
            if values.is_empty() {
                return Ok("FALSE".to_owned());
            }
            let mut compiled = Vec::with_capacity(values.len());
            for value in values {
                compiled.push(compile_predicate_inner::<T>(
                    value,
                    alias,
                    start_index,
                    params,
                )?);
            }
            Ok(format!("({})", compiled.join(" OR ")))
        }
        Predicate::Not(value) => Ok(format!(
            "NOT ({})",
            compile_predicate_inner::<T>(value, alias, start_index, params)?
        )),
    }
}

pub(super) fn materialize_deterministic_ordering<T: Entity>(plan: &mut QueryPlan<T>) {
    if plan.ordering.is_empty() && plan.use_default_ordering {
        plan.ordering = T::default_ordering()
            .into_iter()
            .map(|field| match field.strip_prefix('-') {
                Some(name) => OrderBy::desc(name),
                None => OrderBy::asc(field),
            })
            .collect();
    }

    if plan.ordering.is_empty() {
        plan.ordering.push(OrderBy::asc(T::primary_key()));
        return;
    }

    if !plan
        .ordering
        .iter()
        .any(|order| order.field == T::primary_key())
    {
        let direction = plan
            .ordering
            .last()
            .map(|order| order.direction)
            .unwrap_or(SortDirection::Asc);
        plan.ordering.push(OrderBy {
            field: T::primary_key().to_owned(),
            direction,
            nulls: NullOrdering::DatabaseDefault,
        });
    }
}

pub(super) fn ensure_deterministic_first_ordering<T: Entity>(plan: &mut QueryPlan<T>) {
    materialize_deterministic_ordering::<T>(plan);
}

pub(super) fn reverse_effective_ordering<T: Entity>(plan: &mut QueryPlan<T>) {
    materialize_deterministic_ordering::<T>(plan);
    for order in &mut plan.ordering {
        order.direction = match order.direction {
            SortDirection::Asc => SortDirection::Desc,
            SortDirection::Desc => SortDirection::Asc,
        };
        order.nulls = match order.nulls {
            NullOrdering::DatabaseDefault => NullOrdering::DatabaseDefault,
            NullOrdering::First => NullOrdering::Last,
            NullOrdering::Last => NullOrdering::First,
        };
    }
}

pub(super) fn build_ordering<T: Entity>(
    plan: &QueryPlan<T>,
    alias: &str,
) -> Result<String, ModelError> {
    let mut ordering = plan.ordering.clone();
    if ordering.is_empty() && plan.use_default_ordering {
        for field in T::default_ordering() {
            let (direction, name) = match field.strip_prefix('-') {
                Some(value) => (SortDirection::Desc, value),
                None => (SortDirection::Asc, field),
            };
            ordering.push(OrderBy {
                field: name.to_owned(),
                direction,
                nulls: NullOrdering::DatabaseDefault,
            });
        }
    }
    if ordering.is_empty() {
        return Ok(String::new());
    }
    let mut parts = Vec::with_capacity(ordering.len());
    for order in ordering {
        validate_field::<T>(&order.field)?;
        let column = qualified_ident(alias, &order.field)?;
        let direction = match order.direction {
            SortDirection::Asc => "ASC",
            SortDirection::Desc => "DESC",
        };
        let nulls = match order.nulls {
            NullOrdering::DatabaseDefault => "",
            NullOrdering::First => " NULLS FIRST",
            NullOrdering::Last => " NULLS LAST",
        };
        parts.push(format!("{column} {direction}{nulls}"));
    }
    Ok(format!(" ORDER BY {}", parts.join(", ")))
}

pub(super) fn build_pagination<T>(plan: &QueryPlan<T>) -> Result<String, ModelError> {
    if plan.limit.is_none() && plan.offset == 0 {
        return Ok(String::new());
    }
    if plan.offset > i64::MAX as u64 {
        return Err(ModelError::InvalidQuery(
            "offset exceeds PostgreSQL range".into(),
        ));
    }
    let mut sql = String::new();
    if let Some(limit) = plan.limit {
        if limit == 0 || limit > 10_001 {
            return Err(ModelError::InvalidQuery("limit must be 1..=10001".into()));
        }
        sql.push_str(&format!(" LIMIT {limit}"));
    } else if plan.offset > 0 {
        sql.push_str(" LIMIT ALL");
    }
    if plan.offset > 0 {
        sql.push_str(&format!(" OFFSET {}", plan.offset));
    }
    Ok(sql)
}

pub(super) fn build_lock<T>(plan: &QueryPlan<T>) -> Result<String, ModelError> {
    let Some((lock, wait)) = plan.lock else {
        return Ok(String::new());
    };
    let lock = match lock {
        RowLock::Update => "FOR UPDATE",
        RowLock::NoKeyUpdate => "FOR NO KEY UPDATE",
        RowLock::Share => "FOR SHARE",
        RowLock::KeyShare => "FOR KEY SHARE",
    };
    let wait = match wait {
        LockWait::Wait => "",
        LockWait::NoWait => " NOWAIT",
        LockWait::SkipLocked => " SKIP LOCKED",
    };
    Ok(format!(" {lock}{wait}"))
}

pub(super) fn build_lock_for_alias<T>(
    plan: &QueryPlan<T>,
    alias: &str,
) -> Result<String, ModelError> {
    let Some((lock, wait)) = plan.lock else {
        return Ok(String::new());
    };
    if !matches!(alias, "t0" | "t1") {
        return Err(ModelError::SqlBuildError(
            "unsupported internal SQL alias for row lock".into(),
        ));
    }
    let lock = match lock {
        RowLock::Update => "FOR UPDATE",
        RowLock::NoKeyUpdate => "FOR NO KEY UPDATE",
        RowLock::Share => "FOR SHARE",
        RowLock::KeyShare => "FOR KEY SHARE",
    };
    let wait = match wait {
        LockWait::Wait => "",
        LockWait::NoWait => " NOWAIT",
        LockWait::SkipLocked => " SKIP LOCKED",
    };
    Ok(format!(" {lock} OF {alias}{wait}"))
}

pub(super) fn select_columns<T: Entity>(alias: Option<&str>) -> Result<String, ModelError> {
    let mut columns = T::schema().into_keys().collect::<Vec<_>>();
    columns.sort();
    if columns.is_empty() {
        return Err(ModelError::SqlBuildError(format!(
            "model {} has no persisted fields",
            T::table_name()
        )));
    }
    columns
        .into_iter()
        .map(|column| match alias {
            Some(alias) => qualified_ident(alias, &column),
            None => ident(&column),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|values| values.join(", "))
}

pub(super) fn validate_field<T: Entity>(field: &str) -> Result<(), ModelError> {
    if !effective_schema::<T>().contains_key(field) {
        return Err(ModelError::InvalidQuery(format!(
            "unknown field {field} for model {}",
            T::table_name()
        )));
    }
    Ok(())
}

pub(super) fn validate_bound_value<T: Entity>(
    field: &str,
    value: &DbValue,
) -> Result<(), ModelError> {
    let schema = effective_schema::<T>();
    let spec = schema
        .get(field)
        .ok_or_else(|| ModelError::InvalidQuery(format!("unknown field: {field}")))?;
    if db_value_matches_sql_type(spec, value) {
        Ok(())
    } else {
        Err(ModelError::InvalidQuery(format!(
            "bound value type is incompatible with {field} ({})",
            spec.sql_type
        )))
    }
}

pub(super) fn db_value_matches_sql_type(spec: &ColumnSpec, value: &DbValue) -> bool {
    let sql_type = spec.sql_type.trim().to_ascii_uppercase();
    match sql_type.as_str() {
        "BOOLEAN" => matches!(value, DbValue::Bool(_)),
        "SMALLINT" | "INTEGER" | "BIGINT" | "BIGSERIAL" => {
            matches!(value, DbValue::I16(_) | DbValue::I32(_) | DbValue::I64(_))
        }
        "REAL" | "DOUBLE PRECISION" | "NUMERIC" => matches!(
            value,
            DbValue::I16(_)
                | DbValue::I32(_)
                | DbValue::I64(_)
                | DbValue::F32(_)
                | DbValue::F64(_)
                | DbValue::Decimal(_)
        ),
        "UUID" => matches!(value, DbValue::Uuid(_)),
        "TIMESTAMP" | "TIMESTAMPTZ" => matches!(value, DbValue::DateTime(_)),
        "JSON" | "JSONB" => matches!(value, DbValue::Json(_)),
        "BYTEA" => matches!(value, DbValue::Bytes(_)),
        "TEXT" | "CHARACTER VARYING" | "DATE" | "TIME" | "INTERVAL" | "INET" | "CIDR" => {
            matches!(value, DbValue::String(_))
        }
        _ => false,
    }
}

pub(super) fn is_numeric_sql_type(sql_type: &str) -> bool {
    matches!(
        sql_type.trim().to_ascii_uppercase().as_str(),
        "SMALLINT" | "INTEGER" | "BIGINT" | "BIGSERIAL" | "REAL" | "DOUBLE PRECISION" | "NUMERIC"
    )
}

pub(super) fn sql_types_compatible(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
        || (is_numeric_sql_type(left) && is_numeric_sql_type(right))
}

pub(super) fn validate_field_comparison<T: Entity>(
    left: &str,
    right: &str,
) -> Result<(), ModelError> {
    let schema = effective_schema::<T>();
    let left_spec = schema
        .get(left)
        .ok_or_else(|| ModelError::InvalidQuery(format!("unknown field: {left}")))?;
    let right_spec = schema
        .get(right)
        .ok_or_else(|| ModelError::InvalidQuery(format!("unknown field: {right}")))?;
    if sql_types_compatible(&left_spec.sql_type, &right_spec.sql_type) {
        return Ok(());
    }
    Err(ModelError::InvalidQuery(format!(
        "cannot compare incompatible fields {left} ({}) and {right} ({})",
        left_spec.sql_type, right_spec.sql_type
    )))
}

pub(super) fn validate_aggregate<T: Entity>(
    field: &str,
    function: AggregateFunction,
) -> Result<(), ModelError> {
    validate_field::<T>(field)?;
    let schema = effective_schema::<T>();
    let spec = schema
        .get(field)
        .ok_or_else(|| ModelError::InvalidQuery(format!("unknown field: {field}")))?;
    let sql_type = spec.sql_type.trim().to_ascii_uppercase();
    let numeric = matches!(
        sql_type.as_str(),
        "SMALLINT" | "INTEGER" | "BIGINT" | "BIGSERIAL" | "REAL" | "DOUBLE PRECISION" | "NUMERIC"
    );
    let boolean = sql_type == "BOOLEAN";
    match function {
        AggregateFunction::Sum
        | AggregateFunction::Avg
        | AggregateFunction::StdDev
        | AggregateFunction::Variance
            if !numeric =>
        {
            Err(ModelError::InvalidQuery(format!(
                "aggregate {:?} requires a numeric field",
                function
            )))
        }
        AggregateFunction::BoolAnd | AggregateFunction::BoolOr if !boolean => Err(
            ModelError::InvalidQuery(format!("aggregate {:?} requires a boolean field", function)),
        ),
        _ => Ok(()),
    }
}

pub(super) fn write_map<T: Entity>(
    model: &T,
    include_primary_key: bool,
) -> Result<Map<String, Value>, ModelError> {
    let schema = T::schema();
    let primary_key = T::primary_key();
    let mut output = Map::new();
    for (field, value) in model.to_map() {
        let spec = schema.get(&field).ok_or_else(|| {
            ModelError::Validation(format!("model emitted unknown persisted field: {field}"))
        })?;
        if !include_primary_key && field == primary_key {
            continue;
        }
        if value.is_null() && !spec.nullable {
            return Err(ModelError::Validation(format!(
                "field {field} cannot be null"
            )));
        }
        output.insert(field, value);
    }
    if output.is_empty() {
        return Err(ModelError::Validation(
            "model has no persistable fields".into(),
        ));
    }
    Ok(output)
}

pub(super) fn build_insert_sql<T: Entity>(
    map: &Map<String, Value>,
    conflict_field: Option<&str>,
    ignore_conflict: bool,
) -> Result<(String, Vec<DbValue>), ModelError> {
    let table = ident(T::table_name())?;
    let schema = effective_schema::<T>();
    let mut fields = map.keys().cloned().collect::<Vec<_>>();
    fields.sort();
    if fields.is_empty() {
        return Err(ModelError::Validation(
            "insert has no persisted fields".into(),
        ));
    }
    let columns = fields
        .iter()
        .map(|field| ident(field))
        .collect::<Result<Vec<_>, _>>()?;
    let mut params = Vec::with_capacity(fields.len());
    let mut placeholders = Vec::with_capacity(fields.len());
    for field in &fields {
        let spec = schema
            .get(field)
            .ok_or_else(|| ModelError::InvalidQuery(format!("unknown insert field: {field}")))?;
        let value = map
            .get(field)
            .ok_or_else(|| ModelError::Validation(format!("missing insert value: {field}")))?;
        if value.is_null() {
            if !spec.nullable {
                return Err(ModelError::Validation(format!(
                    "field {field} cannot be null"
                )));
            }
            // Null bindings use a typed cast on PostgreSQL. SQLite strips the cast in its executor.
            placeholders.push("NULL".to_owned());
        } else {
            params.push(DbValue::from_json_for_column(spec, value)?);
            placeholders.push(placeholder_for_field::<T>(field, params.len())?);
        }
    }
    let returning = select_columns::<T>(None)?;
    let conflict = match conflict_field {
        Some(field) => {
            validate_field::<T>(field)?;
            if ignore_conflict {
                format!(" ON CONFLICT ({}) DO NOTHING", ident(field)?)
            } else {
                String::new()
            }
        }
        None => String::new(),
    };
    let sql = format!(
        "INSERT INTO {table} ({}) VALUES ({}){conflict} RETURNING {returning}",
        columns.join(", "),
        placeholders.join(", ")
    );
    Ok((sql, params))
}

pub(super) fn placeholder_for_field<T: Entity>(
    field: &str,
    index: usize,
) -> Result<String, ModelError> {
    let schema = effective_schema::<T>();
    let spec = schema
        .get(field)
        .ok_or_else(|| ModelError::InvalidQuery(format!("unknown field: {field}")))?;
    let cast = match spec.sql_type.trim().to_ascii_uppercase().as_str() {
        "DATE" => Some("DATE"),
        "TIME" => Some("TIME"),
        "INTERVAL" => Some("INTERVAL"),
        "INET" => Some("INET"),
        "CIDR" => Some("CIDR"),
        _ => None,
    };
    Ok(match cast {
        Some(cast) => format!("${index}::{cast}"),
        None => format!("${index}"),
    })
}

pub(super) fn validate_date_part<T: Entity>(
    field: &str,
    part: DatePart,
    value: i32,
) -> Result<(), ModelError> {
    let schema = effective_schema::<T>();
    let spec = schema
        .get(field)
        .ok_or_else(|| ModelError::InvalidQuery(format!("unknown field: {field}")))?;
    if !matches!(
        spec.sql_type.trim().to_ascii_uppercase().as_str(),
        "DATE" | "TIMESTAMP" | "TIMESTAMPTZ"
    ) {
        return Err(ModelError::InvalidQuery(format!(
            "date-part lookup cannot be used with {field} ({})",
            spec.sql_type
        )));
    }
    let valid = match part {
        // PostgreSQL DATE and TIMESTAMP have different representable year ranges; the database
        // remains authoritative for the model's exact temporal type.
        DatePart::Year => true,
        DatePart::Quarter => (1..=4).contains(&value),
        DatePart::Month => (1..=12).contains(&value),
        DatePart::Week => (1..=53).contains(&value),
        DatePart::Day => (1..=31).contains(&value),
        DatePart::Hour => (0..=23).contains(&value),
        DatePart::Minute | DatePart::Second => (0..=59).contains(&value),
    };
    if !valid {
        return Err(ModelError::InvalidQuery(format!(
            "invalid {:?} date-part value: {value}",
            part
        )));
    }
    Ok(())
}

pub(super) fn validate_json_field<T: Entity>(field: &str) -> Result<(), ModelError> {
    let schema = effective_schema::<T>();
    let spec = schema
        .get(field)
        .ok_or_else(|| ModelError::InvalidQuery(format!("unknown field: {field}")))?;
    if !matches!(spec.sql_type.trim().to_ascii_uppercase().as_str(), "JSONB") {
        return Err(ModelError::InvalidQuery(format!(
            "JSONB lookup cannot be used with {field} ({})",
            spec.sql_type
        )));
    }
    Ok(())
}

pub(super) fn validate_text_field<T: Entity>(field: &str) -> Result<(), ModelError> {
    let schema = effective_schema::<T>();
    let spec = schema
        .get(field)
        .ok_or_else(|| ModelError::InvalidQuery(format!("unknown field: {field}")))?;
    if !matches!(
        spec.sql_type.trim().to_ascii_uppercase().as_str(),
        "TEXT" | "CHARACTER VARYING"
    ) {
        return Err(ModelError::InvalidQuery(format!(
            "text lookup cannot be used with {} ({})",
            field, spec.sql_type
        )));
    }
    Ok(())
}

pub(super) fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub(super) fn ident(value: &str) -> Result<String, ModelError> {
    quote_identifier(value)
        .ok_or_else(|| ModelError::SqlBuildError(format!("unsafe SQL identifier: {value}")))
}

pub(super) fn qualified_ident(alias: &str, column: &str) -> Result<String, ModelError> {
    if !matches!(alias, "t0" | "t1") {
        return Err(ModelError::SqlBuildError(
            "unsupported internal SQL alias".into(),
        ));
    }
    Ok(format!("{alias}.{}", ident(column)?))
}

/// Build a predicate from a schema-aware dynamic filter (used by REST adapters).
pub fn dynamic_predicate<T: Entity>(
    field: &str,
    lookup: &str,
    raw: &str,
) -> Result<Predicate, ModelError> {
    let schema = effective_schema::<T>();
    let spec = schema
        .get(field)
        .ok_or_else(|| ModelError::InvalidQuery(format!("unknown filter field: {field}")))?;
    match lookup {
        "exact" => Ok(Predicate::Comparison {
            field: field.to_owned(),
            operator: ComparisonOperator::Eq,
            value: DbValue::parse_for_column(spec, raw)?,
        }),
        "neq" => Ok(Predicate::Comparison {
            field: field.to_owned(),
            operator: ComparisonOperator::NotEq,
            value: DbValue::parse_for_column(spec, raw)?,
        }),
        "gt" | "gte" | "lt" | "lte" => {
            let operator = match lookup {
                "gt" => ComparisonOperator::Gt,
                "gte" => ComparisonOperator::Gte,
                "lt" => ComparisonOperator::Lt,
                "lte" => ComparisonOperator::Lte,
                _ => unreachable!(),
            };
            Ok(Predicate::Comparison {
                field: field.to_owned(),
                operator,
                value: DbValue::parse_for_column(spec, raw)?,
            })
        }
        "isnull" => {
            let value = raw
                .parse::<bool>()
                .map_err(|_| ModelError::InvalidQuery("isnull expects true or false".into()))?;
            Ok(Predicate::IsNull {
                field: field.to_owned(),
                is_null: value,
            })
        }
        "in" => {
            let parts = raw
                .split(',')
                .filter(|value| !value.trim().is_empty())
                .collect::<Vec<_>>();
            if parts.len() > 1_000 {
                return Err(ModelError::InvalidQuery(
                    "REST IN filters accept at most 1000 values".into(),
                ));
            }
            let values = parts
                .into_iter()
                .map(|value| DbValue::parse_for_column(spec, value.trim()))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Predicate::In {
                field: field.to_owned(),
                values,
                negated: false,
            })
        }
        "range" => {
            let mut values = raw.splitn(2, ',').map(str::trim);
            let lower = values
                .next()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| ModelError::InvalidQuery("range requires lower,upper".into()))?;
            let upper = values
                .next()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| ModelError::InvalidQuery("range requires lower,upper".into()))?;
            Ok(Predicate::Between {
                field: field.to_owned(),
                lower: DbValue::parse_for_column(spec, lower)?,
                upper: DbValue::parse_for_column(spec, upper)?,
            })
        }
        "year" | "quarter" | "month" | "week" | "day" | "hour" | "minute" | "second" => {
            let value = raw
                .parse::<i32>()
                .map_err(|_| ModelError::InvalidQuery(format!("{lookup} expects an integer")))?;
            let part = match lookup {
                "year" => DatePart::Year,
                "quarter" => DatePart::Quarter,
                "month" => DatePart::Month,
                "week" => DatePart::Week,
                "day" => DatePart::Day,
                "hour" => DatePart::Hour,
                "minute" => DatePart::Minute,
                "second" => DatePart::Second,
                _ => unreachable!(),
            };
            validate_date_part::<T>(field, part, value)?;
            Ok(Predicate::DatePart {
                field: field.to_owned(),
                part,
                operator: ComparisonOperator::Eq,
                value,
            })
        }
        "jsoncontains" => {
            validate_json_field::<T>(field)?;
            let value = serde_json::from_str::<Value>(raw)
                .map_err(|_| ModelError::InvalidQuery("jsoncontains expects valid JSON".into()))?;
            Ok(Predicate::JsonContains {
                field: field.to_owned(),
                value,
            })
        }
        "haskey" => {
            validate_json_field::<T>(field)?;
            if raw.is_empty() || raw.len() > 1_024 {
                return Err(ModelError::InvalidQuery(
                    "haskey expects a 1..=1024 byte key".into(),
                ));
            }
            Ok(Predicate::JsonHasKey {
                field: field.to_owned(),
                key: raw.to_owned(),
            })
        }
        "contains" | "icontains" | "startswith" | "istartswith" | "endswith" | "iendswith"
        | "iexact" => {
            validate_text_field::<T>(field)?;
            let lookup = match lookup {
                "contains" => TextLookup::Contains,
                "icontains" => TextLookup::IContains,
                "startswith" => TextLookup::StartsWith,
                "istartswith" => TextLookup::IStartsWith,
                "endswith" => TextLookup::EndsWith,
                "iendswith" => TextLookup::IEndsWith,
                "iexact" => TextLookup::IExact,
                _ => unreachable!(),
            };
            Ok(Predicate::Text {
                field: field.to_owned(),
                lookup,
                value: raw.to_owned(),
            })
        }
        other => Err(ModelError::InvalidQuery(format!(
            "unsupported lookup: {other}"
        ))),
    }
}
