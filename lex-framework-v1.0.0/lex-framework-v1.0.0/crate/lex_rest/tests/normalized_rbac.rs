use lex_rest::rest_framework::permissions::{PermissionRepository, RbacPermissionRepository};
#[cfg(feature = "postgres-integration")]
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use uuid::Uuid;

enum Database {
    Sqlite(sqlx::SqlitePool),
    #[cfg(feature = "postgres-integration")]
    Postgres(sqlx::PgPool),
}

impl Database {
    async fn execute(&self, sql: &str, values: &[Uuid]) -> Result<(), sqlx::Error> {
        match self {
            #[cfg(feature = "postgres-integration")]
            Self::Postgres(pool) => {
                let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
                for value in values {
                    query = query.bind(value.to_string());
                }
                query.execute(pool).await?;
            }
            Self::Sqlite(pool) => {
                let sql = sql.replace("::uuid", "");
                let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
                for value in values {
                    query = query.bind(value.to_string());
                }
                query.execute(pool).await?;
            }
        }
        Ok(())
    }
    fn repository(&self) -> RbacPermissionRepository {
        match self {
            #[cfg(feature = "postgres-integration")]
            Self::Postgres(pool) => RbacPermissionRepository::postgres(pool.clone()),
            Self::Sqlite(pool) => RbacPermissionRepository::sqlite(pool.clone()),
        }
    }
}

async fn a_tenant_role_cannot_cross_tenants_and_removed_grants_take_effect(db: Database) {
    let [
        user,
        alpha,
        beta,
        member,
        role,
        other_role,
        permission,
        assignment,
        grant,
    ] = std::array::from_fn::<_, 9, _>(|i| Uuid::from_u128(i as u128 + 1));
    db.execute("INSERT INTO users(id,name,email,password) VALUES ($1::uuid,'Alice','alice@example.test','test-only')", &[user]).await.unwrap();
    db.execute(
        "INSERT INTO tenants(id,name) VALUES ($1::uuid,'Alpha'),($2::uuid,'Beta')",
        &[alpha, beta],
    )
    .await
    .unwrap();
    db.execute(
        "INSERT INTO memberships(id,user_id,tenant_id) VALUES ($1::uuid,$2::uuid,$3::uuid)",
        &[member, user, alpha],
    )
    .await
    .unwrap();
    assert!(
        db.execute(
            "INSERT INTO memberships(id,user_id,tenant_id) VALUES ($1::uuid,$2::uuid,$3::uuid)",
            &[Uuid::new_v4(), user, alpha]
        )
        .await
        .is_err()
    );
    assert!(
        db.execute(
            "INSERT INTO memberships(id,user_id,tenant_id) VALUES ($1::uuid,$2::uuid,$3::uuid)",
            &[Uuid::new_v4(), Uuid::new_v4(), alpha]
        )
        .await
        .is_err()
    );
    db.execute("INSERT INTO tenant_roles(id,tenant_id,name) VALUES ($1::uuid,$2::uuid,'viewer'),($3::uuid,$4::uuid,'viewer')", &[role,alpha,other_role,beta]).await.unwrap();
    assert!(
        db.execute(
            "INSERT INTO tenant_roles(id,tenant_id,name) VALUES ($1::uuid,$2::uuid,'viewer')",
            &[Uuid::new_v4(), alpha]
        )
        .await
        .is_err()
    );
    db.execute(
        "INSERT INTO permission_definitions(id,name) VALUES ($1::uuid,'projects.read')",
        &[permission],
    )
    .await
    .unwrap();
    assert!(
        db.execute(
            "INSERT INTO permission_definitions(id,name) VALUES ($1::uuid,'projects.read')",
            &[Uuid::new_v4()]
        )
        .await
        .is_err()
    );
    let assignment_sql = "INSERT INTO membership_roles(id,tenant_id,membership_id,role_id) VALUES($1::uuid,$2::uuid,$3::uuid,$4::uuid)";
    assert!(
        db.execute(assignment_sql, &[assignment, alpha, member, other_role])
            .await
            .is_err()
    );
    assert!(
        db.execute(assignment_sql, &[assignment, beta, member, other_role])
            .await
            .is_err()
    );
    db.execute(assignment_sql, &[assignment, alpha, member, role])
        .await
        .unwrap();
    assert!(
        db.execute(assignment_sql, &[Uuid::new_v4(), alpha, member, role])
            .await
            .is_err()
    );
    let grant_sql = "INSERT INTO role_permissions(id,tenant_id,role_id,permission_id) VALUES($1::uuid,$2::uuid,$3::uuid,$4::uuid)";
    assert!(
        db.execute(grant_sql, &[grant, beta, role, permission])
            .await
            .is_err()
    );
    assert!(
        db.execute(grant_sql, &[grant, alpha, role, Uuid::new_v4()])
            .await
            .is_err()
    );
    db.execute(grant_sql, &[grant, alpha, role, permission])
        .await
        .unwrap();
    assert!(
        db.execute(grant_sql, &[Uuid::new_v4(), alpha, role, permission])
            .await
            .is_err()
    );
    let repository = db.repository();
    assert!(repository.membership(user, beta).await.unwrap().is_none());
    assert_eq!(
        repository
            .permission_names(member, alpha)
            .await
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>(),
        ["projects.read"]
    );
    assert!(
        repository
            .permission_names(member, beta)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        db.execute("DELETE FROM tenant_roles WHERE id=$1::uuid", &[role])
            .await
            .is_err()
    );
    assert!(
        db.execute(
            "UPDATE tenant_roles SET tenant_id=$1::uuid WHERE id=$2::uuid",
            &[beta, role]
        )
        .await
        .is_err()
    );
    db.execute("DELETE FROM role_permissions WHERE id=$1::uuid", &[grant])
        .await
        .unwrap();
    assert!(
        repository
            .permission_names(member, alpha)
            .await
            .unwrap()
            .is_empty()
    );
    db.execute(grant_sql, &[grant, alpha, role, permission])
        .await
        .unwrap();
    db.execute(
        "UPDATE memberships SET deleted_at=CURRENT_TIMESTAMP WHERE id=$1::uuid",
        &[member],
    )
    .await
    .unwrap();
    assert!(repository.membership(user, alpha).await.unwrap().is_none());
    assert!(
        repository
            .permission_names(member, alpha)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn sqlite_rejects_invalid_rbac_state_and_revokes_permissions_immediately() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .in_memory(true)
                .foreign_keys(true),
        )
        .await
        .unwrap();
    migration::rbac::install_rbac_sqlite(&pool).await.unwrap();
    migration::rbac::install_rbac_sqlite(&pool).await.unwrap();
    a_tenant_role_cannot_cross_tenants_and_removed_grants_take_effect(Database::Sqlite(pool)).await;
}

