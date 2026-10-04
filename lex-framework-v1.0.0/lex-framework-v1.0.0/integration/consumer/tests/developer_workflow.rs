use lex_developer_acceptance::Project;
use lex_framework::{
    db_controller::{self as orm, Field, SqliteModelManager, TextField, UUIDField},
    prelude::*,
};
use serde_json::{Map, json};

#[tokio::test]
async fn developer_can_create_query_update_and_delete_isolated_tenant_projects() {
    let pool = orm::sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    orm::sqlx::raw_sql(orm::sqlx::AssertSqlSafe(
        orm::generate_create_table_sql_sqlite::<Project>(),
    ))
    .execute(&pool)
    .await
    .unwrap();
    Project::register();
    let manager = SqliteModelManager::<Project>::new(pool.clone());
    let alpha = UUIDField::new();
    let beta = UUIDField::new();
    let injection = "admin' OR 1=1 -- 世界";
    for tenant in [&alpha, &beta] {
        manager
            .queryset()
            .for_tenant(tenant.clone())
            .create(&Project {
                id: UUIDField::new(),
                tenant_id: tenant.clone(),
                title: TextField::new(injection),
                deleted_at: None,
            })
            .await
            .unwrap();
    }
    assert!(
        manager.queryset().all().await.is_err(),
        "missing tenant fails closed"
    );
    let row = manager
        .queryset()
        .for_tenant(alpha.clone())
        .filter(Project::TITLE.eq(injection))
        .filter(Project::TENANT_ID.eq(alpha.clone()))
        .get()
        .await
        .unwrap();
    assert_eq!(row.title.as_str(), injection);
    assert_eq!(row.tenant_id.0, alpha.0);
    assert_eq!(
        manager
            .queryset()
            .for_tenant(beta.clone())
            .count()
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        manager
            .queryset()
            .for_tenant(alpha.clone())
            .filter(Field::<UUIDField>::new("id").eq(row.id.clone()))
            .update(Map::from_iter([("title".into(), json!("changed"))]))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        manager
            .queryset()
            .for_tenant(beta.clone())
            .get()
            .await
            .unwrap()
            .title
            .as_str(),
        injection
    );
    assert_eq!(
        manager
            .queryset()
            .for_tenant(alpha.clone())
            .filter(Project::ID.eq(row.id.clone()))
            .soft_delete()
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        manager
            .queryset()
            .for_tenant(alpha.clone())
            .count()
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        manager
            .queryset()
            .for_tenant(alpha.clone())
            .only_deleted()
            .count()
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        manager
            .queryset()
            .for_tenant(alpha.clone())
            .only_deleted()
            .filter(Project::ID.eq(row.id.clone()))
            .restore()
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        manager
            .queryset()
            .for_tenant(alpha)
            .filter(Project::ID.eq(row.id.clone()))
            .hard_delete()
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        manager.queryset().for_tenant(beta).count().await.unwrap(),
        1
    );
    assert!(
        manager
            .queryset()
            .filter(Field::<String>::new("title; DROP TABLE acceptance_projects").eq("x"))
            .preview()
            .is_err()
    );
}

#[tokio::test]
async fn sqlite_model_round_trips_exact_decimal_values_and_nullable_adjustments() {
    use lex_developer_acceptance::Budget;
    use orm::DecimalField;
    let pool = orm::sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let manager = Budget::objects_sqlite_auto(pool).await.unwrap();
    let exact = "99999999999999999.123456789";
    let created = manager
        .queryset()
        .create(&Budget {
            id: UUIDField::new(),
            amount: DecimalField::from_str(exact).unwrap(),
            adjustment: None,
        })
        .await
        .unwrap();
    assert_eq!(created.amount.value().to_string(), exact);
    assert!(created.adjustment.is_none());
    let loaded = manager.queryset().get().await.unwrap();
    assert_eq!(loaded.amount.value().to_string(), exact);
}

#[tokio::test]
async fn stale_task_edit_returns_a_conflict_without_overwriting_the_first_editor() {
    use lex_developer_acceptance::Task;
    use orm::{ModelError, Tracked};
    let pool = orm::sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let manager = Task::objects_sqlite_auto(pool).await.unwrap();
    manager
        .queryset()
        .create(&Task {
            id: UUIDField::new(),
            title: TextField::new("original"),
            version: 1,
        })
        .await
        .unwrap();
    let mut alice = Tracked::new(manager.queryset().get().await.unwrap());
    let mut bob = Tracked::new(manager.queryset().get().await.unwrap());
    assert_eq!(
        alice.save_sqlite(&manager).await.unwrap().version,
        1,
        "a clean save must not advance the version"
    );
    alice.title = TextField::new("Alice's edit");
    assert!(alice.is_dirty());
    assert_eq!(
        alice.dirty_fields().into_iter().collect::<Vec<_>>(),
        ["title"]
    );
    assert_eq!(alice.save_sqlite(&manager).await.unwrap().version, 2);
    assert!(!alice.is_dirty());
    bob.title = TextField::new("stale Bob edit");
    assert!(matches!(
        bob.save_sqlite(&manager).await,
        Err(ModelError::ConcurrentModification)
    ));
    let persisted = manager.queryset().get().await.unwrap();
    assert_eq!(persisted.title.as_str(), "Alice's edit");
    assert_eq!(persisted.version, 2);
    alice.id = UUIDField::new();
    alice.title = TextField::new("forged identity");
    assert!(matches!(
        alice.save_sqlite(&manager).await,
        Err(ModelError::InvalidQuery(_))
    ));
    assert_eq!(
        manager.queryset().get().await.unwrap().title.as_str(),
        "Alice's edit"
    );
}

