use super::*;
use crate::{ColumnSpec, PoolExecutor};
use sqlx::PgPool;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Demo {
    id: i64,
    name: String,
    secret: String,
}

impl Entity for Demo {
    fn table_name() -> &'static str {
        "rest_demo"
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
            (
                "secret".into(),
                ColumnSpec {
                    sql_type: "TEXT".into(),
                    nullable: false,
                    unique: false,
                    primary_key: false,
                    default: None,
                    encrypted: true,
                },
            ),
        ])
    }
    fn to_map(&self) -> HashMap<String, Value> {
        HashMap::from([
            ("id".into(), serde_json::json!(self.id)),
            ("name".into(), serde_json::json!(self.name)),
            ("secret".into(), serde_json::json!(self.secret)),
        ])
    }
}

#[test]
fn serializer_rejects_read_only_unknown_and_null_fields() {
    let policy = SerializerPolicy::for_model::<Demo>();
    let error = policy
        .validate_input(
            &serde_json::json!({"id": 9, "name": null, "unknown": true, "secret": "x"}),
            SerializerMode::Create,
        )
        .expect_err("payload should fail");
    assert!(error.fields.contains_key("id"));
    assert!(error.fields.contains_key("name"));
    assert!(error.fields.contains_key("unknown"));
}

#[test]
fn serializer_omits_write_only_encrypted_fields() {
    let policy = SerializerPolicy::for_model::<Demo>();
    let serialized = policy
        .serialize(&Demo {
            id: 1,
            name: "a".into(),
            secret: "hash".into(),
        })
        .unwrap_or_else(|error| panic!("serialize: {error}"));
    assert_eq!(serialized, serde_json::json!({"id": 1, "name": "a"}));
}

#[test]
fn framework_owned_fields_are_read_only_by_default() {
    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TenantDemo {
        id: i64,
        tenant_id: i64,
        name: String,
        version: i64,
    }
    impl Entity for TenantDemo {
        fn table_name() -> &'static str {
            "tenant_rest_demo"
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
                    "tenant_id".into(),
                    ColumnSpec {
                        sql_type: "BIGINT".into(),
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
                    "version".into(),
                    ColumnSpec {
                        sql_type: "BIGINT".into(),
                        nullable: false,
                        unique: false,
                        primary_key: false,
                        default: Some("1".into()),
                        encrypted: false,
                    },
                ),
            ])
        }
        fn tenant_column() -> Option<&'static str> {
            Some("tenant_id")
        }
        fn version_column() -> Option<&'static str> {
            Some("version")
        }
        fn to_map(&self) -> HashMap<String, Value> {
            HashMap::new()
        }
    }

    let policy = SerializerPolicy::for_model::<TenantDemo>();
    let error = policy
        .validate_input(
            &serde_json::json!({"tenant_id": 999, "version": 9, "name": "safe"}),
            SerializerMode::Create,
        )
        .expect_err("framework fields must reject client writes");
    assert!(error.fields.contains_key("tenant_id"));
    assert!(error.fields.contains_key("version"));
}

#[test]
fn serializer_never_constructs_models_from_untransformed_sensitive_input() {
    let policy = SerializerPolicy::for_model::<Demo>();
    let error = policy
        .deserialize::<Demo>(
            &serde_json::json!({"name": "alice", "secret": "plaintext-secret"}),
            SerializerMode::Create,
        )
        .expect_err("write-only values require an application transform");
    assert_eq!(error.code, "sensitive_field_requires_transform");
}

#[test]
fn rest_policy_rejects_sensitive_fields() {
    let policy = RestQueryPolicy::default().allow_filter("secret", [Lookup::Exact]);
    assert!(policy.validate_model_policy::<Demo>().is_err());
}

#[test]
fn rest_lookup_policy_rejects_type_incompatible_lookups() {
    let text_on_integer = RestQueryPolicy::default().allow_filter("id", [Lookup::IContains]);
    assert!(text_on_integer.validate_model_policy::<Demo>().is_err());

    let json_on_text = RestQueryPolicy::default().allow_filter("name", [Lookup::JsonContains]);
    assert!(json_on_text.validate_model_policy::<Demo>().is_err());

    assert_eq!(Lookup::parse("month"), Some(Lookup::Month));
    assert_eq!(Lookup::parse("haskey"), Some(Lookup::HasKey));
}

#[tokio::test]
async fn filter_search_ordering_and_page_are_allowlisted() {
    let pool = PgPool::connect_lazy("postgresql://localhost/test")
        .unwrap_or_else(|error| panic!("lazy pool: {error}"));
    let queryset = QuerySet::<Demo, _>::from_executor(PoolExecutor::new(pool));
    let policy = RestQueryPolicy::default()
        .allow_filter("name", [Lookup::Exact, Lookup::IContains])
        .allow_search("name")
        .allow_ordering("name");
    let params = HashMap::from([
        ("name__icontains".into(), "alice".into()),
        ("ordering".into(), "-name".into()),
        ("page".into(), "2".into()),
        ("page_size".into(), "25".into()),
    ]);
    let applied = policy
        .apply::<Demo, _>(&params, queryset)
        .unwrap_or_else(|error| panic!("apply: {error}"));
    let preview = applied
        .queryset
        .preview()
        .unwrap_or_else(|error| panic!("preview: {error}"));
    assert!(preview.sql.contains("ILIKE $1"));
    assert!(preview.sql.contains("ORDER BY"));
    assert!(preview.sql.contains("LIMIT 25 OFFSET 25"));
}

