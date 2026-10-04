#![cfg(feature = "postgres-integration")]
use lex_developer_acceptance::{Budget, Project, Task};
use lex_framework::{
    db_controller::{self as orm, DecimalField, Entity, ModelError, TextField, Tracked, UUIDField},
    migration,
};

#[tokio::test]
async fn postgres_application_migrates_tenant_projects_and_rejects_stale_edits() {
    assert_eq!(
        std::env::var("CONFIRM_DISPOSABLE_DATABASE").as_deref(),
        Ok("yes")
    );
    let url = std::env::var("TEST_DATABASE_URL").expect("a disposable PostgreSQL URL is required");
    let admin = orm::sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("lex_consumer_{}", UUIDField::new().0.simple());
    orm::sqlx::query(orm::sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .unwrap();
    let setting = format!("SET search_path TO {schema}");
    let pool = orm::sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .after_connect(move |connection, _| {
            let sql = setting.clone();
            Box::pin(async move {
                orm::sqlx::query(orm::sqlx::AssertSqlSafe(sql))
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    let files = tempfile::tempdir().unwrap();
    let up = [
        orm::generate_create_table_sql::<Project>(),
        orm::generate_create_table_sql::<Budget>(),
        orm::generate_create_table_sql::<Task>(),
    ]
    .join("\n");
    std::fs::write(files.path().join("0001_models_up.sql"), up).unwrap();
    std::fs::write(files.path().join("0001_models_down.sql"),"DROP TABLE acceptance_tasks; DROP TABLE acceptance_budgets; DROP TABLE acceptance_projects;").unwrap();
    let directory = files.path().to_str().unwrap();
    migration::handle_migrate(&pool, directory).await.unwrap();
    migration::handle_migrate(&pool, directory).await.unwrap();
    assert!(
        migration::pending_migrations(&pool, directory)
            .await
            .unwrap()
            .is_empty()
    );
    Project::register();
    let manager = Project::objects(pool.clone());
    let [alpha, beta] = [UUIDField::new(), UUIDField::new()];
    let mut beta_id = None;
    for tenant in [&alpha, &beta] {
        let row = manager
            .queryset()
            .for_tenant(tenant.clone())
            .create(&Project {
                id: UUIDField::new(),
                tenant_id: tenant.clone(),
                title: TextField::new("'; DROP TABLE acceptance_projects; -- 世界"),
                deleted_at: None,
            })
            .await
            .unwrap();
        if tenant.0 == beta.0 {
            beta_id = Some(row.id);
        }
    }
    assert!(manager.queryset().all().await.is_err());
    assert_eq!(
        manager
            .queryset()
            .for_tenant(alpha.clone())
            .count()
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        manager
            .queryset()
            .for_tenant(alpha.clone())
            .filter(Project::ID.eq(beta_id.unwrap()))
            .update(serde_json::Map::from_iter([(
                "title".into(),
                serde_json::json!("forged")
            )]))
            .await
            .unwrap(),
        0
    );
    let tasks = Task::objects(pool.clone());
    tasks
        .queryset()
        .create(&Task {
            id: UUIDField::new(),
            title: TextField::new("original"),
            version: 1,
        })
        .await
        .unwrap();
    let mut alice = Tracked::new(tasks.queryset().get().await.unwrap());
    let mut bob = Tracked::new(tasks.queryset().get().await.unwrap());
    alice.title = TextField::new("Alice's edit");
    assert_eq!(alice.save(&tasks).await.unwrap().version, 2);
    bob.title = TextField::new("stale Bob edit");
    assert!(matches!(
        bob.save(&tasks).await,
        Err(ModelError::ConcurrentModification)
    ));
    assert_eq!(
        tasks.queryset().get().await.unwrap().title.as_str(),
        "Alice's edit"
    );
    let exact = "99999999999999999.123456789";
    let budget = Budget::objects(pool.clone())
        .queryset()
        .create(&Budget {
            id: UUIDField::new(),
            amount: DecimalField::from_str(exact).unwrap(),
            adjustment: None,
        })
        .await
        .unwrap();
    assert_eq!(budget.amount.value().to_string(), exact);
    migration::handle_rollback(&pool, directory).await.unwrap();
    let table: Option<String> =
        orm::sqlx::query_scalar("SELECT to_regclass('acceptance_projects')::text")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(table.is_none());
    pool.close().await;
    orm::sqlx::query(orm::sqlx::AssertSqlSafe(format!(
        "DROP SCHEMA {schema} CASCADE"
    )))
    .execute(&admin)
    .await
    .unwrap();
}