#[tokio::test]
async fn a_failed_bulk_import_never_leaves_a_partial_batch_in_sqlite() {
    use lex_developer_acceptance::Task;
    let pool = orm::sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let manager = Task::objects_sqlite_auto(pool).await.unwrap();
    let id = UUIDField::new();
    let batch = [
        Task {
            id: id.clone(),
            title: TextField::new("first"),
            version: 1,
        },
        Task {
            id,
            title: TextField::new("duplicate primary key"),
            version: 1,
        },
    ];
    assert!(manager.queryset().bulk_create(&batch, false).await.is_err());
    assert_eq!(
        manager.queryset().count().await.unwrap(),
        0,
        "the failed batch must persist no rows"
    );
}

#[tokio::test]
async fn a_failed_bulk_edit_preserves_every_original_row_in_sqlite() {
    use lex_developer_acceptance::Task;
    let pool = orm::sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let manager = Task::objects_sqlite_auto(pool.clone()).await.unwrap();
    orm::sqlx::query("CREATE UNIQUE INDEX acceptance_unique_task_title ON acceptance_tasks(title)")
        .execute(&pool)
        .await
        .unwrap();
    let mut batch = [
        Task {
            id: UUIDField::new(),
            title: TextField::new("first"),
            version: 1,
        },
        Task {
            id: UUIDField::new(),
            title: TextField::new("second"),
            version: 1,
        },
    ];
    manager.queryset().bulk_create(&batch, false).await.unwrap();
    for row in &mut batch {
        row.title = TextField::new("same conflicting title");
    }
    assert!(
        manager
            .queryset()
            .bulk_update(&batch, &["title"])
            .await
            .is_err()
    );
    let titles = manager
        .queryset()
        .order_by(orm::OrderBy::asc("title"))
        .values_list(&["title"], true)
        .await
        .unwrap();
    assert_eq!(titles, [json!("first"), json!("second")]);
    assert_eq!(
        manager
            .queryset()
            .aggregate("version", orm::AggregateFunction::Sum)
            .await
            .unwrap(),
        json!(2)
    );
}

#[tokio::test]
async fn sqlite_decimal_queries_reject_lexical_comparison_and_lossy_arithmetic() {
    use lex_developer_acceptance::Budget;
    use lex_framework::db_controller::{AggregateFunction, DecimalField, ModelError};
    let pool = orm::sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let manager = Budget::objects_sqlite_auto(pool).await.unwrap();
    let record = Budget {
        id: UUIDField::new(),
        amount: DecimalField::from_str("9007199254740993.125").unwrap(),
        adjustment: None,
    };
    manager.queryset().create(&record).await.unwrap();
    let comparison = manager
        .queryset()
        .filter(Budget::AMOUNT.lt(DecimalField::from_str("3").unwrap()))
        .all()
        .await;
    assert!(
        matches!(comparison, Err(ModelError::InvalidQuery(_))),
        "exact decimal comparisons must not use SQLite text ordering: {comparison:?}"
    );
    assert!(matches!(
        manager
            .queryset()
            .order_by(Budget::AMOUNT.asc())
            .all()
            .await,
        Err(ModelError::InvalidQuery(_))
    ));
    for function in [
        AggregateFunction::Sum,
        AggregateFunction::Avg,
        AggregateFunction::Min,
        AggregateFunction::Max,
    ] {
        assert!(matches!(
            manager.queryset().aggregate("amount", function).await,
            Err(ModelError::InvalidQuery(_))
        ));
    }
    assert!(matches!(
        manager
            .queryset()
            .filter(Budget::ID.eq(record.id.clone()))
            .increment("amount", DecimalField::from_str("0.001").unwrap())
            .await,
        Err(ModelError::InvalidQuery(_))
    ));
    assert!(matches!(
        manager
            .queryset()
            .filter(Budget::ID.eq(record.id.clone()))
            .update_expressions(&[Budget::AMOUNT.add(DecimalField::from_str("0.001").unwrap())])
            .await,
        Err(ModelError::InvalidQuery(_))
    ));
    let loaded = manager
        .queryset()
        .filter(Budget::ID.eq(record.id))
        .get()
        .await
        .unwrap();
    assert_eq!(loaded.amount.0.to_string(), "9007199254740993.125");
    assert_eq!(
        manager
            .queryset()
            .order_by(Budget::AMOUNT.asc())
            .count()
            .await
            .unwrap(),
        1
    );
    assert!(
        manager
            .queryset()
            .order_by(Budget::AMOUNT.asc())
            .exists()
            .await
            .unwrap()
    );
    assert_eq!(
        manager
            .queryset()
            .filter(Budget::ADJUSTMENT.is_null())
            .count()
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        manager
            .queryset()
            .aggregate("amount", AggregateFunction::Count)
            .await
            .unwrap(),
        serde_json::json!(1)
    );
}

#[tokio::test]
async fn sqlite_rejects_decimal_unique_keys_and_disabled_foreign_key_pools() {
    use lex_framework::db_controller::{DecimalField, ModelError};
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Model)]
    #[model(global)]
    #[table(name = "unsupported_decimal_unique")]
    struct UniqueAmount {
        #[field(primary_key)]
        id: UUIDField,
        #[field(unique)]
        amount: DecimalField,
    }
    let pool = orm::sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    assert!(matches!(
        UniqueAmount::objects_sqlite_auto(pool).await,
        Err(ModelError::InvalidQuery(_))
    ));
    let pool = orm::sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            orm::sqlx::sqlite::SqliteConnectOptions::new()
                .in_memory(true)
                .foreign_keys(false),
        )
        .await
        .unwrap();
    assert!(matches!(
        Project::objects_sqlite_auto(pool).await,
        Err(ModelError::InvalidQuery(_))
    ));
}
