//! Relation metadata validation and eager-loading results.

use super::*;

#[derive(Debug)]
pub struct Joined<T, R> {
    pub parent: T,
    pub related: Option<R>,
}

#[derive(Debug)]
pub struct Prefetched<T, R> {
    pub parents: Vec<T>,
    pub(super) related_by_key: HashMap<String, Vec<R>>,
    pub(super) local_field: &'static str,
}

impl<T, R> Prefetched<T, R>
where
    T: Entity,
{
    pub fn related_for(&self, parent: &T) -> Result<&[R], ModelError> {
        let map = parent.to_map();
        let Some(value) = map.get(self.local_field) else {
            return Err(ModelError::Conversion(format!(
                "parent model did not emit relation key {}",
                self.local_field
            )));
        };
        if value.is_null() {
            return Ok(&[]);
        }
        let key = canonical_json_key(value)?;
        Ok(self
            .related_by_key
            .get(&key)
            .map(Vec::as_slice)
            .unwrap_or(&[]))
    }
}

pub(super) fn validate_relation<T: Entity, R: Entity>(
    relation: &RelationSpec,
) -> Result<(), ModelError> {
    if relation.target_table != R::table_name() {
        return Err(ModelError::InvalidQuery(format!(
            "relation {} targets {}, not {}",
            relation.field_name,
            relation.target_table,
            R::table_name()
        )));
    }
    let parent_schema = effective_schema::<T>();
    let related_schema = effective_schema::<R>();
    let local = parent_schema.get(relation.local_column).ok_or_else(|| {
        ModelError::InvalidQuery(format!(
            "relation local column does not exist: {}",
            relation.local_column
        ))
    })?;
    let remote = related_schema.get(relation.remote_column).ok_or_else(|| {
        ModelError::InvalidQuery(format!(
            "relation remote column does not exist: {}",
            relation.remote_column
        ))
    })?;
    if !sql_types_compatible(&local.sql_type, &remote.sql_type) {
        return Err(ModelError::InvalidQuery(format!(
            "relation {} has incompatible column types {} ({}) -> {} ({})",
            relation.field_name,
            relation.local_column,
            local.sql_type,
            relation.remote_column,
            remote.sql_type
        )));
    }
    Ok(())
}

pub(super) fn validate_singular_relation_uniqueness<R: Entity>(
    relation: &RelationSpec,
    tenant_scoped: bool,
) -> Result<(), ModelError> {
    if relation.cardinality != RelationCardinality::One {
        return Ok(());
    }
    let schema = effective_schema::<R>();
    let remote = schema.get(relation.remote_column).ok_or_else(|| {
        ModelError::InvalidQuery(format!(
            "relation remote column does not exist: {}",
            relation.remote_column
        ))
    })?;
    if remote.primary_key || remote.unique {
        return Ok(());
    }

    let tenant_column = R::tenant_column();
    let constrained = R::unique_constraints().iter().any(|constraint| {
        if tenant_scoped {
            match tenant_column {
                Some(tenant) => {
                    constraint.columns.len() == 2
                        && constraint.columns.contains(&tenant)
                        && constraint.columns.contains(&relation.remote_column)
                }
                None => {
                    constraint.columns.len() == 1 && constraint.columns[0] == relation.remote_column
                }
            }
        } else {
            constraint.columns.len() == 1 && constraint.columns[0] == relation.remote_column
        }
    });
    if constrained {
        return Ok(());
    }

    Err(ModelError::InvalidQuery(format!(
        "singular relation {} requires {}.{} to be unique{}",
        relation.field_name,
        R::table_name(),
        relation.remote_column,
        if tenant_scoped && tenant_column.is_some() {
            " within the related tenant scope"
        } else {
            " across the selected scope"
        }
    )))
}

pub(super) fn hydrate_relation_json<R>(value: Value) -> Result<R, ModelError>
where
    R: Entity + DeserializeOwned,
{
    let mut value = value;
    if let Value::Object(object) = &mut value {
        for relation in R::relations() {
            object
                .entry(relation.field_name.to_owned())
                .or_insert_with(|| {
                    if relation.is_many() {
                        json!({"related_ids": []})
                    } else {
                        Value::Null
                    }
                });
        }
    }
    serde_json::from_value(value).map_err(ModelError::from)
}

pub(super) fn json_object_sql<T: Entity>(
    alias: &str,
    dialect: DatabaseDialect,
) -> Result<String, ModelError> {
    let mut columns = T::schema().into_iter().collect::<Vec<_>>();
    columns.sort_by(|left, right| left.0.cmp(&right.0));
    if columns.is_empty() {
        return Ok("'{}'::jsonb".to_owned());
    }
    let mut pairs = Vec::with_capacity(columns.len() * 2);
    for (column, specification) in columns {
        let quoted = qualified_ident(alias, &column)?;
        pairs.push(format!("'{column}'"));
        let sql_type = specification.sql_type.trim().to_ascii_uppercase();
        let value = if sql_type.starts_with("NUMERIC") || sql_type.starts_with("DECIMAL") {
            // JSON numeric decoding would round high-precision decimals through f64.
            format!("CAST({quoted} AS TEXT)")
        } else if dialect.is_sqlite() && matches!(sql_type.as_str(), "BOOLEAN" | "BOOL") {
            format!(
                "json(CASE WHEN {quoted} IS NULL THEN 'null' WHEN {quoted} THEN 'true' ELSE 'false' END)"
            )
        } else if dialect.is_sqlite() && matches!(sql_type.as_str(), "JSON" | "JSONB") {
            format!("json({quoted})")
        } else if sql_type == "BYTEA" {
            return Err(ModelError::InvalidQuery("select_related JSON hydration does not support binary columns; use prefetch_related for typed row decoding".into()));
        } else {
            quoted
        };
        pairs.push(value);
    }
    Ok(format!("jsonb_build_object({})", pairs.join(", ")))
}
