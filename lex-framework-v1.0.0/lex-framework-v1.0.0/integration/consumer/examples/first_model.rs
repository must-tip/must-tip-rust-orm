use lex_framework::{
    db_controller::{TextField, UUIDField, sqlx},
    prelude::*,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "notes")]
#[model(global)]
struct Note {
    #[field(primary_key)]
    id: UUIDField,
    title: TextField,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    let notes = Note::objects_sqlite_auto(pool).await?;
    let saved = notes
        .queryset()
        .create(&Note {
            id: UUIDField::new(),
            title: TextField::new("Hello 世界"),
        })
        .await?;
    let loaded = notes.queryset().filter(Note::ID.eq(saved.id)).get().await?;
    assert_eq!(loaded.title.as_str(), "Hello 世界");
    Ok(())
}
