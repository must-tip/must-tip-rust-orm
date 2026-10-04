//! Explicit model-state tracking for Django-like dirty-field updates without injecting hidden
//! mutable state into user model structs.

use serde_json::{Map, Value};
use sqlx::{FromRow, postgres::PgRow, sqlite::SqliteRow};
use std::{
    collections::BTreeSet,
    ops::{Deref, DerefMut},
};

use crate::{
    ComparisonOperator, DbValue, Entity, ModelError, ModelManager, Predicate, QueryExecutor,
    QuerySet, SqliteModelManager, SqliteTransactionExecutor, TransactionExecutor, effective_schema,
};

#[derive(Debug, Clone)]
pub struct Tracked<T>
where
    T: Entity,
{
    model: T,
    original: std::collections::HashMap<String, Value>,
}

impl<T> Tracked<T>
where
    T: Entity,
{
    pub fn new(model: T) -> Self {
        let original = model.to_map();
        Self { model, original }
    }

    pub fn dirty_fields(&self) -> BTreeSet<String> {
        let current = self.model.to_map();
        let mut fields = BTreeSet::new();
        for (name, value) in &current {
            if self.original.get(name) != Some(value) {
                fields.insert(name.clone());
            }
        }
        for name in self.original.keys() {
            if !current.contains_key(name) {
                fields.insert(name.clone());
            }
        }
        fields
    }

    pub fn is_dirty(&self) -> bool {
        !self.dirty_fields().is_empty()
    }

    pub fn dirty_patch(&self) -> Result<Map<String, Value>, ModelError> {
        let current = self.model.to_map();
        let schema = T::schema();
        let mut patch = Map::new();
        for field in self.dirty_fields() {
            if field == T::primary_key()
                || field == "created_at"
                || field == "updated_at"
                || T::version_column().is_some_and(|version| version == field)
                || T::tenant_column().is_some_and(|tenant| tenant == field)
                || T::soft_delete_column().is_some_and(|deleted| deleted == field)
            {
                return Err(ModelError::InvalidQuery(format!(
                    "framework-owned tracked field {field} cannot be changed"
                )));
            }
            let spec = schema.get(&field).ok_or_else(|| {
                ModelError::Conversion(format!("tracked model emitted unknown field: {field}"))
            })?;
            let value = current.get(&field).cloned().unwrap_or(Value::Null);
            if value.is_null() && !spec.nullable {
                return Err(ModelError::Validation(format!(
                    "field {field} cannot be null"
                )));
            }
            patch.insert(field, value);
        }
        Ok(patch)
    }

    pub fn mark_clean(&mut self) {
        self.original = self.model.to_map();
    }

    pub fn into_inner(self) -> T {
        self.model
    }

    pub fn inner(&self) -> &T {
        &self.model
    }

    pub fn inner_mut(&mut self) -> &mut T {
        &mut self.model
    }
}

