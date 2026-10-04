use super::*;

#[derive(Debug, Clone)]
pub struct RestQueryPolicy {
    filter_fields: BTreeMap<String, BTreeSet<Lookup>>,
    search_fields: BTreeSet<String>,
    ordering_fields: BTreeSet<String>,
    pagination: PaginationStyle,
    default_page_size: u32,
    max_page_size: u32,
    max_search_length: usize,
    max_query_params: usize,
    max_filter_value_length: usize,
    deny_unknown_params: bool,
    cursor_keys: Option<CursorKeyRing>,
}

impl Default for RestQueryPolicy {
    fn default() -> Self {
        Self {
            filter_fields: BTreeMap::new(),
            search_fields: BTreeSet::new(),
            ordering_fields: BTreeSet::new(),
            pagination: PaginationStyle::PageNumber,
            default_page_size: 50,
            max_page_size: 500,
            max_search_length: 256,
            max_query_params: 100,
            max_filter_value_length: 4_096,
            deny_unknown_params: true,
            cursor_keys: None,
        }
    }
}

impl RestQueryPolicy {
    pub fn allow_filter(
        mut self,
        field: impl Into<String>,
        lookups: impl IntoIterator<Item = Lookup>,
    ) -> Self {
        self.filter_fields
            .insert(field.into(), lookups.into_iter().collect());
        self
    }

    pub fn allow_search(mut self, field: impl Into<String>) -> Self {
        self.search_fields.insert(field.into());
        self
    }

    pub fn allow_ordering(mut self, field: impl Into<String>) -> Self {
        self.ordering_fields.insert(field.into());
        self
    }

    pub fn pagination(mut self, pagination: PaginationStyle) -> Self {
        self.pagination = pagination;
        self
    }

    pub fn page_sizes(
        mut self,
        default_page_size: u32,
        max_page_size: u32,
    ) -> Result<Self, RestError> {
        if default_page_size == 0
            || max_page_size == 0
            || default_page_size > max_page_size
            || max_page_size > 10_000
        {
            return Err(RestError::new(
                "invalid_pagination_policy",
                "page sizes must satisfy 1 <= default <= max <= 10000",
            ));
        }
        self.default_page_size = default_page_size;
        self.max_page_size = max_page_size;
        Ok(self)
    }

    pub fn request_limits(
        mut self,
        max_query_params: usize,
        max_filter_value_length: usize,
        max_search_length: usize,
    ) -> Result<Self, RestError> {
        if !(1..=1_000).contains(&max_query_params)
            || !(1..=65_536).contains(&max_filter_value_length)
            || !(1..=4_096).contains(&max_search_length)
        {
            return Err(RestError::new(
                "invalid_query_policy",
                "query limits exceed supported bounds",
            ));
        }
        self.max_query_params = max_query_params;
        self.max_filter_value_length = max_filter_value_length;
        self.max_search_length = max_search_length;
        Ok(self)
    }

    pub fn deny_unknown_params(mut self, deny: bool) -> Self {
        self.deny_unknown_params = deny;
        self
    }

    /// Configure the active HMAC-SHA256 cursor signing key. Required for cursor pagination.
    /// Supply key material from a secret manager/environment, never source control.
    pub fn cursor_signing_key(mut self, key: impl Into<Vec<u8>>) -> Result<Self, RestError> {
        self.cursor_keys = Some(CursorKeyRing::new(key.into())?);
        Ok(self)
    }

    /// Add a prior signing key accepted only for verification during key rotation.
    pub fn previous_cursor_signing_key(
        mut self,
        key: impl Into<Vec<u8>>,
    ) -> Result<Self, RestError> {
        let Some(keys) = self.cursor_keys.as_mut() else {
            return Err(RestError::new(
                "invalid_cursor_key",
                "configure the active cursor signing key before previous keys",
            ));
        };
        keys.push_previous(key.into())?;
        Ok(self)
    }

