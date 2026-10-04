use db_controller::{DateTimeField, Entity, ManyToMany, TextField, UUIDField};
use orm::Model;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "derive_examples")]
#[model(ordering = "-name")]
struct DeriveExample {
    #[field(primary_key)]
    id: UUIDField,

    #[field(unique, index, not_empty, max_length = 100)]
    name: TextField,

    #[field(soft_delete)]
    deleted_at: Option<DateTimeField>,

    #[relation(model = "DeriveExample", local = "id", remote = "id", many)]
    related: ManyToMany<UUIDField>,
}

fn assert_pg_row_compatible<T>()
where
    T: Entity
        + for<'row> db_controller::sqlx::FromRow<'row, db_controller::sqlx::postgres::PgRow>
        + Send
        + Sync
        + Unpin,
{
}

#[test]
fn derive_generates_repository_compatible_contract() {
    assert_pg_row_compatible::<DeriveExample>();

    let schema = DeriveExample::schema();
    assert!(schema.get("id").is_some_and(|column| column.primary_key));
    assert!(schema.get("name").is_some_and(|column| column.unique));
    assert!(
        schema
            .get("deleted_at")
            .is_some_and(|column| column.nullable)
    );
    assert!(!schema.contains_key("related"));
    assert_eq!(DeriveExample::primary_key(), "id");
    assert_eq!(DeriveExample::indexes().len(), 1);
    assert_eq!(DeriveExample::typed_indexes().len(), 1);
    assert_eq!(DeriveExample::typed_indexes()[0].columns, &["name"]);
    assert_eq!(DeriveExample::default_ordering(), vec!["-name"]);
    assert_eq!(DeriveExample::soft_delete_column(), Some("deleted_at"));
    assert_eq!(DeriveExample::NAME.name(), "name");

    let constraints = DeriveExample::unique_constraints();
    assert_eq!(constraints.len(), 1);
    assert_eq!(constraints[0].columns, &["name"]);

    let relation = DeriveExample::related_relation();
    assert_eq!(relation.field_name, "related");
    assert!(relation.is_many());
    assert_eq!(relation.target_table, "derive_examples");
}

#[test]
fn derive_validation_and_relation_projection_are_deterministic() {
    let model = DeriveExample {
        id: UUIDField::new(),
        name: TextField::new("valid-name"),
        deleted_at: None,
        related: ManyToMany::default(),
    };
    assert!(model.validate().is_ok());
    assert!(model.full_clean().is_ok());
    assert!(model.related.is_empty());

    let invalid = DeriveExample {
        name: TextField::new("   "),
        ..model
    };
    assert!(invalid.validate().is_err());
}

#[tokio::test]
async fn derive_exposes_django_style_manager_entry_point() {
    let pool = db_controller::sqlx::PgPool::connect_lazy("postgresql://localhost/test")
        .unwrap_or_else(|error| panic!("lazy pool failed: {error}"));
    let manager = DeriveExample::objects(pool);
    let preview = manager
        .queryset()
        .preview()
        .unwrap_or_else(|error| panic!("preview: {error}"));
    assert!(preview.sql.contains("derive_examples"));
}
