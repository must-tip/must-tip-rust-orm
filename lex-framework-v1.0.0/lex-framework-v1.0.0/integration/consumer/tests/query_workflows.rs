use lex_developer_acceptance::Task;
use lex_framework::db_controller::{
    self as orm, ModelError, OrderBy, QueryExecutor, QuerySet, TextField, UUIDField,
};
use serde_json::{Map, json};

async fn query_story<E>(query: impl Fn() -> QuerySet<Task, E>)
where
    E: QueryExecutor,
    Task: for<'r> orm::sqlx::FromRow<'r, E::Row>,
{
    let batch = ["Alpha", "Beta", "Gamma"].map(|title| Task {
        id: UUIDField::new(),
        title: TextField::new(title),
        version: if title == "Gamma" { 2 } else { 1 },
    });
    query().bulk_create(&batch, false).await.unwrap();
    assert_eq!(query().all().await.unwrap().len(), 3);
    assert!(matches!(
        query().get().await,
        Err(ModelError::MultipleObjectsReturned)
    ));
    assert!(!query().none().exists().await.unwrap());
    assert!(query().none().first().await.unwrap().is_none());
    assert!(matches!(
        query().none().get().await,
        Err(ModelError::NotFound)
    ));
    assert_eq!(
        query()
            .order_by(OrderBy::asc("title"))
            .first()
            .await
            .unwrap()
            .unwrap()
            .title
            .as_str(),
        "Alpha"
    );
    assert_eq!(
        query()
            .order_by(OrderBy::asc("title"))
            .last()
            .await
            .unwrap()
            .unwrap()
            .title
            .as_str(),
        "Gamma"
    );
    let beta = query()
        .filter(Task::VERSION.gte(1_i64))
        .exclude(Task::TITLE.eq("Gamma"))
        .filter(Task::TITLE.eq("Beta"))
        .get()
        .await
        .unwrap();
    assert_eq!(beta.title.as_str(), "Beta");
    let page = query()
        .order_by(OrderBy::desc("title"))
        .page(1, 2)
        .unwrap()
        .values_list(&["title"], true)
        .await
        .unwrap();
    assert_eq!(page, [json!("Gamma"), json!("Beta")]);
    let second = query()
        .order_by(OrderBy::asc("title"))
        .offset(1)
        .limit(1)
        .values(&["title", "version"])
        .await
        .unwrap();
    assert_eq!(second, [json!({"title":"Beta","version":1})]);
    let ordering = query()
        .order_by(OrderBy::asc("version"))
        .order_by(OrderBy::desc("title"))
        .values_list(&["title"], true)
        .await
        .unwrap();
    assert_eq!(ordering, [json!("Beta"), json!("Alpha"), json!("Gamma")]);
    assert!(
        query()
            .distinct()
            .order_by(OrderBy::asc("version"))
            .values_list(&["version"], true)
            .await
            .is_err()
    );
    let mut versions = query()
        .distinct()
        .clear_ordering()
        .values_list(&["version"], true)
        .await
        .unwrap();
    versions.sort_by_key(|value| value.as_i64().unwrap());
    assert_eq!(versions, [json!(1), json!(2)]);
    assert_eq!(
        query()
            .aggregate("version", orm::AggregateFunction::Sum)
            .await
            .unwrap(),
        json!(4)
    );
    assert_eq!(
        query()
            .aggregate("version", orm::AggregateFunction::Max)
            .await
            .unwrap(),
        json!(2)
    );
    assert!(query().page(0, 2).is_err());
    assert!(
        query()
            .values_list(&["title", "version"], true)
            .await
            .is_err()
    );
    let hostile = "' OR 1=1 -- 世界";
    assert_eq!(
        query()
            .filter(Task::ID.eq(beta.id.clone()))
            .update(Map::from_iter([("title".into(), json!(hostile))]))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        query()
            .filter(Task::TITLE.eq(hostile))
            .count()
            .await
            .unwrap(),
        1
    );
    assert_eq!(query().count().await.unwrap(), 3);
    assert_eq!(
        query()
            .filter(Task::TITLE.eq("Gamma"))
            .hard_delete()
            .await
            .unwrap(),
        1
    );
    assert_eq!(query().count().await.unwrap(), 2);
}

#[tokio::test]
async fn sqlite_application_queries_have_predictable_cardinality_ordering_projection_and_pagination()
 {
    let pool = orm::sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let manager = Task::objects_sqlite_auto(pool).await.unwrap();
    query_story(|| manager.queryset()).await;
}