    pub fn apply<T, E>(
        &self,
        params: &HashMap<String, String>,
        mut queryset: QuerySet<T, E>,
    ) -> Result<AppliedRestQuery<T, E>, RestError>
    where
        T: Entity,
        E: QueryExecutor,
    {
        self.validate_model_policy::<T>()?;
        if params.len() > self.max_query_params {
            return Err(RestError::new(
                "query_too_complex",
                format!(
                    "at most {} query parameters are allowed",
                    self.max_query_params
                ),
            ));
        }
        const RESERVED: &[&str] = &[
            "search",
            "ordering",
            "page",
            "page_size",
            "limit",
            "offset",
            "cursor",
        ];

        for (key, raw) in params {
            if raw.len() > self.max_filter_value_length && key != "cursor" {
                return Err(RestError::new(
                    "query_value_too_large",
                    format!("query parameter {key} exceeds the configured size limit"),
                ));
            }
            if RESERVED.contains(&key.as_str()) {
                continue;
            }
            let (field, lookup_name) = key
                .split_once("__")
                .map_or((key.as_str(), "exact"), |(field, lookup)| (field, lookup));
            let Some(lookup) = Lookup::parse(lookup_name) else {
                if self.deny_unknown_params {
                    return Err(RestError::new(
                        "invalid_filter",
                        format!("unsupported lookup in query parameter {key}"),
                    ));
                }
                continue;
            };
            let Some(allowed) = self.filter_fields.get(field) else {
                if self.deny_unknown_params {
                    return Err(RestError::new(
                        "invalid_filter",
                        format!("filter field is not allowlisted: {field}"),
                    ));
                }
                continue;
            };
            if !allowed.contains(&lookup) {
                return Err(RestError::new(
                    "invalid_filter",
                    format!("lookup {} is not allowed for {field}", lookup.name()),
                ));
            }
            queryset = queryset.filter(dynamic_predicate::<T>(field, lookup.name(), raw)?);
        }

        if let Some(search) = params
            .get("search")
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            if search.chars().count() > self.max_search_length {
                return Err(RestError::new("invalid_search", "search term is too long"));
            }
            if self.search_fields.is_empty() {
                return Err(RestError::new("invalid_search", "search is not enabled"));
            }
            let mut expression: Option<Predicate> = None;
            for field in &self.search_fields {
                let predicate = dynamic_predicate::<T>(field, "icontains", search)?;
                expression = Some(match expression {
                    Some(existing) => existing.or(predicate),
                    None => predicate,
                });
            }
            if let Some(expression) = expression {
                queryset = queryset.filter(expression);
            }
        }

