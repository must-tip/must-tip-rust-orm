use super::*;
use std::collections::HashMap;

struct AllowGlobal;
impl GlobalScopeAuthorization for AllowGlobal {
    fn authorize_global_scope(&self, _model: &'static str) -> Result<(), ModelError> {
        Ok(())
    }
}

struct DenyGlobal;
impl GlobalScopeAuthorization for DenyGlobal {
    fn authorize_global_scope(&self, model: &'static str) -> Result<(), ModelError> {
        Err(ModelError::PermissionDenied(format!(
            "global scope denied for {model}"
        )))
    }
}

#[derive(Debug)]
struct Demo;

impl Entity for Demo {
    fn table_name() -> &'static str {
        "demo"
    }
    fn schema() -> HashMap<String, ColumnSpec> {
        HashMap::from([
            (
                "id".into(),
                ColumnSpec {
                    sql_type: "UUID".into(),
                    nullable: false,
                    unique: false,
                    primary_key: true,
                    default: None,
                    encrypted: false,
                },
            ),
            (
                "tenant_id".into(),
                ColumnSpec {
                    sql_type: "UUID".into(),
                    nullable: false,
                    unique: false,
                    primary_key: false,
                    default: None,
                    encrypted: false,
                },
            ),
            (
                "name".into(),
                ColumnSpec {
                    sql_type: "TEXT".into(),
                    nullable: false,
                    unique: false,
                    primary_key: false,
                    default: None,
                    encrypted: false,
                },
            ),
            (
                "age".into(),
                ColumnSpec {
                    sql_type: "INTEGER".into(),
                    nullable: false,
                    unique: false,
                    primary_key: false,
                    default: None,
                    encrypted: false,
                },
            ),
            (
                "metadata".into(),
                ColumnSpec {
                    sql_type: "JSONB".into(),
                    nullable: false,
                    unique: false,
                    primary_key: false,
                    default: Some("'{}'::jsonb".into()),
                    encrypted: false,
                },
            ),
            (
                "deleted_at".into(),
                ColumnSpec {
                    sql_type: "TIMESTAMPTZ".into(),
                    nullable: true,
                    unique: false,
                    primary_key: false,
                    default: None,
                    encrypted: false,
                },
            ),
        ])
    }
    fn to_map(&self) -> HashMap<String, Value> {
        HashMap::new()
    }
}

#[test]
fn queryset_fails_closed_without_tenant_scope() {
    let plan = QueryPlan::<Demo>::default();
    assert!(matches!(
        build_where::<Demo>(&plan, "t0", 1),
        Err(ModelError::TenantScopeRequired { .. })
    ));
}

#[test]
fn field_wrappers_convert_to_native_query_values() {
    assert_eq!(DbValue::from(BooleanField(true)), DbValue::Bool(true));
    assert_eq!(DbValue::from(IntegerField(7)), DbValue::I32(7));
    assert_eq!(DbValue::from(FloatField(2.5)), DbValue::F64(2.5));
    assert_eq!(
        DbValue::from(TextField("x".into())),
        DbValue::String("x".into())
    );
    assert_eq!(
        DbValue::from(SlugField("x-y".into())),
        DbValue::String("x-y".into())
    );
}

#[test]
fn typed_predicates_are_parameterized_and_escape_like_wildcards() {
    let mut plan = QueryPlan::<Demo> {
        global_scope: true,
        ..QueryPlan::default()
    };
    plan.predicate = Some(Field::<String>::new("name").icontains("a%b_c"));
    plan.user_predicate = true;
    let (sql, params) = build_select_sql::<Demo>(&plan, 1)
        .unwrap_or_else(|error| panic!("query build failed: {error}"));
    assert!(sql.contains("ILIKE $1 ESCAPE E'\\\\'"));
    assert!(sql.contains("\"deleted_at\" IS NULL"));
    assert_eq!(params, vec![DbValue::String("%a\\%b\\_c%".into())]);
    assert!(!sql.contains("a%b_c"));
}

