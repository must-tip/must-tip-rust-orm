//! Public query builders and read terminals.

use super::*;

/// A lazy model query with explicit tenant, deletion, ordering and executor state.
///
/// Builders do no database IO. Terminal methods consume the query and execute through `E`.
/// Tenant-aware entities reject terminal operations without authorized scope. Values are bound,
/// and model identifiers are validated; selecting global scope requires a separate authorization.
pub struct QuerySet<T, E> {
    pub(super) executor: E,
    pub(super) plan: QueryPlan<T>,
}

impl<T, E> QuerySet<T, E>
where
    T: Entity,
    E: QueryExecutor,
{
    /// Start a query on an explicit pool or transaction executor.
    pub fn from_executor(executor: E) -> Self {
        Self {
            executor,
            plan: QueryPlan::default(),
        }
    }

    /// Return an explicitly empty QuerySet while preserving scope/ordering composition.
    pub fn none(mut self) -> Self {
        self.plan.force_empty = true;
        self
    }

    /// Add a predicate, combined with previous filters using AND. Tenant scope stays mandatory.
    pub fn filter(mut self, predicate: Predicate) -> Self {
        self.plan.user_predicate = true;
        self.plan.predicate = Some(match self.plan.predicate.take() {
            Some(existing) => existing.and(predicate),
            None => predicate,
        });
        self
    }

    /// Add the negation of a predicate using AND with previous conditions.
    pub fn exclude(mut self, predicate: Predicate) -> Self {
        self.plan.user_predicate = true;
        let predicate = predicate.negate();
        self.plan.predicate = Some(match self.plan.predicate.take() {
            Some(existing) => existing.and(predicate),
            None => predicate,
        });
        self
    }

    /// Append an ordering field; the first explicit order replaces model default ordering.
    pub fn order_by(mut self, order: OrderBy) -> Self {
        self.plan.use_default_ordering = false;
        self.plan.ordering.push(order);
        self
    }

    /// Remove both explicit and model default ordering.
    pub fn clear_ordering(mut self) -> Self {
        self.plan.ordering.clear();
        self.plan.use_default_ordering = false;
        self
    }

    /// Limit row-returning terminals. Count and existence checks do not apply this slice.
    pub fn limit(mut self, limit: u32) -> Self {
        self.plan.limit = Some(limit);
        self
    }

    /// Skip rows for row-returning terminals. Count and existence checks ignore this offset.
    pub fn offset(mut self, offset: u64) -> Self {
        self.plan.offset = offset;
        self
    }

    /// Select a one-based page. Page and size must be positive; size is at most 10,000.
    pub fn page(mut self, page: u32, page_size: u32) -> Result<Self, ModelError> {
        if page == 0 || page_size == 0 || page_size > 10_000 {
            return Err(ModelError::InvalidQuery(
                "page must be >= 1 and page_size must be 1..=10000".into(),
            ));
        }
        self.plan.limit = Some(page_size);
        self.plan.offset = u64::from(page.saturating_sub(1)) * u64::from(page_size);
        Ok(self)
    }

    /// Request distinct rows or projected values; projection/ordering compatibility is validated.
    pub fn distinct(mut self) -> Self {
        self.plan.distinct = true;
        self
    }

    /// Include soft-deleted and live rows without changing tenant authorization.
    pub fn with_deleted(mut self) -> Self {
        self.plan.include_deleted = true;
        self.plan.only_deleted = false;
        self
    }

    /// Restrict soft-delete-aware models to deleted rows, preserving tenant scope.
    pub fn only_deleted(mut self) -> Self {
        self.plan.include_deleted = true;
        self.plan.only_deleted = true;
        self
    }

    /// Bind trusted tenant context and clear any previous global-scope grant.
    /// This selects a database scope; the application must authorize the tenant first.
    pub fn for_tenant<V: Into<DbValue>>(mut self, tenant: V) -> Self {
        self.plan.tenant = Some(tenant.into());
        self.plan.global_scope = false;
        self.plan.authorized_global_models.clear();
        self
    }

    /// Explicit privileged scope for cross-tenant maintenance/admin operations.
    /// Authorization must be performed by the service layer before selecting this scope.
    pub fn global_scope<A: GlobalScopeAuthorization>(
        mut self,
        authorization: &A,
    ) -> Result<Self, ModelError> {
        authorization.authorize_global_scope(T::table_name())?;
        self.plan.global_scope = true;
        self.plan.tenant = None;
        self.plan.authorized_global_models.insert(T::table_name());
        Ok(self)
    }

    /// Authorize unscoped loading for one related tenant-aware model. This is intentionally
    /// separate from the parent model grant so a privilege for one table cannot silently widen
    /// access through select_related/prefetch_related.
    pub fn authorize_related_global<R, A>(mut self, authorization: &A) -> Result<Self, ModelError>
    where
        R: Entity,
        A: GlobalScopeAuthorization,
    {
        authorization.authorize_global_scope(R::table_name())?;
        self.plan.authorized_global_models.insert(R::table_name());
        Ok(self)
    }

    /// Set a nonzero timeout for terminal execution; timeout returns an error.
    pub fn timeout(mut self, timeout: Duration) -> Result<Self, ModelError> {
        if timeout.is_zero() {
            return Err(ModelError::InvalidQuery(
                "query timeout must be non-zero".into(),
            ));
        }
        self.plan.timeout = Some(timeout);
        Ok(self)
    }

    /// Request PostgreSQL row locking. Execution requires a transaction; SQLite rejects it.
    pub fn select_for_update(mut self, lock: RowLock, wait: LockWait) -> Self {
        self.plan.lock = Some((lock, wait));
        self
    }

    /// Opt-in required before an UPDATE/DELETE with no caller-provided predicate.
    pub fn allow_all_mutation(mut self) -> Self {
        self.plan.allow_all_mutation = true;
        self
    }

    /// Validate and compile a SELECT without IO, exposing SQL and bound parameter count.
    /// Bound values are deliberately omitted from the preview.
    pub fn preview(&self) -> Result<QueryPreview, ModelError> {
        if self.executor.dialect().is_sqlite() {
            validate_sqlite_decimal_plan(&self.plan, true)?;
        }
        let (sql, params) = build_select_sql::<T>(&self.plan, 1)?;
        Ok(QueryPreview {
            sql,
            parameter_count: params.len(),
        })
    }

    /// Fetch the selected rows, or an empty vector when no rows match.
    pub async fn all(mut self) -> Result<Vec<T>, ModelError>
    where
        T: for<'r> FromRow<'r, E::Row> + Send + Unpin,
    {
        self.ensure_lock_context()?;
        let (sql, params) = build_select_sql::<T>(&self.plan, 1)?;
        let timeout = self.plan.timeout;
        run_with_timeout(timeout, self.executor.fetch_all::<T>(sql, params)).await
    }

    /// Fetch the first row with deterministic ordering, or None if absent.
    pub async fn first(mut self) -> Result<Option<T>, ModelError>
    where
        T: for<'r> FromRow<'r, E::Row> + Send + Unpin,
    {
        ensure_deterministic_first_ordering::<T>(&mut self.plan);
        self.plan.limit = Some(1);
        let mut rows = self.all().await?;
        Ok(rows.pop())
    }

    /// Fetch the last row by reversing effective ordering, or None if absent.
    pub async fn last(mut self) -> Result<Option<T>, ModelError>
    where
        T: for<'r> FromRow<'r, E::Row> + Send + Unpin,
    {
        reverse_effective_ordering::<T>(&mut self.plan);
        self.plan.limit = Some(1);
        let mut rows = self.all().await?;
        Ok(rows.pop())
    }

    /// Fetch the first row ordered ascending by a validated model field.
    pub async fn earliest(mut self, field: &str) -> Result<Option<T>, ModelError>
    where
        T: for<'r> FromRow<'r, E::Row> + Send + Unpin,
    {
        validate_field::<T>(field)?;
        self.plan.ordering = vec![OrderBy::asc(field)];
        self.first().await
    }

    /// Fetch the first row ordered descending by a validated model field.
    pub async fn latest(mut self, field: &str) -> Result<Option<T>, ModelError>
    where
        T: for<'r> FromRow<'r, E::Row> + Send + Unpin,
    {
        validate_field::<T>(field)?;
        self.plan.ordering = vec![OrderBy::desc(field)];
        self.first().await
    }

    /// Reload a persisted model by primary key using this QuerySet's tenant, soft-delete,
    /// authorization, timeout, and transaction context.
    pub async fn refresh_from_db(mut self, model: &T) -> Result<T, ModelError>
    where
        T: for<'r> FromRow<'r, E::Row> + Send + Unpin,
    {
        let schema = effective_schema::<T>();
        let primary_key = T::primary_key();
        let spec = schema
            .get(primary_key)
            .ok_or_else(|| ModelError::MissingField(primary_key))?;
        let map = model.to_map();
        let value = map
            .get(primary_key)
            .ok_or_else(|| ModelError::MissingField(primary_key))?;
        let value = DbValue::from_json_for_column(spec, value)?;
        self.plan.user_predicate = true;
        let predicate = Predicate::Comparison {
            field: primary_key.to_owned(),
            operator: ComparisonOperator::Eq,
            value,
        };
        self.plan.predicate = Some(match self.plan.predicate.take() {
            Some(existing) => existing.and(predicate),
            None => predicate,
        });
        self.get().await
    }

    /// Require exactly one match: absence is NotFound and ambiguity is MultipleObjectsReturned.
    /// Replaces the limit with two so an existing slice cannot conceal a second match.
    pub async fn get(mut self) -> Result<T, ModelError>
    where
        T: for<'r> FromRow<'r, E::Row> + Send + Unpin,
    {
        self.plan.limit = Some(2);
        let mut rows = self.all().await?;
        match rows.len() {
            0 => Err(ModelError::NotFound),
            1 => rows.pop().ok_or(ModelError::NotFound),
            _ => Err(ModelError::MultipleObjectsReturned),
        }
    }

    /// Check for any scoped match, ignoring row ordering and slicing. Row locks are rejected.
    pub async fn exists(mut self) -> Result<bool, ModelError> {
        self.plan.ordering.clear();
        self.plan.use_default_ordering = false;
        self.ensure_lock_context()?;
        self.reject_row_lock_for_scalar_terminal("exists")?;
        let (where_sql, params) = build_where::<T>(&self.plan, "t0", 1)?;
        let table = ident(T::table_name())?;
        let sql = format!("SELECT EXISTS(SELECT 1 FROM {table} AS t0 WHERE {where_sql})");
        let timeout = self.plan.timeout;
        run_with_timeout(timeout, self.executor.fetch_bool(sql, params)).await
    }

    /// Count scoped matches without ordering or slicing. Row locks are rejected.
    pub async fn count(mut self) -> Result<i64, ModelError> {
        self.plan.ordering.clear();
        self.plan.use_default_ordering = false;
        self.ensure_lock_context()?;
        self.reject_row_lock_for_scalar_terminal("count")?;
        let (where_sql, params) = build_where::<T>(&self.plan, "t0", 1)?;
        let table = ident(T::table_name())?;
        let distinct = if self.plan.distinct {
            format!("DISTINCT t0.{}", ident(T::primary_key())?)
        } else {
            "*".to_owned()
        };
        let sql = format!("SELECT COUNT({distinct}) FROM {table} AS t0 WHERE {where_sql}");
        let timeout = self.plan.timeout;
        run_with_timeout(timeout, self.executor.fetch_i64(sql, params)).await
    }

    /// Return PostgreSQL JSON EXPLAIN output; SQLite has an unsupported result shape.
    /// With analyze=true PostgreSQL executes the query to collect runtime statistics.
    pub async fn explain(mut self, analyze: bool) -> Result<Value, ModelError>
    where
        T: for<'r> FromRow<'r, E::Row> + Send + Unpin,
    {
        if self.executor.dialect().is_sqlite() {
            return Err(ModelError::InvalidQuery(
                "SQLite EXPLAIN QUERY PLAN has a different result shape; use raw SQLite EXPLAIN tooling for now".into(),
            ));
        }
        if analyze && self.plan.lock.is_some() {
            return Err(ModelError::InvalidQuery(
                "EXPLAIN ANALYZE cannot be combined with row locking through this API".into(),
            ));
        }
        let (select, params) = build_select_sql::<T>(&self.plan, 1)?;
        let options = if analyze {
            "ANALYZE TRUE, BUFFERS TRUE, WAL TRUE, SETTINGS TRUE, FORMAT JSON"
        } else {
            "ANALYZE FALSE, SETTINGS TRUE, FORMAT JSON"
        };
        let sql = format!("EXPLAIN ({options}) {select}");
        let timeout = self.plan.timeout;
        run_with_timeout(timeout, self.executor.fetch_json(sql, params)).await
    }

    /// Return JSON objects containing only the requested persisted fields.
    pub async fn values(mut self, fields: &[&str]) -> Result<Vec<Value>, ModelError> {
        self.ensure_lock_context()?;
        self.validate_projection_shape()?;
        if self.executor.dialect().is_sqlite() && self.plan.distinct {
            for field in fields {
                reject_sqlite_decimal_field::<T>(field, "distinct projection")?;
            }
        }
        if fields.is_empty() || fields.len() > 100 {
            return Err(ModelError::InvalidQuery(
                "values() requires 1..=100 fields".into(),
            ));
        }
        let mut seen = std::collections::HashSet::new();
        let mut pairs = Vec::with_capacity(fields.len());
        for field in fields {
            validate_field::<T>(field)?;
            if !seen.insert(*field) {
                return Err(ModelError::InvalidQuery(format!(
                    "duplicate values() field: {field}"
                )));
            }
            let column = qualified_ident("t0", field)?;
            // Field names are validated SQL identifiers, so embedding as a JSON object key is safe.
            pairs.push(format!("'{field}', {column}"));
        }
        let (where_sql, params) = build_where::<T>(&self.plan, "t0", 1)?;
        let table = ident(T::table_name())?;
        let distinct = if self.plan.distinct { "DISTINCT " } else { "" };
        let ordering = build_ordering::<T>(&self.plan, "t0")?;
        let pagination = build_pagination(&self.plan)?;
        let lock = build_lock(&self.plan)?;
        let sql = format!(
            "SELECT {distinct}jsonb_build_object({}) FROM {table} AS t0 WHERE {where_sql}{ordering}{pagination}{lock}",
            pairs.join(", ")
        );
        let timeout = self.plan.timeout;
        run_with_timeout(timeout, self.executor.fetch_json_all(sql, params)).await
    }

    /// Return positional projections similar to Django values_list().
    ///
    /// `flat=true` is permitted only for a single field and returns JSON scalars. Otherwise each
    /// result is a JSON array preserving the requested field order.
    pub async fn values_list(
        mut self,
        fields: &[&str],
        flat: bool,
    ) -> Result<Vec<Value>, ModelError> {
        self.ensure_lock_context()?;
        self.validate_projection_shape()?;
        if self.executor.dialect().is_sqlite() && self.plan.distinct {
            for field in fields {
                reject_sqlite_decimal_field::<T>(field, "distinct projection")?;
            }
        }
        if fields.is_empty() || fields.len() > 100 {
            return Err(ModelError::InvalidQuery(
                "values_list() requires 1..=100 fields".into(),
            ));
        }
        if flat && fields.len() != 1 {
            return Err(ModelError::InvalidQuery(
                "flat values_list() requires exactly one field".into(),
            ));
        }
        let mut seen = std::collections::HashSet::new();
        let mut columns = Vec::with_capacity(fields.len());
        for field in fields {
            validate_field::<T>(field)?;
            if !seen.insert(*field) {
                return Err(ModelError::InvalidQuery(format!(
                    "duplicate values_list() field: {field}"
                )));
            }
            columns.push(qualified_ident("t0", field)?);
        }
        let projection = if self.executor.dialect().is_sqlite() {
            if flat {
                format!("json_quote({})", columns[0])
            } else {
                format!("json_array({})", columns.join(", "))
            }
        } else if flat {
            format!("COALESCE(to_jsonb({}), 'null'::jsonb)", columns[0])
        } else {
            format!("jsonb_build_array({})", columns.join(", "))
        };
        let (where_sql, params) = build_where::<T>(&self.plan, "t0", 1)?;
        let table = ident(T::table_name())?;
        let distinct = if self.plan.distinct { "DISTINCT " } else { "" };
        let ordering = build_ordering::<T>(&self.plan, "t0")?;
        let pagination = build_pagination(&self.plan)?;
        let lock = build_lock(&self.plan)?;
        let sql = format!(
            "SELECT {distinct}{projection} FROM {table} AS t0 WHERE {where_sql}{ordering}{pagination}{lock}"
        );
        let timeout = self.plan.timeout;
        run_with_timeout(timeout, self.executor.fetch_json_all(sql, params)).await
    }

    /// Execute a validated database aggregate and return the native PostgreSQL value as JSON.
    pub async fn aggregate(
        mut self,
        field: &str,
        function: AggregateFunction,
    ) -> Result<Value, ModelError> {
        self.plan.ordering.clear();
        self.plan.use_default_ordering = false;
        self.ensure_lock_context()?;
        self.reject_row_lock_for_scalar_terminal("aggregate")?;
        validate_aggregate::<T>(field, function)?;
        if self.executor.dialect().is_sqlite()
            && (function != AggregateFunction::Count || self.plan.distinct)
        {
            reject_sqlite_decimal_field::<T>(field, "aggregation")?;
        }
        let (where_sql, params) = build_where::<T>(&self.plan, "t0", 1)?;
        let table = ident(T::table_name())?;
        let column = qualified_ident("t0", field)?;
        let distinct = if self.plan.distinct { "DISTINCT " } else { "" };
        let sql = if self.executor.dialect().is_sqlite() {
            let function_sql = match function {
                AggregateFunction::Count => "COUNT",
                AggregateFunction::Sum => "SUM",
                AggregateFunction::Avg => "AVG",
                AggregateFunction::Min => "MIN",
                AggregateFunction::Max => "MAX",
                _ => {
                    return Err(ModelError::InvalidQuery(format!(
                        "aggregate {:?} is PostgreSQL-only; SQLite supports count/sum/avg/min/max through this API",
                        function
                    )));
                }
            };
            format!(
                "SELECT CAST(COALESCE({function_sql}({distinct}{column}), 'null') AS TEXT) FROM {table} AS t0 WHERE {where_sql}"
            )
        } else {
            format!(
                "SELECT COALESCE(to_jsonb({}({distinct}{column})), 'null'::jsonb) FROM {table} AS t0 WHERE {where_sql}",
                function.as_sql()
            )
        };
        let timeout = self.plan.timeout;
        run_with_timeout(timeout, self.executor.fetch_json(sql, params)).await
    }

    /// Insert one model using the same executor and tenant scope as this QuerySet.
    pub async fn select_related<R>(
        mut self,
        relation: &RelationSpec,
    ) -> Result<Vec<Joined<T, R>>, ModelError>
    where
        T: for<'r> FromRow<'r, E::Row> + Send + Unpin,
        R: Entity + DeserializeOwned + Send,
        for<'c> &'c str: sqlx::ColumnIndex<E::Row>,
        for<'r> Json<Value>:
            sqlx::Decode<'r, <E::Row as Row>::Database> + sqlx::Type<<E::Row as Row>::Database>,
    {
        if relation.cardinality != RelationCardinality::One {
            return Err(ModelError::InvalidQuery(
                "select_related is valid only for singular relations".into(),
            ));
        }
        validate_relation::<T, R>(relation)?;
        if self.executor.dialect().is_sqlite() {
            reject_sqlite_decimal_field::<T>(relation.local_column, "relation keys")?;
            reject_sqlite_decimal_field::<R>(relation.remote_column, "relation keys")?;
        }
        validate_singular_relation_uniqueness::<R>(relation, self.plan.tenant.is_some())?;
        self.ensure_lock_context()?;
        let (where_sql, mut params) = build_where::<T>(&self.plan, "t0", 1)?;
        let parent_table = ident(T::table_name())?;
        let target_table = ident(R::table_name())?;
        let parent_columns = select_columns::<T>(Some("t0"))?;
        let local = qualified_ident("t0", relation.local_column)?;
        let remote = qualified_ident("t1", relation.remote_column)?;
        let related_json = json_object_sql::<R>("t1", self.executor.dialect())?;
        let mut join_conditions = vec![format!("{local} = {remote}")];
        if let Some(deleted_at) = R::soft_delete_column() {
            join_conditions.push(format!("{} IS NULL", qualified_ident("t1", deleted_at)?));
        }
        if let Some(tenant_column) = R::tenant_column() {
            if let Some(tenant) = &self.plan.tenant {
                validate_bound_value::<R>(tenant_column, tenant)?;
                let parameter = params.len() + 1;
                params.push(tenant.clone());
                join_conditions.push(format!(
                    "{} = ${parameter}",
                    qualified_ident("t1", tenant_column)?
                ));
            } else if !self.plan.authorized_global_models.contains(R::table_name()) {
                return Err(ModelError::TenantScopeRequired {
                    model: R::table_name(),
                });
            }
        }
        let ordering = build_ordering::<T>(&self.plan, "t0")?;
        let pagination = build_pagination(&self.plan)?;
        let lock = build_lock_for_alias(&self.plan, "t0")?;
        let sql = format!(
            "SELECT {parent_columns}, CASE WHEN {remote} IS NULL THEN NULL ELSE {related_json} END AS \"__related\" FROM {parent_table} AS t0 LEFT JOIN {target_table} AS t1 ON {} WHERE {where_sql}{ordering}{pagination}{lock}",
            join_conditions.join(" AND ")
        );
        let timeout = self.plan.timeout;
        let rows = run_with_timeout(timeout, self.executor.fetch_rows(sql, params)).await?;
        let mut output = Vec::with_capacity(rows.len());
        for row in rows {
            let parent = T::from_row(&row)?;
            let related_json: Option<Json<Value>> = row.try_get("__related")?;
            let related = related_json
                .map(|value| hydrate_relation_json::<R>(value.0))
                .transpose()?;
            output.push(Joined { parent, related });
        }
        Ok(output)
    }

    pub async fn prefetch_related<R>(
        mut self,
        relation: &RelationSpec,
    ) -> Result<Prefetched<T, R>, ModelError>
    where
        T: for<'r> FromRow<'r, E::Row> + Send + Unpin,
        R: Entity + for<'r> FromRow<'r, E::Row> + Send + Unpin,
    {
        validate_relation::<T, R>(relation)?;
        if self.executor.dialect().is_sqlite() {
            reject_sqlite_decimal_field::<T>(relation.local_column, "relation keys")?;
            reject_sqlite_decimal_field::<R>(relation.remote_column, "relation keys")?;
        }
        self.ensure_lock_context()?;
        let (parent_sql, parent_params) = build_select_sql::<T>(&self.plan, 1)?;
        let timeout = self.plan.timeout;
        let parents = run_with_timeout(
            timeout,
            self.executor.fetch_all::<T>(parent_sql, parent_params),
        )
        .await?;

        let parent_schema = effective_schema::<T>();
        let local_spec = parent_schema.get(relation.local_column).ok_or_else(|| {
            ModelError::InvalidQuery(format!(
                "unknown relation local column: {}",
                relation.local_column
            ))
        })?;
        let mut values = Vec::new();
        for parent in &parents {
            if let Some(value) = parent.to_map().get(relation.local_column)
                && !value.is_null()
            {
                values.push(DbValue::from_json_for_column(local_spec, value)?);
            }
        }
        if values.is_empty() {
            return Ok(Prefetched {
                parents,
                related_by_key: HashMap::new(),
                local_field: relation.local_column,
            });
        }

        let related_plan = QueryPlan::<R> {
            predicate: Some(Predicate::In {
                field: relation.remote_column.to_owned(),
                values,
                negated: false,
            }),
            user_predicate: true,
            tenant: self.plan.tenant.clone(),
            global_scope: self.plan.authorized_global_models.contains(R::table_name()),
            authorized_global_models: self.plan.authorized_global_models.clone(),
            timeout: self.plan.timeout,
            ..QueryPlan::default()
        };
        let (related_sql, related_params) = build_select_sql::<R>(&related_plan, 1)?;
        let related = run_with_timeout(
            timeout,
            self.executor.fetch_all::<R>(related_sql, related_params),
        )
        .await?;

        let mut related_by_key: HashMap<String, Vec<R>> = HashMap::new();
        for item in related {
            let key = item
                .to_map()
                .get(relation.remote_column)
                .map(canonical_json_key)
                .transpose()?
                .ok_or_else(|| {
                    ModelError::Conversion(format!(
                        "related model did not emit relation key {}",
                        relation.remote_column
                    ))
                })?;
            related_by_key.entry(key).or_default().push(item);
        }
        Ok(Prefetched {
            parents,
            related_by_key,
            local_field: relation.local_column,
        })
    }

    pub(super) fn validate_write_scope(&self, map: &Map<String, Value>) -> Result<(), ModelError> {
        if self.executor.dialect().is_sqlite() {
            validate_sqlite_decimal_schema::<T>()?;
        }
        let Some(tenant_column) = T::tenant_column() else {
            return Ok(());
        };
        if self.plan.global_scope {
            return Ok(());
        }
        let expected = self
            .plan
            .tenant
            .as_ref()
            .ok_or(ModelError::TenantScopeRequired {
                model: T::table_name(),
            })?;
        let schema = effective_schema::<T>();
        let spec = schema.get(tenant_column).ok_or_else(|| {
            ModelError::SqlBuildError(format!(
                "tenant metadata references missing field: {tenant_column}"
            ))
        })?;
        let actual_json = map.get(tenant_column).ok_or_else(|| {
            ModelError::Validation(format!(
                "tenant-aware model must emit field: {tenant_column}"
            ))
        })?;
        if actual_json.is_null() {
            return Err(ModelError::Validation(format!(
                "tenant-aware write requires non-null {tenant_column}; use an authorized global_scope(&authorization) only for privileged global records"
            )));
        }
        let actual = DbValue::from_json_for_column(spec, actual_json)?;
        if &actual != expected {
            return Err(ModelError::PermissionDenied(format!(
                "write tenant does not match QuerySet tenant scope for {}",
                T::table_name()
            )));
        }
        Ok(())
    }

    pub(super) fn ensure_lock_context(&self) -> Result<(), ModelError> {
        if self.executor.dialect().is_sqlite() {
            validate_sqlite_decimal_plan(&self.plan, true)?;
        }
        if self.plan.lock.is_some() && self.executor.dialect().is_sqlite() {
            return Err(ModelError::InvalidQuery(
                "SQLite does not support SELECT ... FOR UPDATE row-lock syntax; use an explicit SQLite write transaction instead".into(),
            ));
        }
        if self.plan.lock.is_some() && !self.executor.is_transactional() {
            return Err(ModelError::TransactionRequired);
        }
        if self.plan.lock.is_some() && self.plan.distinct {
            return Err(ModelError::InvalidQuery(
                "row locking cannot be combined with DISTINCT".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn validate_projection_shape(&self) -> Result<(), ModelError> {
        let has_ordering = !self.plan.ordering.is_empty()
            || (self.plan.use_default_ordering && !T::default_ordering().is_empty());
        if self.plan.distinct && has_ordering {
            return Err(ModelError::InvalidQuery(
                "DISTINCT values()/values_list() with ordering is ambiguous in PostgreSQL; call clear_ordering() or remove distinct()".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn reject_row_lock_for_scalar_terminal(
        &self,
        operation: &str,
    ) -> Result<(), ModelError> {
        if self.plan.lock.is_some() {
            return Err(ModelError::InvalidQuery(format!(
                "{operation}() does not return lockable model rows; apply select_for_update() to a row-returning QuerySet instead"
            )));
        }
        Ok(())
    }

    pub(super) fn guard_bulk_mutation(&self) -> Result<(), ModelError> {
        if self.executor.dialect().is_sqlite() {
            validate_sqlite_decimal_plan(&self.plan, false)?;
        }
        if !self.plan.user_predicate && !self.plan.allow_all_mutation {
            return Err(ModelError::UnsafeMutation(
                "bulk mutation requires filter()/exclude() or explicit allow_all_mutation()".into(),
            ));
        }
        Ok(())
    }
}
