use db_controller::{
    ColumnSpec, Entity, Field, SqliteModelManager, generate_create_table_sql_sqlite,
};
use serde_json::{Value, json};
use sqlx::{FromRow, sqlite::SqlitePoolOptions};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
struct Note {
    id: i64,
    title: String,
    active: bool,
}

impl Entity for Note {
    fn table_name() -> &'static str {
        "notes"
    }
    fn primary_key() -> &'static str {
        "id"
    }

    fn schema() -> HashMap<String, ColumnSpec> {
        HashMap::from([
            (
                "id".to_owned(),
                ColumnSpec {
                    sql_type: "BIGINT".to_owned(),
                    nullable: false,
                    unique: false,
                    primary_key: true,
                    default: None,
                    encrypted: false,
                },
            ),
            (
                "title".to_owned(),
                ColumnSpec {
                    sql_type: "TEXT".to_owned(),
                    nullable: false,
                    unique: false,
                    primary_key: false,
                    default: None,
                    encrypted: false,
                },
            ),
            (
                "active".to_owned(),
                ColumnSpec {
                    sql_type: "BOOLEAN".to_owned(),
                    nullable: false,
                    unique: false,
                    primary_key: false,
                    default: Some("true".to_owned()),
                    encrypted: false,
                },
            ),
        ])
    }

    fn to_map(&self) -> HashMap<String, Value> {
        HashMap::from([
            ("id".to_owned(), json!(self.id)),
            ("title".to_owned(), json!(self.title)),
            ("active".to_owned(), json!(self.active)),
        ])
    }
}

async fn pool() -> sqlx::SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory SQLite")
}

#[tokio::test]
async fn sqlite_queryset_supports_crud_without_postgres() {
    let pool = pool().await;
    let ddl = generate_create_table_sql_sqlite::<Note>();
    sqlx::query(sqlx::AssertSqlSafe(ddl))
        .execute(&pool)
        .await
        .expect("create schema");

    let manager = SqliteModelManager::<Note>::new(pool.clone());
    let created = manager
        .queryset()
        .create(&Note {
            id: 1,
            title: "first".to_owned(),
            active: true,
        })
        .await
        .expect("create note");
    assert_eq!(created.id, 1);

    let count = manager.queryset().count().await.expect("count notes");
    assert_eq!(count, 1);

    let row = manager
        .queryset()
        .filter(Field::<String>::new("title").iexact("FIRST"))
        .get()
        .await
        .expect("case-insensitive lookup");
    assert_eq!(row.title, "first");

    let updated = manager
        .queryset()
        .filter(Field::<i64>::new("id").eq(1_i64))
        .update(serde_json::Map::from_iter([(
            "title".to_owned(),
            json!("updated"),
        )]))
        .await
        .expect("update note");
    assert_eq!(updated, 1);

    let row = manager
        .queryset()
        .filter(Field::<i64>::new("id").eq(1_i64))
        .get()
        .await
        .expect("read updated note");
    assert_eq!(row.title, "updated");

    let deleted = manager
        .queryset()
        .filter(Field::<i64>::new("id").eq(1_i64))
        .hard_delete()
        .await
        .expect("delete note");
    assert_eq!(deleted, 1);
    assert_eq!(
        manager
            .queryset()
            .count()
            .await
            .expect("count after delete"),
        0
    );

    pool.close().await;
}

#[test]
fn sqlite_ddl_maps_postgres_specific_framework_types() {
    let ddl = generate_create_table_sql_sqlite::<Note>();
    assert!(ddl.contains("CREATE TABLE IF NOT EXISTS \"notes\""));
    assert!(!ddl.contains("BIGSERIAL"));
    assert!(!ddl.contains("TIMESTAMPTZ"));
}

#[test]
fn sqlite_ddl_preserves_exact_numeric_values_as_text() {
    struct LedgerAmount;
    impl Entity for LedgerAmount {
        fn table_name() -> &'static str {
            "ledger_amounts"
        }
        fn schema() -> HashMap<String, ColumnSpec> {
            HashMap::from([(
                "amount".to_owned(),
                ColumnSpec {
                    sql_type: "NUMERIC".to_owned(),
                    nullable: false,
                    unique: false,
                    primary_key: false,
                    default: None,
                    encrypted: false,
                },
            )])
        }
        fn to_map(&self) -> HashMap<String, Value> {
            HashMap::new()
        }
    }

    let ddl = generate_create_table_sql_sqlite::<LedgerAmount>();
    assert!(ddl.contains("\"amount\" TEXT"));
    assert!(!ddl.contains("\"amount\" REAL"));
}