#[test]
fn field_comparisons_are_validated_without_bound_values() {
    let predicate =
        Field::<i32>::new("age").compare_field(ComparisonOperator::Gte, Field::<i32>::new("age"));
    let (sql, params) = compile_predicate::<Demo>(&predicate, "t0", 1)
        .unwrap_or_else(|error| panic!("compile failed: {error}"));
    assert_eq!(sql, "t0.\"age\" >= t0.\"age\"");
    assert!(params.is_empty());
    assert!(
        compile_predicate::<Demo>(
            &Field::<i32>::new("age")
                .compare_field(ComparisonOperator::Eq, Field::<String>::new("name"),),
            "t0",
            1,
        )
        .is_err()
    );
}

#[test]
fn generic_updates_reject_tenant_framework_and_type_violations() {
    let tenant_change =
        Map::from_iter([("tenant_id".to_owned(), serde_json::json!(Uuid::new_v4()))]);
    assert!(validate_update_values::<Demo>(&tenant_change).is_err());

    let timestamp_change =
        Map::from_iter([("updated_at".to_owned(), serde_json::json!(Utc::now()))]);
    assert!(validate_update_values::<Demo>(&timestamp_change).is_err());

    let wrong_type =
        Map::from_iter([("age".to_owned(), Value::String("not-an-integer".to_owned()))]);
    assert!(validate_update_values::<Demo>(&wrong_type).is_err());

    let valid = Map::from_iter([("age".to_owned(), serde_json::json!(42))]);
    assert_eq!(
        validate_update_values::<Demo>(&valid)
            .unwrap_or_else(|error| panic!("valid update: {error}")),
        vec!["age".to_owned()]
    );
}

#[test]
fn dynamic_filters_parse_database_types() {
    let predicate = dynamic_predicate::<Demo>("age", "gte", "18")
        .unwrap_or_else(|error| panic!("filter failed: {error}"));
    assert_eq!(
        predicate,
        Predicate::Comparison {
            field: "age".into(),
            operator: ComparisonOperator::Gte,
            value: DbValue::I32(18),
        }
    );
    assert!(dynamic_predicate::<Demo>("age", "icontains", "1").is_err());
    assert!(dynamic_predicate::<Demo>("unknown", "exact", "1").is_err());
}

#[test]
fn textual_postgres_types_are_validated_and_explicitly_cast() {
    struct TypedDemo;
    impl Entity for TypedDemo {
        fn table_name() -> &'static str {
            "typed_demo"
        }
        fn schema() -> HashMap<String, ColumnSpec> {
            HashMap::from([
                (
                    "id".into(),
                    ColumnSpec {
                        sql_type: "BIGINT".into(),
                        nullable: false,
                        unique: false,
                        primary_key: true,
                        default: None,
                        encrypted: false,
                    },
                ),
                (
                    "event_date".into(),
                    ColumnSpec {
                        sql_type: "DATE".into(),
                        nullable: false,
                        unique: false,
                        primary_key: false,
                        default: None,
                        encrypted: false,
                    },
                ),
                (
                    "network".into(),
                    ColumnSpec {
                        sql_type: "CIDR".into(),
                        nullable: false,
                        unique: false,
                        primary_key: false,
                        default: None,
                        encrypted: false,
                    },
                ),
            ])
        }
        fn to_map(&self) -> HashMap<String, Value> {
            HashMap::new()
        }
    }

    let date = dynamic_predicate::<TypedDemo>("event_date", "exact", "2026-08-13")
        .unwrap_or_else(|error| panic!("date lookup: {error}"));
    let (date_sql, date_params) = compile_predicate::<TypedDemo>(&date, "t0", 1)
        .unwrap_or_else(|error| panic!("date compile: {error}"));
    assert_eq!(date_sql, "t0.\"event_date\" = $1::DATE");
    assert_eq!(date_params, vec![DbValue::String("2026-08-13".into())]);
    assert!(dynamic_predicate::<TypedDemo>("event_date", "exact", "2026-02-30").is_err());

    let cidr = dynamic_predicate::<TypedDemo>("network", "exact", "10.0.0.0/24")
        .unwrap_or_else(|error| panic!("cidr lookup: {error}"));
    let (cidr_sql, _) = compile_predicate::<TypedDemo>(&cidr, "t0", 1)
        .unwrap_or_else(|error| panic!("cidr compile: {error}"));
    assert_eq!(cidr_sql, "t0.\"network\" = $1::CIDR");
    assert!(dynamic_predicate::<TypedDemo>("network", "exact", "10.0.0.0/99").is_err());
}

