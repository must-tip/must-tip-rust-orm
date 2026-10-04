use db_controller::{
    ColumnSpec, TABLE_REGISTRY, is_safe_default_expression, is_safe_sql_type, quote_identifier,
};
use std::collections::{HashMap, HashSet};
use tracing::warn;

fn ident(value: &str) -> Result<String, String> {
    quote_identifier(value).ok_or_else(|| format!("unsafe SQL identifier: {value}"))
}

fn normalized_type(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase()
        .replace("TIMESTAMP WITH TIME ZONE", "TIMESTAMPTZ")
}

fn normalize_default(value: &str) -> String {
    let mut value = value.trim().to_owned();
    while value.starts_with('(') && value.ends_with(')') && value.len() > 1 {
        value = value[1..value.len() - 1].trim().to_owned();
    }

    for suffix in [
        "::character varying",
        "::text",
        "::jsonb",
        "::json",
        "::boolean",
        "::uuid",
    ] {
        if value.to_ascii_lowercase().ends_with(suffix) {
            value.truncate(value.len() - suffix.len());
            value = value.trim().to_owned();
            break;
        }
    }
    value
}

fn defaults_equal(left: &Option<String>, right: &Option<String>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => normalize_default(left) == normalize_default(right),
        _ => false,
    }
}

fn validate_spec(table: &str, column: &str, spec: &ColumnSpec) -> Result<(), String> {
    if !is_safe_sql_type(&spec.sql_type) {
        return Err(format!(
            "unsupported or unsafe SQL type for {table}.{column}: {}",
            spec.sql_type
        ));
    }
    if let Some(default) = &spec.default
        && !is_safe_default_expression(default)
    {
        return Err(format!("unsafe SQL default for {table}.{column}"));
    }
    Ok(())
}

pub fn diff_schema(
    old: &HashMap<String, HashMap<String, ColumnSpec>>,
    new: &HashMap<String, HashMap<String, ColumnSpec>>,
) -> Result<Vec<String>, String> {
    let mut statements = Vec::new();
    let mut tables = new.keys().cloned().collect::<Vec<_>>();
    tables.sort();

    for table in tables {
        let table_sql = ident(&table)?;
        let new_cols = &new[&table];
        for (name, spec) in new_cols {
            ident(name)?;
            validate_spec(&table, name, spec)?;
        }

        match old.get(&table) {
            None => {
                let registry = TABLE_REGISTRY
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let migration = registry
                    .get(&table)
                    .ok_or_else(|| format!("missing registry entry for new table {table}"))?;
                statements.push(migration.create_sql.clone());
                statements.extend(
                    migration
                        .migrate_sql
                        .lines()
                        .skip(1)
                        .filter(|line| line.trim_start().starts_with("CREATE INDEX"))
                        .map(str::to_owned),
                );
            }
            Some(old_cols) => {
                let mut names = new_cols.keys().cloned().collect::<Vec<_>>();
                names.sort();
                for name in names {
                    let new_spec = &new_cols[&name];
                    let column = ident(&name)?;
                    match old_cols.get(&name) {
                        None => statements.push(format!(
                            "ALTER TABLE {table_sql} ADD COLUMN {column} {};",
                            new_spec.build()
                        )),
                        Some(old_spec) => {
                            validate_spec(&table, &name, old_spec)?;
                            if old_spec.primary_key != new_spec.primary_key
                                || old_spec.unique != new_spec.unique
                            {
                                return Err(format!(
                                    "constraint change for {table}.{name} requires an explicit reviewed migration"
                                ));
                            }
                            if normalized_type(&old_spec.sql_type)
                                != normalized_type(&new_spec.sql_type)
                            {
                                let target_type = normalized_type(&new_spec.sql_type);
                                statements.push(format!(
                                    "ALTER TABLE {table_sql} ALTER COLUMN {column} TYPE {target_type} USING {column}::{target_type};"
                                ));
                            }
                            if old_spec.nullable != new_spec.nullable {
                                statements.push(format!(
                                    "ALTER TABLE {table_sql} ALTER COLUMN {column} {};",
                                    if new_spec.nullable {
                                        "DROP NOT NULL"
                                    } else {
                                        "SET NOT NULL"
                                    }
                                ));
                            }
                            if !defaults_equal(&old_spec.default, &new_spec.default) {
                                match &new_spec.default {
                                    Some(default) => statements.push(format!(
                                        "ALTER TABLE {table_sql} ALTER COLUMN {column} SET DEFAULT {default};"
                                    )),
                                    None => statements.push(format!(
                                        "ALTER TABLE {table_sql} ALTER COLUMN {column} DROP DEFAULT;"
                                    )),
                                }
                            }
                        }
                    }
                }

                let removed: HashSet<_> = old_cols
                    .keys()
                    .filter(|name| !new_cols.contains_key(*name))
                    .collect();
                for column in removed {
                    warn!(table = %table, column = %column, "column exists in database but not model; automatic DROP skipped");
                }
            }
        }
    }

    for table in old.keys().filter(|table| !new.contains_key(*table)) {
        warn!(table = %table, "table exists in database but not model; automatic DROP skipped");
    }
    Ok(statements)
}

pub struct DynamicEntity;

impl DynamicEntity {
    pub fn discover() -> Result<HashMap<String, HashMap<String, ColumnSpec>>, String> {
        let registry = TABLE_REGISTRY
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(registry
            .iter()
            .map(|(table, migration)| (table.clone(), migration.schema.clone()))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(sql_type: &str) -> ColumnSpec {
        ColumnSpec {
            sql_type: sql_type.to_owned(),
            nullable: false,
            unique: false,
            primary_key: false,
            default: None,
            encrypted: false,
        }
    }

    #[test]
    fn removed_columns_are_not_dropped() {
        let old = HashMap::from([(
            "users".to_owned(),
            HashMap::from([("legacy".to_owned(), spec("TEXT"))]),
        )]);
        let new = HashMap::from([("users".to_owned(), HashMap::new())]);
        let sql = diff_schema(&old, &new).unwrap_or_else(|error| panic!("diff: {error}"));
        assert!(
            sql.iter()
                .all(|statement| !statement.contains("DROP COLUMN"))
        );
    }

    #[test]
    fn constraint_mutations_require_explicit_migration() {
        let old_spec = spec("TEXT");
        let mut new_spec = spec("TEXT");
        new_spec.unique = true;
        let old = HashMap::from([(
            "users".to_owned(),
            HashMap::from([("email".to_owned(), old_spec)]),
        )]);
        let new = HashMap::from([(
            "users".to_owned(),
            HashMap::from([("email".to_owned(), new_spec)]),
        )]);
        assert!(diff_schema(&old, &new).is_err());
    }

    #[test]
    fn postgres_default_casts_do_not_create_false_drift() {
        assert!(defaults_equal(
            &Some("'user'::text".to_owned()),
            &Some("'user'".to_owned())
        ));
        assert!(defaults_equal(
            &Some("'{}'::jsonb".to_owned()),
            &Some("'{}'::jsonb".to_owned())
        ));
    }

    #[test]
    fn unsafe_type_or_default_is_rejected_before_sql_generation() {
        let mut bad = spec("TEXT); DROP TABLE users; --");
        assert!(validate_spec("users", "name", &bad).is_err());
        bad.sql_type = "TEXT".to_owned();
        bad.default = Some("false; DROP TABLE users".to_owned());
        assert!(validate_spec("users", "name", &bad).is_err());
    }
}