impl<T> Tracked<T>
where
    T: Entity + Clone + for<'r> FromRow<'r, PgRow> + Send + Sync + Unpin,
{
    /// Persist dirty fields on PostgreSQL, using the originally loaded concurrency version.
    pub async fn save(&mut self, manager: &ModelManager<T>) -> Result<T, ModelError> {
        self.model
            .validate_model()
            .map_err(ModelError::Validation)?;
        if self.dirty_patch()?.is_empty() {
            return Ok(self.model.clone());
        }
        let mut tx = manager.pool().begin().await?;
        let saved = self
            .persist_with(&mut TransactionExecutor::new(&mut tx))
            .await?;
        tx.commit().await?;
        self.model = saved.clone();
        self.mark_clean();
        Ok(saved)
    }
}
impl<T> Tracked<T>
where
    T: Entity + Clone + for<'r> FromRow<'r, SqliteRow> + Send + Sync + Unpin,
{
    /// Persist dirty fields on SQLite in one transaction. A stale version returns
    /// `ModelError::ConcurrentModification`; a clean save does not increment the version.
    pub async fn save_sqlite(&mut self, manager: &SqliteModelManager<T>) -> Result<T, ModelError> {
        self.model
            .validate_model()
            .map_err(ModelError::Validation)?;
        if self.dirty_patch()?.is_empty() {
            return Ok(self.model.clone());
        }
        let mut tx = manager.pool().begin().await?;
        let saved = self
            .persist_with(&mut SqliteTransactionExecutor::new(&mut tx))
            .await?;
        tx.commit().await?;
        self.model = saved.clone();
        self.mark_clean();
        Ok(saved)
    }
}
impl<T> Tracked<T>
where
    T: Entity + Clone + Send + Sync + Unpin,
{
    async fn persist_with<E: QueryExecutor>(&self, executor: &mut E) -> Result<T, ModelError>
    where
        T: for<'r> FromRow<'r, E::Row>,
    {
        self.model
            .validate_model()
            .map_err(ModelError::Validation)?;
        let patch = self.dirty_patch()?;
        if patch.is_empty() {
            return Ok(self.model.clone());
        }

        let model_map = self.model.to_map();
        let schema = effective_schema::<T>();
        let primary_key = T::primary_key();
        let pk_spec = schema
            .get(primary_key)
            .ok_or_else(|| ModelError::MissingField(primary_key))?;
        let pk_json = model_map
            .get(primary_key)
            .ok_or_else(|| ModelError::MissingField(primary_key))?;
        let pk_value = DbValue::from_json_for_column(pk_spec, pk_json)?;

        let tenant_value = match T::tenant_column() {
            Some(tenant_column) => {
                let spec = schema
                    .get(tenant_column)
                    .ok_or_else(|| ModelError::MissingField(tenant_column))?;
                let value = model_map
                    .get(tenant_column)
                    .ok_or_else(|| ModelError::MissingField(tenant_column))?;
                Some(DbValue::from_json_for_column(spec, value)?)
            }
            None => None,
        };

        let expected_version = T::version_column()
            .map(|version_column| original_version(&self.original, version_column))
            .transpose()?;

        let mut update = QuerySet::<T, _>::from_executor(&mut *executor);
        if let Some(tenant) = tenant_value.clone() {
            update = update.for_tenant(tenant);
        }
        update = update.filter(Predicate::Comparison {
            field: primary_key.to_owned(),
            operator: ComparisonOperator::Eq,
            value: pk_value.clone(),
        });

        let affected = match expected_version {
            Some(version) => update.update_versioned(version, patch).await?,
            None => update.update(patch).await?,
        };
        if affected != 1 {
            return Err(if affected == 0 {
                ModelError::NotFound
            } else {
                ModelError::Conflict(format!(
                    "tracked save matched {affected} rows for {} primary key",
                    T::table_name()
                ))
            });
        }

        let mut refresh = QuerySet::<T, _>::from_executor(&mut *executor);
        if let Some(tenant) = tenant_value {
            refresh = refresh.for_tenant(tenant);
        }
        let saved = refresh
            .filter(Predicate::Comparison {
                field: primary_key.to_owned(),
                operator: ComparisonOperator::Eq,
                value: pk_value,
            })
            .get()
            .await?;
        Ok(saved)
    }
}

fn original_version(
    original: &std::collections::HashMap<String, Value>,
    version_column: &'static str,
) -> Result<i64, ModelError> {
    let value = original
        .get(version_column)
        .ok_or_else(|| ModelError::MissingField(version_column))?;
    match value {
        Value::Number(value) => value.as_i64().ok_or_else(|| {
            ModelError::Conversion(format!("version field {version_column} must be an integer"))
        }),
        Value::String(value) => value.parse::<i64>().map_err(|_| {
            ModelError::Conversion(format!("version field {version_column} must be an integer"))
        }),
        _ => Err(ModelError::Conversion(format!(
            "version field {version_column} must be an integer"
        ))),
    }
}

impl<T> Deref for Tracked<T>
where
    T: Entity,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.model
    }
}

impl<T> DerefMut for Tracked<T>
where
    T: Entity,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ColumnSpec;
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct Demo {
        id: i64,
        name: String,
        note: Option<String>,
    }

    impl Entity for Demo {
        fn table_name() -> &'static str {
            "tracked_demo"
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
                    "note".into(),
                    ColumnSpec {
                        sql_type: "TEXT".into(),
                        nullable: true,
                        unique: false,
                        primary_key: false,
                        default: None,
                        encrypted: false,
                    },
                ),
            ])
        }
        fn to_map(&self) -> HashMap<String, Value> {
            HashMap::from([
                ("id".into(), serde_json::json!(self.id)),
                ("name".into(), serde_json::json!(self.name)),
                ("note".into(), serde_json::json!(self.note)),
            ])
        }
    }

    #[test]
    fn tracked_model_reports_only_changed_fields() {
        let mut tracked = Tracked::new(Demo {
            id: 1,
            name: "a".into(),
            note: None,
        });
        tracked.name = "b".into();
        assert_eq!(tracked.dirty_fields(), BTreeSet::from(["name".to_owned()]));
        let patch = tracked
            .dirty_patch()
            .unwrap_or_else(|error| panic!("patch: {error}"));
        assert_eq!(
            patch,
            Map::from_iter([("name".into(), Value::String("b".into()))])
        );
        tracked.mark_clean();
        assert!(!tracked.is_dirty());
    }
}
