use db_controller::{
    AtomicTransaction, ColumnSpec, Entity, Field, LockWait, ModelError, ModelManager, OnDelete,
    RelationCardinality, RelationSpec, RowLock, Tracked, TransactionOptions,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::{FromRow, postgres::PgPoolOptions};
use std::{
    collections::HashMap,
    env,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
struct IntegrationRecord {
    id: Uuid,
    tenant_id: Uuid,
    external_key: String,
    name: String,
    balance: i64,
    version: i64,
    active: bool,
    deleted_at: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl Entity for IntegrationRecord {
    fn table_name() -> &'static str {
        "orm_integration_records"
    }

    fn primary_key() -> &'static str {
        "id"
    }

    fn tenant_column() -> Option<&'static str> {
        Some("tenant_id")
    }

    fn soft_delete_column() -> Option<&'static str> {
        Some("deleted_at")
    }

    fn version_column() -> Option<&'static str> {
        Some("version")
    }

    fn schema() -> HashMap<String, ColumnSpec> {
        HashMap::from([
            ("id".into(), column("UUID", false, false, true, None)),
            (
                "tenant_id".into(),
                column("UUID", false, false, false, None),
            ),
            (
                "external_key".into(),
                column("TEXT", false, true, false, None),
            ),
            ("name".into(), column("TEXT", false, false, false, None)),
            (
                "balance".into(),
                column("BIGINT", false, false, false, None),
            ),
            (
                "version".into(),
                column("BIGINT", false, false, false, Some("0")),
            ),
            (
                "active".into(),
                column("BOOLEAN", false, false, false, Some("true")),
            ),
            (
                "deleted_at".into(),
                column("TIMESTAMPTZ", true, false, false, None),
            ),
            (
                "created_at".into(),
                column("TIMESTAMPTZ", false, false, false, Some("now()")),
            ),
            (
                "updated_at".into(),
                column("TIMESTAMPTZ", false, false, false, Some("now()")),
            ),
        ])
    }

    fn to_map(&self) -> HashMap<String, Value> {
        HashMap::from([
            ("id".into(), json!(self.id)),
            ("tenant_id".into(), json!(self.tenant_id)),
            ("external_key".into(), json!(self.external_key)),
            ("name".into(), json!(self.name)),
            ("balance".into(), json!(self.balance)),
            ("version".into(), json!(self.version)),
            ("active".into(), json!(self.active)),
            ("deleted_at".into(), json!(self.deleted_at)),
            ("created_at".into(), json!(self.created_at)),
            ("updated_at".into(), json!(self.updated_at)),
        ])
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
struct IntegrationProfile {
    id: Uuid,
    tenant_id: Uuid,
    record_id: Uuid,
    label: String,
    deleted_at: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl Entity for IntegrationProfile {
    fn table_name() -> &'static str {
        "orm_integration_profiles"
    }

    fn primary_key() -> &'static str {
        "id"
    }

    fn tenant_column() -> Option<&'static str> {
        Some("tenant_id")
    }

    fn soft_delete_column() -> Option<&'static str> {
        Some("deleted_at")
    }

    fn schema() -> HashMap<String, ColumnSpec> {
        HashMap::from([
            ("id".into(), column("UUID", false, false, true, None)),
            (
                "tenant_id".into(),
                column("UUID", false, false, false, None),
            ),
            (
                "record_id".into(),
                column("UUID", false, false, false, None),
            ),
            ("label".into(), column("TEXT", false, false, false, None)),
            (
                "deleted_at".into(),
                column("TIMESTAMPTZ", true, false, false, None),
            ),
            (
                "created_at".into(),
                column("TIMESTAMPTZ", false, false, false, Some("now()")),
            ),
            (
                "updated_at".into(),
                column("TIMESTAMPTZ", false, false, false, Some("now()")),
            ),
        ])
    }

    fn unique_constraints() -> Vec<db_controller::UniqueConstraintSpec> {
        vec![db_controller::UniqueConstraintSpec {
            name: "uq_orm_integration_profiles_tenant_record",
            columns: &["tenant_id", "record_id"],
        }]
    }

    fn to_map(&self) -> HashMap<String, Value> {
        HashMap::from([
            ("id".into(), json!(self.id)),
            ("tenant_id".into(), json!(self.tenant_id)),
            ("record_id".into(), json!(self.record_id)),
            ("label".into(), json!(self.label)),
            ("deleted_at".into(), json!(self.deleted_at)),
            ("created_at".into(), json!(self.created_at)),
            ("updated_at".into(), json!(self.updated_at)),
        ])
    }
}