#[tokio::test]
async fn rest_query_limits_fail_closed() {
    let pool = PgPool::connect_lazy("postgresql://localhost/test")
        .unwrap_or_else(|error| panic!("lazy pool: {error}"));
    let queryset = QuerySet::<Demo, _>::from_executor(PoolExecutor::new(pool));
    let policy = RestQueryPolicy::default()
        .allow_filter("name", [Lookup::Exact])
        .request_limits(2, 4, 16)
        .unwrap_or_else(|error| panic!("policy: {error}"));
    let oversized = HashMap::from([("name".into(), "abcde".into())]);
    assert!(matches!(
        policy.apply::<Demo, _>(&oversized, queryset),
        Err(RestError { ref code, .. }) if code == "query_value_too_large"
    ));
}

#[test]
fn partial_serializer_returns_validated_patch_instead_of_constructing_partial_model() {
    let policy = SerializerPolicy::for_model::<Demo>();
    let patch = policy
        .validate_input(
            &serde_json::json!({"name": "updated"}),
            SerializerMode::PartialUpdate,
        )
        .unwrap_or_else(|error| panic!("patch: {error}"));
    assert_eq!(patch.get("name"), Some(&serde_json::json!("updated")));
    assert!(
        policy
            .deserialize::<Demo>(
                &serde_json::json!({"name": "updated"}),
                SerializerMode::PartialUpdate
            )
            .is_err()
    );
}

#[test]
fn cursor_roundtrip_is_signed_versioned_and_bounded() {
    let keys = CursorKeyRing::new(vec![7_u8; 32]).unwrap_or_else(|error| panic!("keys: {error}"));
    let token = CursorToken {
        version: CursorToken::VERSION,
        model: Demo::table_name().into(),
        field: "id".into(),
        direction: SortDirection::Asc,
        value: "10".into(),
        primary_key: "10".into(),
    };
    let encoded = token
        .encode(&keys)
        .unwrap_or_else(|error| panic!("encode: {error}"));
    let decoded = CursorToken::decode(&encoded, Some(&keys))
        .unwrap_or_else(|error| panic!("decode: {error}"));
    assert_eq!(decoded, token);

    let mut tampered = encoded.into_bytes();
    let last = tampered.len().saturating_sub(1);
    tampered[last] = if tampered[last] == b'A' { b'B' } else { b'A' };
    let tampered = String::from_utf8(tampered).unwrap_or_else(|error| panic!("utf8: {error}"));
    assert!(CursorToken::decode(&tampered, Some(&keys)).is_err());
    assert!(CursorToken::decode(&"x".repeat(4097), Some(&keys)).is_err());
}

#[tokio::test]
async fn cursor_is_bound_to_model_policy() {
    let pool = PgPool::connect_lazy("postgresql://localhost/test")
        .unwrap_or_else(|error| panic!("lazy pool: {error}"));
    let keys = CursorKeyRing::new(vec![9_u8; 32]).unwrap_or_else(|error| panic!("keys: {error}"));
    let token = CursorToken {
        version: CursorToken::VERSION,
        model: "different_model".into(),
        field: "id".into(),
        direction: SortDirection::Asc,
        value: "10".into(),
        primary_key: "10".into(),
    };
    let encoded = token
        .encode(&keys)
        .unwrap_or_else(|error| panic!("encode: {error}"));
    let policy = RestQueryPolicy::default()
        .allow_ordering("id")
        .pagination(PaginationStyle::Cursor {
            ordering_field: "id".into(),
            direction: SortDirection::Asc,
        })
        .cursor_signing_key(vec![9_u8; 32])
        .unwrap_or_else(|error| panic!("policy: {error}"));
    let params = HashMap::from([("cursor".into(), encoded)]);
    let queryset = QuerySet::<Demo, _>::from_executor(PoolExecutor::new(pool));
    assert!(matches!(
        policy.apply::<Demo, _>(&params, queryset),
        Err(RestError { ref code, .. }) if code == "invalid_cursor"
    ));
}

#[test]
fn cursor_key_rotation_accepts_previous_key() {
    let previous =
        CursorKeyRing::new(vec![3_u8; 32]).unwrap_or_else(|error| panic!("previous keys: {error}"));
    let token = CursorToken {
        version: CursorToken::VERSION,
        model: Demo::table_name().into(),
        field: "id".into(),
        direction: SortDirection::Asc,
        value: "10".into(),
        primary_key: "10".into(),
    };
    let encoded = token
        .encode(&previous)
        .unwrap_or_else(|error| panic!("encode: {error}"));
    let mut rotated =
        CursorKeyRing::new(vec![4_u8; 32]).unwrap_or_else(|error| panic!("active keys: {error}"));
    rotated
        .push_previous(vec![3_u8; 32])
        .unwrap_or_else(|error| panic!("rotate: {error}"));
    assert_eq!(
        CursorToken::decode(&encoded, Some(&rotated))
            .unwrap_or_else(|error| panic!("decode: {error}")),
        token
    );
}