        if let Some(ordering) = params
            .get("ordering")
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            for raw_field in ordering.split(',') {
                let raw_field = raw_field.trim();
                let (direction, field) = raw_field
                    .strip_prefix('-')
                    .map_or((SortDirection::Asc, raw_field), |field| {
                        (SortDirection::Desc, field)
                    });
                if !self.ordering_fields.contains(field) {
                    return Err(RestError::new(
                        "invalid_ordering",
                        format!("ordering field is not allowlisted: {field}"),
                    ));
                }
                queryset = queryset.order_by(match direction {
                    SortDirection::Asc => OrderBy::asc(field),
                    SortDirection::Desc => OrderBy::desc(field),
                });
            }
        }

        let page_size = parse_bounded_u32(
            params.get("page_size"),
            self.default_page_size,
            self.max_page_size,
            "page_size",
        )?;
        let mut cursor_page = None;
        match &self.pagination {
            PaginationStyle::PageNumber => {
                reject_params(params, &["limit", "offset", "cursor"])?;
                let page = parse_bounded_u32(params.get("page"), 1, u32::MAX, "page")?;
                queryset = queryset.page(page, page_size)?;
            }
            PaginationStyle::LimitOffset => {
                reject_params(params, &["page", "cursor"])?;
                let limit =
                    parse_bounded_u32(params.get("limit"), page_size, self.max_page_size, "limit")?;
                let offset = parse_u64(params.get("offset"), 0, "offset")?;
                queryset = queryset.limit(limit).offset(offset);
            }
            PaginationStyle::Cursor {
                ordering_field,
                direction,
            } => {
                reject_params(params, &["page", "limit", "offset"])?;
                let primary_key = T::primary_key();
                if let Some(encoded) = params.get("cursor") {
                    let cursor = CursorToken::decode(encoded, self.cursor_keys.as_ref())?;
                    if cursor.model != T::table_name() {
                        return Err(RestError::new(
                            "invalid_cursor",
                            "cursor model does not match endpoint policy",
                        ));
                    }
                    if cursor.field != *ordering_field || cursor.direction != *direction {
                        return Err(RestError::new(
                            "invalid_cursor",
                            "cursor ordering does not match endpoint policy",
                        ));
                    }
                    let comparison = match direction {
                        SortDirection::Asc => ComparisonOperator::Gt,
                        SortDirection::Desc => ComparisonOperator::Lt,
                    };
                    let field_spec = effective_schema::<T>()
                        .get(ordering_field)
                        .cloned()
                        .ok_or_else(|| {
                            RestError::new("invalid_cursor", "cursor field is not a model field")
                        })?;
                    let pk_spec = effective_schema::<T>()
                        .get(primary_key)
                        .cloned()
                        .ok_or_else(|| {
                            RestError::new("invalid_cursor", "primary key is not queryable")
                        })?;
                    let field_value = DbValue::parse_for_column(&field_spec, &cursor.value)?;
                    let pk_value = DbValue::parse_for_column(&pk_spec, &cursor.primary_key)?;
                    let predicate = Predicate::Comparison {
                        field: ordering_field.clone(),
                        operator: comparison,
                        value: field_value.clone(),
                    }
                    .or(Predicate::Comparison {
                        field: ordering_field.clone(),
                        operator: ComparisonOperator::Eq,
                        value: field_value,
                    }
                    .and(Predicate::Comparison {
                        field: primary_key.to_owned(),
                        operator: comparison,
                        value: pk_value,
                    }));
                    queryset = queryset.filter(predicate);
                }
                queryset = queryset
                    .order_by(match direction {
                        SortDirection::Asc => OrderBy::asc(ordering_field),
                        SortDirection::Desc => OrderBy::desc(ordering_field),
                    })
                    .order_by(match direction {
                        SortDirection::Asc => OrderBy::asc(primary_key),
                        SortDirection::Desc => OrderBy::desc(primary_key),
                    })
                    .limit(page_size.saturating_add(1));
                let keys = self.cursor_keys.clone().ok_or_else(|| {
                    RestError::new(
                        "invalid_policy",
                        "cursor pagination requires an HMAC signing key",
                    )
                })?;
                cursor_page = Some(CursorPagePolicy {
                    field: ordering_field.clone(),
                    direction: *direction,
                    page_size,
                    keys,
                });
            }
        }

        Ok(AppliedRestQuery {
            queryset,
            cursor_page,
        })
    }

    pub fn apply_authorized<T, E, A>(
        &self,
        params: &HashMap<String, String>,
        mut queryset: QuerySet<T, E>,
        access: &A,
    ) -> Result<AppliedRestQuery<T, E>, RestError>
    where
        T: Entity,
        E: QueryExecutor,
        A: RestAccessPolicy<T> + ?Sized,
    {
        if let Some(predicate) = access.scope_predicate(RestAction::List)? {
            queryset = queryset.filter(predicate);
        }
        self.apply(params, queryset)
    }

    pub(super) fn validate_model_policy<T: Entity>(&self) -> Result<(), RestError> {
        let schema = effective_schema::<T>();
        for field in self
            .filter_fields
            .keys()
            .chain(self.search_fields.iter())
            .chain(self.ordering_fields.iter())
        {
            let spec = schema.get(field).ok_or_else(|| {
                RestError::new(
                    "invalid_policy",
                    format!("policy references unknown model field: {field}"),
                )
            })?;
            if spec.encrypted {
                return Err(RestError::new(
                    "invalid_policy",
                    format!(
                        "encrypted field cannot be exposed to REST filtering/search/ordering: {field}"
                    ),
                ));
            }
        }

        for field in &self.search_fields {
            let spec = schema.get(field).ok_or_else(|| {
                RestError::new("invalid_policy", format!("unknown search field: {field}"))
            })?;
            if !is_text_sql_type(&spec.sql_type) {
                return Err(RestError::new(
                    "invalid_policy",
                    format!("search requires a text field: {field}"),
                ));
            }
        }

        for (field, lookups) in &self.filter_fields {
            let spec = schema.get(field).ok_or_else(|| {
                RestError::new("invalid_policy", format!("unknown filter field: {field}"))
            })?;
            for lookup in lookups {
                validate_lookup_for_column(field, &spec.sql_type, *lookup)?;
            }
        }
        if let PaginationStyle::Cursor { ordering_field, .. } = &self.pagination {
            if self.cursor_keys.is_none() {
                return Err(RestError::new(
                    "invalid_policy",
                    "cursor pagination requires an HMAC signing key",
                ));
            }
            if !self.ordering_fields.contains(ordering_field) {
                return Err(RestError::new(
                    "invalid_policy",
                    "cursor ordering field must also be allowlisted for ordering",
                ));
            }
            let cursor_spec = schema.get(ordering_field).ok_or_else(|| {
                RestError::new("invalid_policy", "cursor field is not a model field")
            })?;
            if cursor_spec.nullable {
                return Err(RestError::new(
                    "invalid_policy",
                    "cursor ordering fields must be non-nullable for deterministic keyset pagination",
                ));
            }
        }
        Ok(())
    }
}