fn profile(tenant_id: Uuid, record_id: Uuid, label: &str) -> IntegrationProfile {
    let now = chrono::Utc::now();
    IntegrationProfile {
        id: Uuid::new_v4(),
        tenant_id,
        record_id,
        label: label.into(),
        deleted_at: None,
        created_at: now,
        updated_at: now,
    }
}

fn profile_relation() -> RelationSpec {
    RelationSpec {
        field_name: "profile",
        target_table: IntegrationProfile::table_name(),
        local_column: "id",
        remote_column: "record_id",
        cardinality: RelationCardinality::One,
        nullable: true,
        related_name: None,
        on_delete: OnDelete::Cascade,
    }
}

fn column(
    sql_type: &str,
    nullable: bool,
    unique: bool,
    primary_key: bool,
    default: Option<&str>,
) -> ColumnSpec {
    ColumnSpec {
        sql_type: sql_type.into(),
        nullable,
        unique,
        primary_key,
        default: default.map(str::to_owned),
        encrypted: false,
    }
}

fn record(tenant_id: Uuid, external_key: &str, name: &str) -> IntegrationRecord {
    let now = chrono::Utc::now();
    IntegrationRecord {
        id: Uuid::new_v4(),
        tenant_id,
        external_key: external_key.into(),
        name: name.into(),
        balance: 10,
        version: 0,
        active: true,
        deleted_at: None,
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
#[ignore = "requires a disposable PostgreSQL database"]
async fn postgres_queryset_transaction_and_model_state_contracts()
-> Result<(), Box<dyn std::error::Error>> {
    if env::var("CONFIRM_DISPOSABLE_DATABASE").as_deref() != Ok("yes") {
        return Err(
            "set CONFIRM_DISPOSABLE_DATABASE=yes to run destructive integration tests".into(),
        );
    }
    let database_url = env::var("TEST_DATABASE_URL")
        .map_err(|_| "TEST_DATABASE_URL is required for PostgreSQL integration tests")?;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&database_url)
        .await?;

    sqlx::query("DROP TABLE IF EXISTS orm_integration_profiles")
        .execute(&pool)
        .await?;
    sqlx::query("DROP TABLE IF EXISTS orm_integration_records")
        .execute(&pool)
        .await?;
    sqlx::query(sqlx::AssertSqlSafe(IntegrationRecord::create_sql()))
        .execute(&pool)
        .await?;
    sqlx::query(sqlx::AssertSqlSafe(IntegrationProfile::create_sql()))
        .execute(&pool)
        .await?;

    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let manager = ModelManager::<IntegrationRecord>::new(pool.clone());

    let first = record(tenant_a, "first", "alpha");
    let created = manager
        .queryset()
        .for_tenant(tenant_a)
        .create(&first)
        .await?;
    assert_eq!(created.external_key, "first");

    assert_eq!(manager.queryset().for_tenant(tenant_a).count().await?, 1);
    assert_eq!(manager.queryset().for_tenant(tenant_b).count().await?, 0);

    let filtered = manager
        .queryset()
        .for_tenant(tenant_a)
        .filter(Field::<String>::new("name").iexact("ALPHA"))
        .get()
        .await?;
    assert_eq!(filtered.id, first.id);

    let profile_manager = ModelManager::<IntegrationProfile>::new(pool.clone());
    let tenant_a_profile = profile(tenant_a, first.id, "tenant-a");
    let tenant_b_profile = profile(tenant_b, first.id, "tenant-b");
    profile_manager
        .queryset()
        .for_tenant(tenant_a)
        .create(&tenant_a_profile)
        .await?;
    profile_manager
        .queryset()
        .for_tenant(tenant_b)
        .create(&tenant_b_profile)
        .await?;
    let joined = manager
        .queryset()
        .for_tenant(tenant_a)
        .filter(Field::<Uuid>::new("id").eq(first.id))
        .select_related::<IntegrationProfile>(&profile_relation())
        .await?;
    assert_eq!(joined.len(), 1);
    let related = joined[0]
        .related
        .as_ref()
        .ok_or("tenant-scoped related row should be present")?;
    assert_eq!(related.tenant_id, tenant_a);
    assert_eq!(related.label, "tenant-a");

    profile_manager
        .queryset()
        .for_tenant(tenant_a)
        .filter(Field::<Uuid>::new("id").eq(tenant_a_profile.id))
        .soft_delete()
        .await?;
    let joined_after_delete = manager
        .queryset()
        .for_tenant(tenant_a)
        .filter(Field::<Uuid>::new("id").eq(first.id))
        .select_related::<IntegrationProfile>(&profile_relation())
        .await?;
    assert_eq!(joined_after_delete.len(), 1);
    assert!(joined_after_delete[0].related.is_none());

    manager
        .queryset()
        .for_tenant(tenant_a)
        .filter(Field::<Uuid>::new("id").eq(first.id))
        .increment("balance", 5_i64)
        .await?;
    let incremented = manager
        .queryset()
        .for_tenant(tenant_a)
        .filter(Field::<Uuid>::new("id").eq(first.id))
        .get()
        .await?;
    assert_eq!(incremented.balance, 15);

    let affected = manager
        .queryset()
        .for_tenant(tenant_a)
        .filter(Field::<Uuid>::new("id").eq(first.id))
        .update_versioned(
            0,
            Map::from_iter([("name".into(), Value::String("beta".into()))]),
        )
        .await?;
    assert_eq!(affected, 1);
    let stale = manager
        .queryset()
        .for_tenant(tenant_a)
        .filter(Field::<Uuid>::new("id").eq(first.id))
        .update_versioned(
            0,
            Map::from_iter([("name".into(), Value::String("stale".into()))]),
        )
        .await;
    assert!(matches!(stale, Err(ModelError::ConcurrentModification)));

    let second = record(tenant_a, "second", "gamma");
    let (created_second, was_created) = manager
        .queryset()
        .for_tenant(tenant_a)
        .get_or_create_by("external_key", "second", &second)
        .await?;
    assert!(was_created);
    let (same_second, was_created_again) = manager
        .queryset()
        .for_tenant(tenant_a)
        .get_or_create_by("external_key", "second", &second)
        .await?;
    assert!(!was_created_again);
    assert_eq!(same_second.id, created_second.id);

    let expression_affected = manager
        .queryset()
        .for_tenant(tenant_a)
        .filter(Field::<Uuid>::new("id").eq(created_second.id))
        .update_expressions(&[Field::<i64>::new("balance").add(7_i64)])
        .await?;
    assert_eq!(expression_affected, 1);
    let expression_updated = manager
        .queryset()
        .for_tenant(tenant_a)
        .filter(Field::<Uuid>::new("id").eq(created_second.id))
        .get()
        .await?;
    assert_eq!(expression_updated.balance, 17);
    assert_eq!(expression_updated.version, 1);

    let mut bulk_a = record(tenant_a, "bulk-a", "bulk-a-before");
    let mut bulk_b = record(tenant_a, "bulk-b", "bulk-b-before");
    manager
        .queryset()
        .for_tenant(tenant_a)
        .bulk_create(&[bulk_a.clone(), bulk_b.clone()], false)
        .await?;
    bulk_a.name = "bulk-a-after".into();
    bulk_a.balance = 101;
    bulk_b.name = "bulk-b-after".into();
    bulk_b.balance = 202;
    let bulk_affected = manager
        .queryset()
        .for_tenant(tenant_a)
        .filter(Field::<Uuid>::new("id").is_in([bulk_a.id, bulk_b.id]))
        .bulk_update(&[bulk_a.clone(), bulk_b.clone()], &["name", "balance"])
        .await?;
    assert_eq!(bulk_affected, 2);
    let bulk_rows = manager
        .queryset()
        .for_tenant(tenant_a)
        .filter(Field::<Uuid>::new("id").is_in([bulk_a.id, bulk_b.id]))
        .order_by(Field::<String>::new("external_key").asc())
        .all()
        .await?;
    assert_eq!(bulk_rows.len(), 2);
    assert_eq!(bulk_rows[0].name, "bulk-a-after");
    assert_eq!(bulk_rows[0].balance, 101);
    assert_eq!(bulk_rows[0].version, 1);
    assert_eq!(bulk_rows[1].name, "bulk-b-after");
    assert_eq!(bulk_rows[1].balance, 202);
    assert_eq!(bulk_rows[1].version, 1);

    let projected = manager
        .queryset()
        .for_tenant(tenant_a)
        .filter(Field::<String>::new("external_key").eq("second"))
        .clear_ordering()
        .values_list(&["external_key", "balance"], false)
        .await?;
    assert_eq!(projected, vec![json!(["second", 17])]);

    let mut upsert_tx = AtomicTransaction::begin(&pool, TransactionOptions::default()).await?;
    upsert_tx.set_tenant(tenant_a).await?;
    let upsert_seed = record(tenant_a, "transactional-upsert", "created-in-upsert");
    let (upsert_created, created_now) = upsert_tx
        .queryset::<IntegrationRecord>()
        .for_tenant(tenant_a)
        .update_or_create_by("external_key", "transactional-upsert", &upsert_seed)
        .await?;
    assert!(created_now);
    let mut upsert_changed = upsert_seed.clone();
    upsert_changed.id = Uuid::new_v4();
    upsert_changed.name = "updated-in-upsert".into();
    upsert_changed.balance = 77;
    let (upsert_updated, created_again) = upsert_tx
        .queryset::<IntegrationRecord>()
        .for_tenant(tenant_a)
        .update_or_create_by("external_key", "transactional-upsert", &upsert_changed)
        .await?;
    assert!(!created_again);
    assert_eq!(upsert_updated.id, upsert_created.id);
    assert_eq!(upsert_updated.name, "updated-in-upsert");
    assert_eq!(upsert_updated.balance, 77);
    assert_eq!(upsert_updated.version, 1);
    upsert_tx.commit().await?;

    let outside_transaction = manager
        .queryset()
        .for_tenant(tenant_a)
        .update_or_create_by("external_key", "transactional-upsert", &upsert_changed)
        .await;
    assert!(matches!(
        outside_transaction,
        Err(ModelError::TransactionRequired)
    ));

    let mut atomic = AtomicTransaction::begin(&pool, TransactionOptions::default()).await?;
    atomic.set_tenant(tenant_a).await?;
    let locked = atomic
        .queryset::<IntegrationRecord>()
        .for_tenant(tenant_a)
        .filter(Field::<Uuid>::new("id").eq(first.id))
        .select_for_update(RowLock::Update, LockWait::NoWait)
        .get()
        .await?;
    assert_eq!(locked.id, first.id);
    atomic.commit().await?;

    let callbacks_run = Arc::new(AtomicUsize::new(0));
    let mut callback_tx = AtomicTransaction::begin(&pool, TransactionOptions::default()).await?;
    let rollback_target = callback_tx.create_savepoint().await?;
    let callback_counter = Arc::clone(&callbacks_run);
    callback_tx.on_commit(move || {
        let callback_counter = Arc::clone(&callback_counter);
        async move {
            callback_counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    });
    callback_tx.rollback_to(&rollback_target).await?;
    callback_tx.commit().await?;
    assert_eq!(callbacks_run.load(Ordering::SeqCst), 0);

    let mut lineage_tx = AtomicTransaction::begin(&pool, TransactionOptions::default()).await?;
    let outer_savepoint = lineage_tx.create_savepoint().await?;
    let mut inner_savepoint = lineage_tx.create_savepoint().await?;
    lineage_tx.rollback_to(&outer_savepoint).await?;
    let stale_inner = lineage_tx.release_savepoint(&mut inner_savepoint).await;
    assert!(matches!(stale_inner, Err(ModelError::InvalidQuery(_))));
    lineage_tx.rollback().await?;

    let current = manager
        .queryset()
        .for_tenant(tenant_a)
        .filter(Field::<Uuid>::new("id").eq(first.id))
        .get()
        .await?;
    let mut tracked = Tracked::new(current);
    tracked.name = "tracked".into();
    let saved = tracked.save(&manager).await?;
    assert_eq!(saved.name, "tracked");
    assert_eq!(saved.version, 2);

    let refreshed = manager
        .queryset()
        .for_tenant(tenant_a)
        .refresh_from_db(&saved)
        .await?;
    assert_eq!(refreshed.version, 2);
    assert_eq!(
        manager
            .queryset()
            .for_tenant(tenant_a)
            .filter(Field::<Uuid>::new("id").eq(first.id))
            .latest("updated_at")
            .await?
            .map(|row| row.id),
        Some(first.id),
    );

    manager
        .queryset()
        .for_tenant(tenant_a)
        .filter(Field::<Uuid>::new("id").eq(first.id))
        .soft_delete()
        .await?;
    let hidden = manager
        .queryset()
        .for_tenant(tenant_a)
        .filter(Field::<Uuid>::new("id").eq(first.id))
        .get()
        .await;
    assert!(matches!(hidden, Err(ModelError::NotFound)));
    let deleted = manager
        .queryset()
        .for_tenant(tenant_a)
        .with_deleted()
        .filter(Field::<Uuid>::new("id").eq(first.id))
        .get()
        .await?;
    assert!(deleted.deleted_at.is_some());

    sqlx::query("DROP TABLE IF EXISTS orm_integration_profiles")
        .execute(&pool)
        .await?;
    sqlx::query("DROP TABLE IF EXISTS orm_integration_records")
        .execute(&pool)
        .await?;
    pool.close().await;
    Ok(())
}
