#![cfg(feature = "postgres-integration")]
use db_controller::{TenantRlsPolicy, set_transaction_tenant};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

#[tokio::test]
async fn pooled_connections_never_retain_a_previous_tenants_rls_context() {
    assert_eq!(
        std::env::var("CONFIRM_DISPOSABLE_DATABASE").as_deref(),
        Ok("yes")
    );
    let url = std::env::var("TEST_DATABASE_URL").expect("dedicated PostgreSQL required");
    let admin = PgPool::connect(&url).await.unwrap();
    let suffix = Uuid::new_v4().simple();
    let schema = format!("lex_rls_{suffix}");
    let role = format!("lex_rls_user_{suffix}");
    let setup = format!(
        "CREATE SCHEMA {schema}; CREATE ROLE {role} NOLOGIN NOSUPERUSER NOBYPASSRLS; CREATE TABLE {schema}.records (id UUID PRIMARY KEY, tenant_id UUID NOT NULL, title TEXT NOT NULL); GRANT USAGE ON SCHEMA {schema} TO {role}; GRANT SELECT,INSERT,UPDATE,DELETE ON {schema}.records TO {role}"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(setup))
        .execute(&admin)
        .await
        .unwrap();
    let [alpha, beta] = [Uuid::new_v4(), Uuid::new_v4()];
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO {schema}.records VALUES ($1,$2,'Alpha'),($3,$4,'Beta')"
    )))
    .bind(Uuid::new_v4())
    .bind(alpha)
    .bind(Uuid::new_v4())
    .bind(beta)
    .execute(&admin)
    .await
    .unwrap();
    let table = format!("{schema}.records");
    // The runtime policy uses static model metadata; leaked strings live for this test process.
    let policy = TenantRlsPolicy {
        table: Box::leak(table.into_boxed_str()),
        tenant_column: "tenant_id",
        force: true,
    };
    for sql in policy.enable_sql().unwrap() {
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(&admin)
            .await
            .unwrap();
    }
    let role_sql = format!("SET ROLE {role}; SET search_path TO {schema}");
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .after_connect(move |connection, _| {
            let sql = role_sql.clone();
            Box::pin(async move {
                sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM records")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    for (tenant, expected) in [(alpha, "Alpha"), (beta, "Beta")] {
        let mut tx = pool.begin().await.unwrap();
        set_transaction_tenant(&mut tx, tenant).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT title FROM records")
                .fetch_one(&mut *tx)
                .await
                .unwrap(),
            expected
        );
        assert_eq!(
            sqlx::query("UPDATE records SET title='attack' WHERE tenant_id<>$1")
                .bind(tenant)
                .execute(&mut *tx)
                .await
                .unwrap()
                .rows_affected(),
            0
        );
        tx.commit().await.unwrap();
        let context: Option<String> =
            sqlx::query_scalar("SELECT NULLIF(current_setting('app.tenant_id',true),'')")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(context, None);
        assert_eq!(
            sqlx::query_scalar::<_, i32>("SELECT pg_backend_pid()")
                .fetch_one(&pool)
                .await
                .unwrap(),
            pid
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM records")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
    }
    let mut tx = pool.begin().await.unwrap();
    set_transaction_tenant(&mut tx, alpha).await.unwrap();
    assert!(
        sqlx::query("INSERT INTO records VALUES($1,$2,'forged')")
            .bind(Uuid::new_v4())
            .bind(beta)
            .execute(&mut *tx)
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM records")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    pool.close().await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP SCHEMA {schema} CASCADE; DROP ROLE {role}"
    )))
    .execute(&admin)
    .await
    .unwrap();
}
