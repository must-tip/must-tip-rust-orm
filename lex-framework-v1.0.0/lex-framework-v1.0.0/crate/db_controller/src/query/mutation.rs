//! Public mutation terminals and scoped execution.

use super::*;

impl<T, E> QuerySet<T, E>
where
    T: Entity,
    E: QueryExecutor,
{
    pub async fn create(mut self, model: &T) -> Result<T, ModelError>
    where
        T: for<'r> FromRow<'r, E::Row> + Send + Unpin,
    {
        model.validate_model().map_err(ModelError::Validation)?;
        let map = write_map::<T>(model, true)?;
        self.validate_write_scope(&map)?;
        let (sql, params) = build_insert_sql::<T>(&map, None, false)?;
        let timeout = self.plan.timeout;
        let mut rows = run_with_timeout(timeout, self.executor.fetch_all::<T>(sql, params)).await?;
        rows.pop()
            .ok_or_else(|| ModelError::Other("insert returned no row".into()))
    }

    /// Insert up to 5,000 rows in one atomic statement on PostgreSQL or SQLite.
    pub async fn bulk_create(
        mut self,
        models: &[T],
        ignore_conflicts: bool,
    ) -> Result<Vec<T>, ModelError>
    where
        T: for<'r> FromRow<'r, E::Row> + Send + Unpin,
    {
        if models.is_empty() {
            return Ok(Vec::new());
        }
        if models.len() > 5_000 {
            return Err(ModelError::InvalidQuery(
                "bulk_create accepts at most 5000 rows per statement".into(),
            ));
        }
        let mut maps = Vec::with_capacity(models.len());
        let mut canonical_keys: Option<Vec<String>> = None;
        for model in models {
            model.validate_model().map_err(ModelError::Validation)?;
            let map = write_map::<T>(model, true)?;
            self.validate_write_scope(&map)?;
            // Validate the whole batch before any statement can persist a row.
            if self.executor.dialect().is_sqlite() {
                let _ = build_insert_sql::<T>(&map, None, false)?;
            }
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            if let Some(expected) = &canonical_keys {
                if expected != &keys {
                    return Err(ModelError::Validation(
                        "bulk_create rows must expose the same persisted fields".into(),
                    ));
                }
            } else {
                canonical_keys = Some(keys);
            }
            maps.push(Value::Object(map));
        }
        let keys = canonical_keys
            .ok_or_else(|| ModelError::Validation("bulk_create has no fields".into()))?;
        let table = ident(T::table_name())?;
        let columns = keys
            .iter()
            .map(|field| ident(field))
            .collect::<Result<Vec<_>, _>>()?;
        let returning = select_columns::<T>(None)?;
        if self.executor.dialect().is_sqlite() {
            let selections = keys
                .iter()
                .map(|field| format!("json_extract(src.value, '$.{field}')"))
                .collect::<Vec<_>>()
                .join(", ");
            let conflict = if ignore_conflicts {
                " ON CONFLICT DO NOTHING"
            } else {
                ""
            };
            // A single statement preserves atomicity even when using a pooled executor.
            // WHERE disambiguates SQLite's INSERT SELECT ... ON CONFLICT grammar.
            let sql = format!(
                "INSERT INTO {table} ({}) SELECT {selections} FROM json_each($1) AS src WHERE 1{conflict} RETURNING {returning}",
                columns.join(", ")
            );
            let timeout = self.plan.timeout;
            return run_with_timeout(
                timeout,
                self.executor
                    .fetch_all::<T>(sql, vec![DbValue::Json(Value::Array(maps))]),
            )
            .await;
        }
        let conflict = if ignore_conflicts {
            " ON CONFLICT DO NOTHING"
        } else {
            ""
        };
        let sql = format!(
            "INSERT INTO {table} ({}) SELECT {} FROM jsonb_populate_recordset(NULL::{table}, $1::jsonb) AS src{conflict} RETURNING {returning}",
            columns.join(", "),
            columns
                .iter()
                .map(|column| format!("src.{column}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let timeout = self.plan.timeout;
        run_with_timeout(
            timeout,
            self.executor
                .fetch_all::<T>(sql, vec![DbValue::Json(Value::Array(maps))]),
        )
        .await
    }

    /// Update up to 5,000 explicitly identified models in one atomic statement.
    ///
    /// Rows are joined by primary key and still pass through the QuerySet tenant/soft-delete
    /// scope. Duplicate primary keys are rejected to avoid UPDATE ... FROM ambiguity.
    pub async fn bulk_update(mut self, models: &[T], fields: &[&str]) -> Result<u64, ModelError> {
        if models.is_empty() {
            return Ok(0);
        }
        if models.len() > 5_000 {
            return Err(ModelError::InvalidQuery(
                "bulk_update accepts at most 5000 rows per statement".into(),
            ));
        }
        if fields.is_empty() || fields.len() > 100 {
            return Err(ModelError::InvalidQuery(
                "bulk_update requires 1..=100 fields".into(),
            ));
        }

        let schema = T::schema();
        let primary_key = T::primary_key();
        let primary_spec = schema.get(primary_key).ok_or_else(|| {
            ModelError::InvalidQuery(format!(
                "bulk_update requires an explicit model primary key field: {primary_key}"
            ))
        })?;
        let mut update_fields = Vec::with_capacity(fields.len());
        let mut unique_fields = BTreeSet::new();
        for field in fields {
            validate_expression_update_field::<T>(&schema, field)?;
            if !unique_fields.insert((*field).to_owned()) {
                return Err(ModelError::InvalidQuery(format!(
                    "duplicate bulk_update field: {field}"
                )));
            }
            update_fields.push((*field).to_owned());
        }
        update_fields.sort();

        let mut rows = Vec::with_capacity(models.len());
        let mut primary_keys = BTreeSet::new();
        for model in models {
            model.validate_model().map_err(ModelError::Validation)?;
            let full = write_map::<T>(model, true)?;
            self.validate_write_scope(&full)?;
            let primary_value = full.get(primary_key).ok_or_else(|| {
                ModelError::Validation(format!(
                    "bulk_update model did not emit primary key: {primary_key}"
                ))
            })?;
            if primary_value.is_null() {
                return Err(ModelError::Validation(format!(
                    "bulk_update primary key cannot be null: {primary_key}"
                )));
            }
            let _ = DbValue::from_json_for_column(primary_spec, primary_value)?;
            let primary_identity = canonical_json_key(primary_value)?;
            if !primary_keys.insert(primary_identity) {
                return Err(ModelError::InvalidQuery(
                    "bulk_update contains duplicate primary keys".into(),
                ));
            }

            let mut row = Map::new();
            row.insert(primary_key.to_owned(), primary_value.clone());
            for field in &update_fields {
                let value = full.get(field).ok_or_else(|| {
                    ModelError::Validation(format!("bulk_update model did not emit field: {field}"))
                })?;
                if self.executor.dialect().is_sqlite() {
                    reject_sqlite_decimal_field::<T>(field, "lookup")?;
                }
                let spec = schema.get(field).ok_or_else(|| {
                    ModelError::InvalidQuery(format!("unknown bulk_update field: {field}"))
                })?;
                if value.is_null() {
                    if !spec.nullable {
                        return Err(ModelError::Validation(format!(
                            "field {field} cannot be null"
                        )));
                    }
                } else {
                    let _ = DbValue::from_json_for_column(spec, value)?;
                }
                row.insert(field.clone(), value.clone());
            }
            rows.push(Value::Object(row));
        }

        let (where_sql, predicate_params) = build_where::<T>(&self.plan, "t0", 2)?;
        let table = ident(T::table_name())?;
        let primary = ident(primary_key)?;
        let assignments = update_fields
            .iter()
            .map(|field| {
                let column = ident(field)?;
                Ok(if self.executor.dialect().is_sqlite() {
                    format!("{column} = json_extract(src.value, '$.{field}')")
                } else {
                    format!("{column} = src.{column}")
                })
            })
            .collect::<Result<Vec<_>, ModelError>>()?
            .join(", ");
        let version_assignment = T::version_column()
            .map(|version| ident(version).map(|version| format!(", {version} = t0.{version} + 1")))
            .transpose()?
            .unwrap_or_default();
        let (source, source_primary) = if self.executor.dialect().is_sqlite() {
            (
                "json_each($1)".to_owned(),
                format!("json_extract(src.value, '$.{primary_key}')"),
            )
        } else {
            (
                format!("jsonb_populate_recordset(NULL::{table}, $1::jsonb)"),
                format!("src.{primary}"),
            )
        };
        let sql = format!(
            "UPDATE {table} AS t0 SET {assignments}{version_assignment}, \"updated_at\" = now() FROM {source} AS src WHERE t0.{primary} = {source_primary} AND ({where_sql})"
        );
        let mut params = vec![DbValue::Json(Value::Array(rows))];
        params.extend(predicate_params);
        let timeout = self.plan.timeout;
        run_with_timeout(timeout, self.executor.execute(sql, params)).await
    }

    /// Concurrency-safe single-column get-or-create for primary-key or UNIQUE fields.
    pub async fn get_or_create_by<V: Into<DbValue>>(
        mut self,
        field: &str,
        value: V,
        defaults: &T,
    ) -> Result<(T, bool), ModelError>
    where
        T: for<'r> FromRow<'r, E::Row> + Send + Unpin,
    {
        let schema = T::schema();
        if self.executor.dialect().is_sqlite() {
            reject_sqlite_decimal_field::<T>(field, "lookup")?;
        }
        let spec = schema.get(field).ok_or_else(|| {
            ModelError::InvalidQuery(format!("unknown get_or_create field: {field}"))
        })?;
        if field != T::primary_key() && !spec.unique {
            return Err(ModelError::InvalidQuery(format!(
                "get_or_create_by requires a primary-key or UNIQUE field: {field}"
            )));
        }
        defaults.validate_model().map_err(ModelError::Validation)?;
        let map = write_map::<T>(defaults, true)?;
        self.validate_write_scope(&map)?;
        let expected = value.into();
        let actual_json = map.get(field).ok_or_else(|| {
            ModelError::Validation(format!("model did not emit conflict field: {field}"))
        })?;
        let actual = DbValue::from_json_for_column(spec, actual_json)?;
        if actual != expected {
            return Err(ModelError::Validation(format!(
                "get_or_create conflict value does not match model field: {field}"
            )));
        }

        let (insert_sql, params) = build_insert_sql::<T>(&map, Some(field), true)?;
        let timeout = self.plan.timeout;
        let mut created =
            run_with_timeout(timeout, self.executor.fetch_all::<T>(insert_sql, params)).await?;
        if let Some(model) = created.pop() {
            return Ok((model, true));
        }

        self.plan.user_predicate = true;
        let lookup = Predicate::Comparison {
            field: field.to_owned(),
            operator: ComparisonOperator::Eq,
            value: expected,
        };
        self.plan.predicate = Some(match self.plan.predicate.take() {
            Some(existing) => existing.and(lookup),
            None => lookup,
        });
        match self.get().await {
            Ok(model) => Ok((model, false)),
            Err(ModelError::NotFound) => Err(ModelError::Conflict(format!(
                "unique field {field} conflicts with a row outside the current QuerySet visibility (for example a soft-deleted or differently scoped row)"
            ))),
            Err(error) => Err(error),
        }
    }

    /// Transaction-only Django-style update_or_create for one primary-key or UNIQUE field.
    /// The transaction requirement guarantees the insert-conflict/update/read sequence stays on
    /// one database transaction instead of racing across pooled connections.
    pub async fn update_or_create_by<V: Into<DbValue>>(
        mut self,
        field: &str,
        value: V,
        desired: &T,
    ) -> Result<(T, bool), ModelError>
    where
        T: for<'r> FromRow<'r, E::Row> + Send + Unpin,
    {
        if !self.executor.is_transactional() {
            return Err(ModelError::TransactionRequired);
        }
        let schema = T::schema();
        if self.executor.dialect().is_sqlite() {
            reject_sqlite_decimal_field::<T>(field, "lookup")?;
        }
        let spec = schema.get(field).ok_or_else(|| {
            ModelError::InvalidQuery(format!("unknown update_or_create field: {field}"))
        })?;
        if field != T::primary_key() && !spec.unique {
            return Err(ModelError::InvalidQuery(format!(
                "update_or_create_by requires a primary-key or UNIQUE field: {field}"
            )));
        }
        desired.validate_model().map_err(ModelError::Validation)?;
        let full = write_map::<T>(desired, true)?;
        self.validate_write_scope(&full)?;
        let expected = value.into();
        let actual_json = full.get(field).ok_or_else(|| {
            ModelError::Validation(format!(
                "model did not emit update_or_create conflict field: {field}"
            ))
        })?;
        let actual = DbValue::from_json_for_column(spec, actual_json)?;
        if actual != expected {
            return Err(ModelError::Validation(format!(
                "update_or_create conflict value does not match model field: {field}"
            )));
        }

        let (insert_sql, insert_params) = build_insert_sql::<T>(&full, Some(field), true)?;
        let timeout = self.plan.timeout;
        let mut inserted = run_with_timeout(
            timeout,
            self.executor.fetch_all::<T>(insert_sql, insert_params),
        )
        .await?;
        if let Some(model) = inserted.pop() {
            return Ok((model, true));
        }

        let mut patch = Map::new();
        for (name, field_value) in &full {
            if name == field || name == T::primary_key() {
                continue;
            }
            if validate_expression_update_field::<T>(&schema, name).is_ok() {
                patch.insert(name.clone(), field_value.clone());
            }
        }

        self.plan.user_predicate = true;
        let lookup = Predicate::Comparison {
            field: field.to_owned(),
            operator: ComparisonOperator::Eq,
            value: expected,
        };
        self.plan.predicate = Some(match self.plan.predicate.take() {
            Some(existing) => existing.and(lookup),
            None => lookup,
        });

        if !patch.is_empty() {
            let keys = validate_update_values::<T>(&patch)?;
            let schema = effective_schema::<T>();
            let mut params = Vec::with_capacity(keys.len());
            let mut assignments = Vec::with_capacity(keys.len());
            for key in &keys {
                let column = ident(key)?;
                let spec = schema.get(key).ok_or_else(|| {
                    ModelError::InvalidQuery(format!("unknown update field: {key}"))
                })?;
                let value = patch.get(key).ok_or_else(|| {
                    ModelError::Validation(format!("missing update value: {key}"))
                })?;
                if value.is_null() {
                    assignments.push(format!("{column} = NULL"));
                } else {
                    params.push(DbValue::from_json_for_column(spec, value)?);
                    assignments.push(format!(
                        "{column} = {}",
                        placeholder_for_field::<T>(key, params.len())?
                    ));
                }
            }
            let where_start = params.len() + 1;
            let (where_sql, predicate_params) = build_where::<T>(&self.plan, "t0", where_start)?;
            params.extend(predicate_params);
            let table = ident(T::table_name())?;
            let version_assignment = T::version_column()
                .map(|version| {
                    ident(version).map(|version| format!(", {version} = t0.{version} + 1"))
                })
                .transpose()?
                .unwrap_or_default();
            let sql = format!(
                "UPDATE {table} AS t0 SET {}{version_assignment}, \"updated_at\" = now() WHERE {where_sql}",
                assignments.join(", ")
            );
            let affected = run_with_timeout(timeout, self.executor.execute(sql, params)).await?;
            if affected == 0 {
                return Err(ModelError::Conflict(format!(
                    "update_or_create conflict row is outside the current QuerySet visibility: {field}"
                )));
            }
        }

        self.get().await.map(|model| (model, false))
    }

    pub async fn update(mut self, values: Map<String, Value>) -> Result<u64, ModelError> {
        self.guard_bulk_mutation()?;
        let keys = validate_update_values::<T>(&values)?;
        let schema = effective_schema::<T>();
        let mut params = Vec::with_capacity(keys.len());
        let mut assignments = Vec::with_capacity(keys.len());
        for key in &keys {
            let column = ident(key)?;
            let spec = schema
                .get(key)
                .ok_or_else(|| ModelError::InvalidQuery(format!("unknown update field: {key}")))?;
            let value = values
                .get(key)
                .ok_or_else(|| ModelError::Validation(format!("missing update value: {key}")))?;
            if value.is_null() {
                assignments.push(format!("{column} = NULL"));
            } else {
                params.push(DbValue::from_json_for_column(spec, value)?);
                assignments.push(format!(
                    "{column} = {}",
                    placeholder_for_field::<T>(key, params.len())?
                ));
            }
        }
        let where_start = params.len() + 1;
        let (where_sql, predicate_params) = build_where::<T>(&self.plan, "t0", where_start)?;
        params.extend(predicate_params);
        let table = ident(T::table_name())?;
        let version_assignment = T::version_column()
            .map(|version| ident(version).map(|version| format!(", {version} = t0.{version} + 1")))
            .transpose()?
            .unwrap_or_default();
        let sql = format!(
            "UPDATE {table} AS t0 SET {}{version_assignment}, \"updated_at\" = now() WHERE {where_sql}",
            assignments.join(", ")
        );
        let timeout = self.plan.timeout;
        run_with_timeout(timeout, self.executor.execute(sql, params)).await
    }

    /// Perform race-safe column updates using typed database expressions (Django F-expression
    /// equivalent) without first loading rows into application memory.
    pub async fn update_expressions(
        mut self,
        expressions: &[UpdateExpression],
    ) -> Result<u64, ModelError> {
        self.guard_bulk_mutation()?;
        if expressions.is_empty() || expressions.len() > 100 {
            return Err(ModelError::InvalidQuery(
                "update_expressions() requires 1..=100 expressions".into(),
            ));
        }

        let schema = T::schema();
        let mut targets = BTreeSet::new();
        let mut assignments = Vec::with_capacity(expressions.len());
        let mut params = Vec::new();
        for expression in expressions {
            let target = match expression {
                UpdateExpression::Set { field, .. }
                | UpdateExpression::Copy { field, .. }
                | UpdateExpression::Arithmetic { field, .. } => field,
            };
            let target_spec = validate_expression_update_field::<T>(&schema, target)?;
            if !targets.insert(target.clone()) {
                return Err(ModelError::InvalidQuery(format!(
                    "duplicate update expression target: {target}"
                )));
            }
            let target_sql = ident(target)?;

            match expression {
                UpdateExpression::Set { value, .. } => {
                    if !db_value_matches_sql_type(target_spec, value) {
                        return Err(ModelError::InvalidQuery(format!(
                            "update expression value is incompatible with {target} ({})",
                            target_spec.sql_type
                        )));
                    }
                    params.push(value.clone());
                    assignments.push(format!("{target_sql} = ${}", params.len()));
                }
                UpdateExpression::Copy { source, .. } => {
                    let source_spec = schema.get(source).ok_or_else(|| {
                        ModelError::InvalidQuery(format!("unknown source field: {source}"))
                    })?;
                    if !sql_types_compatible(&target_spec.sql_type, &source_spec.sql_type) {
                        return Err(ModelError::InvalidQuery(format!(
                            "cannot copy incompatible field {source} ({}) into {target} ({})",
                            source_spec.sql_type, target_spec.sql_type
                        )));
                    }
                    assignments.push(format!("{target_sql} = {}", ident(source)?));
                }
                UpdateExpression::Arithmetic {
                    operator, operand, ..
                } => {
                    if self.executor.dialect().is_sqlite() && is_exact_decimal(target_spec) {
                        reject_sqlite_decimal_field::<T>(target, "arithmetic")?;
                    }
                    if !is_numeric_sql_type(&target_spec.sql_type) {
                        return Err(ModelError::InvalidQuery(format!(
                            "arithmetic update requires numeric field: {target}"
                        )));
                    }
                    if !db_value_matches_sql_type(target_spec, operand) {
                        return Err(ModelError::InvalidQuery(format!(
                            "arithmetic operand is incompatible with {target} ({})",
                            target_spec.sql_type
                        )));
                    }
                    if *operator == ArithmeticOperator::Divide && db_value_is_zero(operand) {
                        return Err(ModelError::InvalidQuery(
                            "division by zero is not allowed in update expressions".into(),
                        ));
                    }
                    params.push(operand.clone());
                    assignments.push(format!(
                        "{target_sql} = {target_sql} {} ${}",
                        operator.as_sql(),
                        params.len()
                    ));
                }
            }
        }

        let where_start = params.len() + 1;
        let (where_sql, predicate_params) = build_where::<T>(&self.plan, "t0", where_start)?;
        params.extend(predicate_params);
        let table = ident(T::table_name())?;
        let version_assignment = T::version_column()
            .map(|version| ident(version).map(|version| format!(", {version} = t0.{version} + 1")))
            .transpose()?
            .unwrap_or_default();
        let sql = format!(
            "UPDATE {table} AS t0 SET {}{version_assignment}, \"updated_at\" = now() WHERE {where_sql}",
            assignments.join(", ")
        );
        let timeout = self.plan.timeout;
        run_with_timeout(timeout, self.executor.execute(sql, params)).await
    }

    pub async fn update_versioned(
        mut self,
        expected_version: i64,
        values: Map<String, Value>,
    ) -> Result<u64, ModelError> {
        self.guard_bulk_mutation()?;
        let version_name = T::version_column().ok_or_else(|| {
            ModelError::InvalidQuery("model does not declare a version column".into())
        })?;
        let keys = validate_update_values::<T>(&values)?;
        let version_predicate = Predicate::Comparison {
            field: version_name.to_owned(),
            operator: ComparisonOperator::Eq,
            value: DbValue::I64(expected_version),
        };
        self.plan.predicate = Some(match self.plan.predicate.take() {
            Some(existing) => existing.and(version_predicate),
            None => version_predicate,
        });
        let schema = effective_schema::<T>();
        let mut params = Vec::with_capacity(keys.len());
        let mut assignments = Vec::with_capacity(keys.len());
        for key in &keys {
            let column = ident(key)?;
            let spec = schema
                .get(key)
                .ok_or_else(|| ModelError::InvalidQuery(format!("unknown update field: {key}")))?;
            let value = values
                .get(key)
                .ok_or_else(|| ModelError::Validation(format!("missing update value: {key}")))?;
            if value.is_null() {
                assignments.push(format!("{column} = NULL"));
            } else {
                params.push(DbValue::from_json_for_column(spec, value)?);
                assignments.push(format!(
                    "{column} = {}",
                    placeholder_for_field::<T>(key, params.len())?
                ));
            }
        }
        let where_start = params.len() + 1;
        let (where_sql, predicate_params) = build_where::<T>(&self.plan, "t0", where_start)?;
        params.extend(predicate_params);
        let table = ident(T::table_name())?;
        let version = ident(version_name)?;
        let sql = format!(
            "UPDATE {table} AS t0 SET {}, {version} = t0.{version} + 1, \"updated_at\" = now() WHERE {where_sql}",
            assignments.join(", ")
        );
        let timeout = self.plan.timeout;
        let affected = run_with_timeout(timeout, self.executor.execute(sql, params)).await?;
        if affected == 0 {
            return Err(ModelError::ConcurrentModification);
        }
        Ok(affected)
    }

    pub async fn increment<V: Into<DbValue>>(
        mut self,
        field: &str,
        delta: V,
    ) -> Result<u64, ModelError> {
        self.guard_bulk_mutation()?;
        let schema = T::schema();
        let spec = schema
            .get(field)
            .ok_or_else(|| ModelError::InvalidQuery(format!("unknown numeric field: {field}")))?;
        if !matches!(
            spec.sql_type.trim().to_ascii_uppercase().as_str(),
            "SMALLINT"
                | "INTEGER"
                | "BIGINT"
                | "BIGSERIAL"
                | "REAL"
                | "DOUBLE PRECISION"
                | "NUMERIC"
        ) {
            return Err(ModelError::InvalidQuery(format!(
                "field {field} is not numeric"
            )));
        }
        if self.executor.dialect().is_sqlite() {
            reject_sqlite_decimal_field::<T>(field, "arithmetic")?;
        }
        let delta = delta.into();
        if !db_value_matches_sql_type(spec, &delta) {
            return Err(ModelError::InvalidQuery(format!(
                "increment delta type is incompatible with {field} ({})",
                spec.sql_type
            )));
        }
        let (where_sql, predicate_params) = build_where::<T>(&self.plan, "t0", 2)?;
        let table = ident(T::table_name())?;
        let column = ident(field)?;
        let sql = format!(
            "UPDATE {table} AS t0 SET {column} = {column} + $1, \"updated_at\" = now() WHERE {where_sql}"
        );
        let mut params = vec![delta];
        params.extend(predicate_params);
        let timeout = self.plan.timeout;
        run_with_timeout(timeout, self.executor.execute(sql, params)).await
    }

    pub async fn soft_delete(mut self) -> Result<u64, ModelError> {
        self.guard_bulk_mutation()?;
        let deleted_at = T::soft_delete_column().ok_or_else(|| {
            ModelError::InvalidQuery("model does not declare soft-delete metadata".into())
        })?;
        let (where_sql, params) = build_where::<T>(&self.plan, "t0", 1)?;
        let table = ident(T::table_name())?;
        let deleted_at = ident(deleted_at)?;
        let sql = format!(
            "UPDATE {table} AS t0 SET {deleted_at} = now(), \"updated_at\" = now() WHERE {where_sql}"
        );
        let timeout = self.plan.timeout;
        run_with_timeout(timeout, self.executor.execute(sql, params)).await
    }

    pub async fn restore(mut self) -> Result<u64, ModelError> {
        self.guard_bulk_mutation()?;
        let deleted_at_name = T::soft_delete_column().ok_or_else(|| {
            ModelError::InvalidQuery("model does not declare soft-delete metadata".into())
        })?;
        self.plan.include_deleted = true;
        self.plan.only_deleted = false;
        let (mut where_sql, params) = build_where::<T>(&self.plan, "t0", 1)?;
        let deleted_at = qualified_ident("t0", deleted_at_name)?;
        where_sql = format!("({where_sql}) AND {deleted_at} IS NOT NULL");
        let table = ident(T::table_name())?;
        let deleted_at = ident(deleted_at_name)?;
        let sql = format!(
            "UPDATE {table} AS t0 SET {deleted_at} = NULL, \"updated_at\" = now() WHERE {where_sql}"
        );
        let timeout = self.plan.timeout;
        run_with_timeout(timeout, self.executor.execute(sql, params)).await
    }

    pub async fn hard_delete(mut self) -> Result<u64, ModelError> {
        self.guard_bulk_mutation()?;
        let (where_sql, params) = build_where::<T>(&self.plan, "t0", 1)?;
        let table = ident(T::table_name())?;
        let sql = format!("DELETE FROM {table} AS t0 WHERE {where_sql}");
        let timeout = self.plan.timeout;
        run_with_timeout(timeout, self.executor.execute(sql, params)).await
    }
}