#[tokio::test]
async fn sqlite_rbac_installer_rejects_existing_bridge_without_assignment_uniqueness() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .in_memory(true)
                .foreign_keys(true),
        )
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TABLE membership_roles(id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL, membership_id TEXT NOT NULL, role_id TEXT NOT NULL, FOREIGN KEY(membership_id,tenant_id) REFERENCES memberships(id,tenant_id) ON DELETE RESTRICT, FOREIGN KEY(role_id,tenant_id) REFERENCES tenant_roles(id,tenant_id) ON DELETE RESTRICT)")
        .execute(&pool).await.unwrap();
    assert!(
        migration::rbac::install_rbac_sqlite(&pool).await.is_err(),
        "existing bridge tables must enforce assignment uniqueness before installation succeeds"
    );
}

#[cfg(feature = "postgres-integration")]
#[tokio::test]
async fn postgres_rejects_invalid_rbac_state_and_revokes_permissions_immediately() {
    assert_eq!(
        std::env::var("CONFIRM_DISPOSABLE_DATABASE").as_deref(),
        Ok("yes")
    );
    let url = std::env::var("TEST_DATABASE_URL").expect("dedicated test database required");
    let admin = sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("lex_rbac_{}", Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .unwrap();
    let search_path = format!("SET search_path TO {schema}");
    let pool = PgPoolOptions::new()
        .after_connect(move |connection, _| {
            let sql = search_path.clone();
            Box::pin(async move {
                sqlx::query(sqlx::AssertSqlSafe(sql))
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    migration::rbac::install_rbac_postgres(&pool).await.unwrap();
    migration::rbac::install_rbac_postgres(&pool).await.unwrap();
    a_tenant_role_cannot_cross_tenants_and_removed_grants_take_effect(Database::Postgres(
        pool.clone(),
    ))
    .await;
    let unique_name: String = sqlx::query_scalar("SELECT conname FROM pg_constraint WHERE conrelid = 'role_permissions'::regclass AND contype = 'u'")
        .fetch_one(&pool).await.unwrap();
    let quoted = db_controller::quote_identifier(&unique_name).unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "ALTER TABLE role_permissions DROP CONSTRAINT {quoted}"
    )))
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        migration::rbac::install_rbac_postgres(&pool).await.is_err(),
        "existing grant tables without uniqueness must not be certified"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "ALTER TABLE role_permissions ADD CONSTRAINT {quoted} UNIQUE(role_id,permission_id)"
    )))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql("ALTER TABLE membership_roles DROP CONSTRAINT assignment_role_tenant_fk; ALTER TABLE membership_roles ADD CONSTRAINT assignment_role_tenant_fk FOREIGN KEY(role_id) REFERENCES tenant_roles(id) ON DELETE RESTRICT")
        .execute(&pool).await.unwrap();
    let repair = migration::rbac::install_rbac_postgres(&pool).await;
    assert!(
        repair.is_err(),
        "a named constraint without tenant isolation must not be certified"
    );
    pool.close().await;
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .unwrap();
}