#[tokio::test]
async fn file_backed_sqlite_retains_commits_but_discards_nested_savepoints_and_rollbacks() {
    use orm::sqlx::Acquire;
    let directory = tempfile::tempdir().unwrap();
    let options = orm::sqlx::sqlite::SqliteConnectOptions::new()
        .filename(directory.path().join("application.sqlite3"))
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = orm::sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(options.clone())
        .await
        .unwrap();
    Task::objects_sqlite_auto(pool.clone()).await.unwrap();
    let mut transaction = pool.begin().await.unwrap();
    QuerySet::<Task, _>::from_executor(orm::SqliteTransactionExecutor::new(&mut transaction))
        .create(&Task {
            id: UUIDField::new(),
            title: TextField::new("committed"),
            version: 1,
        })
        .await
        .unwrap();
    let mut savepoint = transaction.begin().await.unwrap();
    QuerySet::<Task, _>::from_executor(orm::SqliteTransactionExecutor::new(&mut savepoint))
        .create(&Task {
            id: UUIDField::new(),
            title: TextField::new("discarded savepoint"),
            version: 1,
        })
        .await
        .unwrap();
    savepoint.rollback().await.unwrap();
    transaction.commit().await.unwrap();
    let mut transaction = pool.begin().await.unwrap();
    QuerySet::<Task, _>::from_executor(orm::SqliteTransactionExecutor::new(&mut transaction))
        .create(&Task {
            id: UUIDField::new(),
            title: TextField::new("discarded transaction"),
            version: 1,
        })
        .await
        .unwrap();
    transaction.rollback().await.unwrap();
    pool.close().await;
    let reopened = orm::sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .unwrap();
    let rows = Task::objects_sqlite(reopened.clone())
        .queryset()
        .all()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title.as_str(), "committed");
    reopened.close().await;
}

#[cfg(feature = "postgres-integration")]
#[tokio::test]
async fn postgres_application_queries_obey_the_same_portable_query_contract() {
    assert_eq!(
        std::env::var("CONFIRM_DISPOSABLE_DATABASE").as_deref(),
        Ok("yes")
    );
    let url = std::env::var("TEST_DATABASE_URL").expect("dedicated test database required");
    let admin = orm::sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("lex_query_consumer_{}", UUIDField::new().0.simple());
    orm::sqlx::query(orm::sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .unwrap();
    let setting = format!("SET search_path TO {schema}");
    let pool = orm::sqlx::postgres::PgPoolOptions::new()
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
    orm::sqlx::raw_sql(orm::sqlx::AssertSqlSafe(orm::generate_create_table_sql::<
        Task,
    >()))
    .execute(&pool)
    .await
    .unwrap();
    let manager = Task::objects(pool.clone());
    query_story(|| manager.queryset()).await;
    pool.close().await;
    orm::sqlx::query(orm::sqlx::AssertSqlSafe(format!(
        "DROP SCHEMA {schema} CASCADE"
    )))
    .execute(&admin)
    .await
    .unwrap();
}

#[tokio::test]
async fn every_file_sqlite_pool_connection_rejects_invalid_foreign_keys() {
    use lex_framework::db::{Database, Settings};
    let directory = tempfile::tempdir().unwrap();
    let mut settings = Settings::default();
    settings.database.url = format!(
        "sqlite://{}",
        directory.path().join("constraints.sqlite").display()
    );
    settings.database.max_connections = 4;
    settings.database.min_connections = 4;
    let database = Database::initialize_with(&settings).await.unwrap();
    let pool = database.sqlite_pool().unwrap();
    orm::sqlx::raw_sql("CREATE TABLE parent(id INTEGER PRIMARY KEY); CREATE TABLE child(id INTEGER PRIMARY KEY, parent_id INTEGER NOT NULL REFERENCES parent(id))")
        .execute(pool).await.unwrap();
    // Hold every connection simultaneously so the assertions cannot accidentally reuse one.
    let mut connections = Vec::new();
    for _ in 0..4 {
        connections.push(pool.acquire().await.unwrap());
    }
    for (index, connection) in connections.iter_mut().enumerate() {
        let enabled: i64 = orm::sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&mut **connection)
            .await
            .unwrap();
        assert_eq!(enabled, 1);
        assert!(
            orm::sqlx::query("INSERT INTO child(id,parent_id) VALUES($1,999)")
                .bind(index as i64)
                .execute(&mut **connection)
                .await
                .is_err()
        );
    }
    drop(connections);
    database.close().await;
}