#[test]
fn date_and_json_lookups_are_typed_and_parameterized() {
    let month = dynamic_predicate::<Demo>("created_at", "month", "8")
        .unwrap_or_else(|error| panic!("month lookup: {error}"));
    let (sql, params) = compile_predicate::<Demo>(&month, "t0", 1)
        .unwrap_or_else(|error| panic!("month compile: {error}"));
    assert_eq!(sql, "EXTRACT(MONTH FROM t0.\"created_at\") = $1");
    assert_eq!(params, vec![DbValue::I32(8)]);
    assert!(dynamic_predicate::<Demo>("created_at", "month", "13").is_err());

    let json = dynamic_predicate::<Demo>("metadata", "jsoncontains", r#"{"active":true}"#)
        .unwrap_or_else(|error| panic!("JSON lookup: {error}"));
    let (sql, params) = compile_predicate::<Demo>(&json, "t0", 1)
        .unwrap_or_else(|error| panic!("JSON compile: {error}"));
    assert_eq!(sql, "t0.\"metadata\" @> $1::jsonb");
    assert_eq!(
        params,
        vec![DbValue::Json(serde_json::json!({"active": true}))]
    );
    assert!(dynamic_predicate::<Demo>("name", "jsoncontains", "{}").is_err());
}

#[test]
fn none_is_empty_without_requiring_tenant_scope() {
    let plan = QueryPlan::<Demo> {
        force_empty: true,
        ..QueryPlan::default()
    };
    let (sql, params) = build_where::<Demo>(&plan, "t0", 1)
        .unwrap_or_else(|error| panic!("none predicate: {error}"));
    assert_eq!(sql, "FALSE");
    assert!(params.is_empty());
}

#[test]
fn clear_ordering_suppresses_model_default_ordering() {
    struct OrderedDemo;
    impl Entity for OrderedDemo {
        fn table_name() -> &'static str {
            "ordered_demo"
        }
        fn schema() -> HashMap<String, ColumnSpec> {
            HashMap::from([
                (
                    "id".into(),
                    ColumnSpec {
                        sql_type: "BIGINT".into(),
                        nullable: false,
                        unique: false,
                        primary_key: true,
                        default: None,
                        encrypted: false,
                    },
                ),
                (
                    "name".into(),
                    ColumnSpec {
                        sql_type: "TEXT".into(),
                        nullable: false,
                        unique: false,
                        primary_key: false,
                        default: None,
                        encrypted: false,
                    },
                ),
            ])
        }
        fn default_ordering() -> Vec<&'static str> {
            vec!["name"]
        }
        fn to_map(&self) -> HashMap<String, Value> {
            HashMap::new()
        }
    }

    let mut plan = QueryPlan::<OrderedDemo>::default();
    assert!(
        build_ordering::<OrderedDemo>(&plan, "t0")
            .unwrap_or_else(|error| panic!("default ordering: {error}"))
            .contains("name")
    );
    plan.ordering.clear();
    plan.use_default_ordering = false;
    assert_eq!(
        build_ordering::<OrderedDemo>(&plan, "t0")
            .unwrap_or_else(|error| panic!("cleared ordering: {error}")),
        ""
    );
}

#[test]
fn expression_updates_reject_framework_fields_and_zero_division() {
    let schema = Demo::schema();
    assert!(validate_expression_update_field::<Demo>(&schema, "tenant_id").is_err());
    assert!(validate_expression_update_field::<Demo>(&schema, "age").is_ok());
    assert!(db_value_is_zero(&DbValue::I32(0)));
    assert!(!db_value_is_zero(&DbValue::I32(1)));
}