#[cfg(feature = "postgres-integration")]
#[tokio::test]
async fn legacy_rbac_conversion_preserves_data_and_refuses_privilege_escalation() {
    assert_eq!(
        std::env::var("CONFIRM_DISPOSABLE_DATABASE").as_deref(),
        Ok("yes")
    );
    let url = std::env::var("TEST_DATABASE_URL").unwrap();
    let admin = sqlx::PgPool::connect(&url).await.unwrap();
    for inconsistent in [false, true] {
        let schema = format!("lex_legacy_{}", Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .unwrap();
        let setting = format!("SET search_path TO {schema}");
        let pool = PgPoolOptions::new()
            .after_connect(move |connection, _| {
                let sql = setting.clone();
                Box::pin(async move {
                    sqlx::query(sqlx::AssertSqlSafe(sql))
                        .execute(connection)
                        .await?;
                    Ok(())
                })
            })
            .connect(&url)
            .await
            .unwrap();
        for ddl in [
            db_controller::generate_create_table_sql::<models::User>(),
            db_controller::generate_create_table_sql::<models::Tenant>(),
            db_controller::generate_create_table_sql::<models::Membership>(),
            db_controller::generate_create_table_sql::<models::TenantRole>(),
            db_controller::generate_create_table_sql::<models::PermissionDefinition>(),
            db_controller::generate_create_table_sql::<models::TenantRolePermission>(),
        ] {
            sqlx::raw_sql(sqlx::AssertSqlSafe(ddl))
                .execute(&pool)
                .await
                .unwrap();
        }
        let db = Database::Postgres(pool.clone());
        let [alice, bob, tenant, ma, mb, role, read, write] =
            std::array::from_fn::<_, 8, _>(|i| Uuid::from_u128(i as u128 + 100));
        db.execute("INSERT INTO users(id,name,email,password) VALUES($1::uuid,'Alice','alice@test.test','test-only'),($2::uuid,'Bob','bob@test.test','test-only')",&[alice,bob]).await.unwrap();
        db.execute(
            "INSERT INTO tenants(id,name) VALUES($1::uuid,'Alpha')",
            &[tenant],
        )
        .await
        .unwrap();
        db.execute("INSERT INTO memberships(id,user_id,tenant_id) VALUES($1::uuid,$2::uuid,$3::uuid),($4::uuid,$5::uuid,$3::uuid)",&[ma,alice,tenant,mb,bob]).await.unwrap();
        db.execute(
            "INSERT INTO tenant_roles(id,tenant_id,name) VALUES($1::uuid,$2::uuid,'viewer')",
            &[role, tenant],
        )
        .await
        .unwrap();
        db.execute("INSERT INTO permission_definitions(id,name) VALUES($1::uuid,'projects.read'),($2::uuid,'projects.write')",&[read,write]).await.unwrap();
        let grant = "INSERT INTO tenant_role_permissions(membership_id,role_id,permission_id) VALUES($1::uuid,$2::uuid,$3::uuid)";
        db.execute(grant, &[ma, role, read]).await.unwrap();
        db.execute(grant, &[ma, role, read]).await.unwrap(); // source duplicates are preserved and normalized pairs deduplicated
        db.execute(grant, &[mb, role, if inconsistent { write } else { read }])
            .await
            .unwrap();
        let result = migration::rbac::normalize_legacy_rbac_postgres(&pool).await;
        assert_eq!(result.is_err(), inconsistent);
        let source_count: i64 = sqlx::query_scalar("SELECT count(*) FROM tenant_role_permissions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(source_count, 3);
        if inconsistent {
            let table: Option<String> =
                sqlx::query_scalar("SELECT to_regclass('membership_roles')::text")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert!(
                table.is_none(),
                "failed conversion must roll back newly installed tables"
            );
        } else {
            let assignments: i64 = sqlx::query_scalar("SELECT count(*) FROM membership_roles")
                .fetch_one(&pool)
                .await
                .unwrap();
            let permissions: i64 = sqlx::query_scalar("SELECT count(*) FROM role_permissions")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!((assignments, permissions), (2, 1));
            assert_eq!(
                db.repository()
                    .permission_names(ma, tenant)
                    .await
                    .unwrap()
                    .into_iter()
                    .collect::<Vec<_>>(),
                ["projects.read"]
            );
            assert!(
                migration::rbac::normalize_legacy_rbac_postgres(&pool)
                    .await
                    .is_err(),
                "repeated conversion cannot merge live authorization state"
            );
        }
        pool.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&admin)
            .await
            .unwrap();
    }
}