#[test]
fn empty_in_predicates_are_deterministic() {
    let (sql, params) = compile_predicate::<Demo>(
        &Predicate::In {
            field: "id".into(),
            values: vec![],
            negated: false,
        },
        "t0",
        1,
    )
    .unwrap_or_else(|error| panic!("compile failed: {error}"));
    assert_eq!(sql, "FALSE");
    assert!(params.is_empty());
}

#[tokio::test]
async fn mutation_requires_explicit_scope() {
    let pool = PgPool::connect_lazy("postgresql://localhost/test")
        .unwrap_or_else(|error| panic!("lazy pool failed: {error}"));
    let queryset = QuerySet::<Demo, _>::from_executor(PoolExecutor::new(pool))
        .global_scope(&AllowGlobal)
        .unwrap_or_else(|error| panic!("global scope: {error}"));
    assert!(matches!(
        queryset.guard_bulk_mutation(),
        Err(ModelError::UnsafeMutation(_))
    ));
}

#[tokio::test]
async fn global_scope_requires_explicit_authorization() {
    let pool = PgPool::connect_lazy("postgresql://localhost/test")
        .unwrap_or_else(|error| panic!("lazy pool failed: {error}"));
    let result =
        QuerySet::<Demo, _>::from_executor(PoolExecutor::new(pool)).global_scope(&DenyGlobal);
    assert!(matches!(result, Err(ModelError::PermissionDenied(_))));
}

#[tokio::test]
async fn related_global_scope_requires_independent_authorization() {
    #[derive(Debug)]
    struct RelatedDemo;
    impl Entity for RelatedDemo {
        fn table_name() -> &'static str {
            "related_demo"
        }
        fn tenant_column() -> Option<&'static str> {
            Some("tenant_id")
        }
        fn schema() -> HashMap<String, ColumnSpec> {
            HashMap::from([
                (
                    "id".into(),
                    ColumnSpec {
                        sql_type: "UUID".into(),
                        nullable: false,
                        unique: false,
                        primary_key: true,
                        default: None,
                        encrypted: false,
                    },
                ),
                (
                    "tenant_id".into(),
                    ColumnSpec {
                        sql_type: "UUID".into(),
                        nullable: false,
                        unique: false,
                        primary_key: false,
                        default: None,
                        encrypted: false,
                    },
                ),
            ])
        }
        fn to_map(&self) -> HashMap<String, Value> {
            HashMap::new()
        }
    }

    let pool = PgPool::connect_lazy("postgresql://localhost/test")
        .unwrap_or_else(|error| panic!("lazy pool failed: {error}"));
    let queryset = QuerySet::<Demo, _>::from_executor(PoolExecutor::new(pool))
        .global_scope(&AllowGlobal)
        .unwrap_or_else(|error| panic!("global scope: {error}"));
    assert!(
        !queryset
            .plan
            .authorized_global_models
            .contains(RelatedDemo::table_name())
    );
    let queryset = queryset
        .authorize_related_global::<RelatedDemo, _>(&AllowGlobal)
        .unwrap_or_else(|error| panic!("related global scope: {error}"));
    assert!(
        queryset
            .plan
            .authorized_global_models
            .contains(RelatedDemo::table_name())
    );
}

#[test]
fn first_and_last_have_deterministic_effective_ordering() {
    let mut first = QueryPlan::<Demo>::default();
    ensure_deterministic_first_ordering::<Demo>(&mut first);
    assert_eq!(first.ordering, vec![OrderBy::asc("id")]);

    let mut last = QueryPlan::<Demo>::default();
    reverse_effective_ordering::<Demo>(&mut last);
    assert_eq!(last.ordering, vec![OrderBy::desc("id")]);

    let mut explicit_first = QueryPlan::<Demo> {
        ordering: vec![OrderBy::asc("name").nulls_last()],
        ..QueryPlan::default()
    };
    ensure_deterministic_first_ordering::<Demo>(&mut explicit_first);
    assert_eq!(
        explicit_first.ordering,
        vec![OrderBy::asc("name").nulls_last(), OrderBy::asc("id")]
    );

    let mut explicit_last = QueryPlan::<Demo> {
        ordering: vec![OrderBy::asc("name").nulls_last()],
        ..QueryPlan::default()
    };
    reverse_effective_ordering::<Demo>(&mut explicit_last);
    assert_eq!(
        explicit_last.ordering,
        vec![OrderBy::desc("name").nulls_first(), OrderBy::desc("id")]
    );
}

#[test]
fn singular_relations_require_unique_targets_for_the_effective_scope() {
    struct TenantUniqueRelated;
    impl Entity for TenantUniqueRelated {
        fn table_name() -> &'static str {
            "tenant_unique_related"
        }
        fn schema() -> HashMap<String, ColumnSpec> {
            HashMap::from([
                (
                    "tenant_id".into(),
                    ColumnSpec {
                        sql_type: "UUID".into(),
                        nullable: false,
                        unique: false,
                        primary_key: false,
                        default: None,
                        encrypted: false,
                    },
                ),
                (
                    "name".into(),
                    ColumnSpec {
                        sql_type: "TEXT".into(),
                        nullable: false,
                        unique: false,
                        primary_key: false,
                        default: None,
                        encrypted: false,
                    },
                ),
            ])
        }
        fn to_map(&self) -> HashMap<String, Value> {
            HashMap::new()
        }
        fn unique_constraints() -> Vec<crate::UniqueConstraintSpec> {
            vec![crate::UniqueConstraintSpec {
                name: "uq_tenant_unique_related_tenant_name",
                columns: &["tenant_id", "name"],
            }]
        }
    }

    let relation = RelationSpec {
        field_name: "related",
        target_table: TenantUniqueRelated::table_name(),
        local_column: "name",
        remote_column: "name",
        cardinality: RelationCardinality::One,
        nullable: true,
        related_name: None,
        on_delete: crate::OnDelete::Restrict,
    };
    validate_relation::<Demo, TenantUniqueRelated>(&relation)
        .unwrap_or_else(|error| panic!("relation shape: {error}"));
    assert!(validate_singular_relation_uniqueness::<TenantUniqueRelated>(&relation, true).is_ok());
    assert!(
        validate_singular_relation_uniqueness::<TenantUniqueRelated>(&relation, false).is_err()
    );
}

#[test]
fn extending_or_group_appends_each_predicate_once() {
    let predicate = Field::<i64>::new("id")
        .eq(1_i64)
        .or(Field::<i64>::new("id").eq(2_i64))
        .or(Field::<i64>::new("id").eq(3_i64));

    match predicate {
        Predicate::Or(values) => assert_eq!(values.len(), 3),
        other => panic!("expected OR predicate, got {other:?}"),
    }
}

#[test]
fn joined_row_lock_targets_only_parent_alias() {
    let plan = QueryPlan::<Demo> {
        lock: Some((RowLock::Update, LockWait::NoWait)),
        ..QueryPlan::default()
    };
    assert_eq!(
        build_lock_for_alias(&plan, "t0").unwrap_or_else(|error| panic!("lock: {error}")),
        " FOR UPDATE OF t0 NOWAIT"
    );
    assert!(build_lock_for_alias(&plan, "external_alias").is_err());
}

#[tokio::test]
async fn pool_queries_reject_select_for_update() {
    let pool = PgPool::connect_lazy("postgresql://localhost/test")
        .unwrap_or_else(|error| panic!("lazy pool failed: {error}"));
    let queryset = QuerySet::<Demo, _>::from_executor(PoolExecutor::new(pool))
        .global_scope(&AllowGlobal)
        .unwrap_or_else(|error| panic!("global scope: {error}"))
        .select_for_update(RowLock::Update, LockWait::NoWait);
    assert!(matches!(
        queryset.ensure_lock_context(),
        Err(ModelError::TransactionRequired)
    ));
}
