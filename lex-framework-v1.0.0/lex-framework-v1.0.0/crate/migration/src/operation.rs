//! Typed migration operations with explicit forward and reverse SQL. Raw SQL migrations remain
//! supported, but application-generated migrations should prefer these operations so identifiers
//! and schema fragments are validated before SQL is emitted. Destructive operations that depend on
//! external indexes/constraints still require reviewed companion operations to make rollback exact.

use anyhow::{Result, anyhow};
pub use db_controller::IndexMethod;
use db_controller::{
    ColumnSpec, Entity, OnDelete, RelationCardinality, effective_schema, is_safe_check_expression,
    is_safe_default_expression, is_safe_sql_type, quote_identifier,
};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationOperation {
    CreateTable {
        table: String,
        columns: HashMap<String, ColumnSpec>,
    },
    DropTable {
        table: String,
        columns: HashMap<String, ColumnSpec>,
    },
    AddColumn {
        table: String,
        column: String,
        spec: ColumnSpec,
    },
    DropColumn {
        table: String,
        column: String,
        spec: ColumnSpec,
    },
    RenameColumn {
        table: String,
        from: String,
        to: String,
    },
    RenameTable {
        from: String,
        to: String,
    },
    AlterColumn {
        table: String,
        column: String,
        from: ColumnSpec,
        to: ColumnSpec,
    },
    CreateIndex {
        name: String,
        table: String,
        columns: Vec<String>,
        unique: bool,
        method: IndexMethod,
    },
    DropIndex {
        name: String,
        table: String,
        columns: Vec<String>,
        unique: bool,
        method: IndexMethod,
    },
    AddUniqueConstraint {
        name: String,
        table: String,
        columns: Vec<String>,
    },
    DropUniqueConstraint {
        name: String,
        table: String,
        columns: Vec<String>,
    },
    AddCheckConstraint {
        name: String,
        table: String,
        expression: String,
    },
    DropCheckConstraint {
        name: String,
        table: String,
        expression: String,
    },
    AddForeignKey {
        name: String,
        table: String,
        local_column: String,
        target_table: String,
        remote_column: String,
        on_delete: OnDelete,
    },
    DropForeignKey {
        name: String,
        table: String,
        local_column: String,
        target_table: String,
        remote_column: String,
        on_delete: OnDelete,
    },
    RunSql {
        up: String,
        down: String,
    },
}

/// Build deterministic schema operations from the canonical model metadata.
///
/// Singular relations become database foreign keys. Reverse/many relation projections are read
/// metadata and do not create columns or constraints on the parent table. Target tables must be
/// ordered through the migration dependency graph (except self-relations).
pub fn operations_for_model<T: Entity>() -> Result<Vec<MigrationOperation>> {
    let table = T::table_name().to_owned();
    let schema = effective_schema::<T>();
    let mut operations = vec![MigrationOperation::CreateTable {
        table: table.clone(),
        columns: schema.clone(),
    }];

    let mut unique_constraints = T::unique_constraints();
    unique_constraints.sort_by(|left, right| left.name.cmp(right.name));
    for constraint in unique_constraints {
        if constraint.columns.is_empty() {
            return Err(anyhow!(
                "unique constraint {} must contain at least one column",
                constraint.name
            ));
        }
        let _ = ident(constraint.name)?;
        for column in constraint.columns {
            if !schema.contains_key(*column) {
                return Err(anyhow!(
                    "unique constraint {} references unknown column {}.{}",
                    constraint.name,
                    T::table_name(),
                    column
                ));
            }
            let _ = ident(column)?;
        }

        // Single-column uniqueness is already represented by ColumnSpec and therefore appears in
        // CREATE TABLE. Model-level composite constraints need their own deterministic operation.
        if constraint.columns.len() == 1
            && schema
                .get(constraint.columns[0])
                .is_some_and(|column| column.unique || column.primary_key)
        {
            continue;
        }

        operations.push(MigrationOperation::AddUniqueConstraint {
            name: constraint.name.to_owned(),
            table: table.clone(),
            columns: constraint
                .columns
                .iter()
                .map(|column| (*column).to_owned())
                .collect(),
        });
    }

    let mut check_constraints = T::check_constraints();
    check_constraints.sort_by(|left, right| left.name.cmp(right.name));
    for constraint in check_constraints {
        let _ = ident(constraint.name)?;
        if !is_safe_check_expression(constraint.expression) {
            return Err(anyhow!(
                "check constraint {} contains an unsafe SQL expression",
                constraint.name
            ));
        }
        operations.push(MigrationOperation::AddCheckConstraint {
            name: constraint.name.to_owned(),
            table: table.clone(),
            expression: constraint.expression.trim().to_owned(),
        });
    }

    let typed_indexes = T::typed_indexes();
    if typed_indexes.is_empty() && !T::indexes().is_empty() {
        return Err(anyhow!(
            "model {} exposes only legacy raw index SQL; typed model migrations require IndexSpec metadata or an explicit reviewed SQL migration",
            T::table_name()
        ));
    }
    let mut typed_indexes = typed_indexes;
    typed_indexes.sort_by(|left, right| left.name.cmp(right.name));
    for index in typed_indexes {
        let _ = ident(index.name)?;
        if index.columns.is_empty() {
            return Err(anyhow!(
                "index {} must contain at least one column",
                index.name
            ));
        }
        let columns = index
            .columns
            .iter()
            .map(|column| {
                if !schema.contains_key(*column) {
                    return Err(anyhow!(
                        "index {} references unknown column {}.{}",
                        index.name,
                        T::table_name(),
                        column
                    ));
                }
                let _ = ident(column)?;
                Ok((*column).to_owned())
            })
            .collect::<Result<Vec<_>>>()?;
        operations.push(MigrationOperation::CreateIndex {
            name: index.name.to_owned(),
            table: table.clone(),
            columns,
            unique: index.unique,
            method: index.method,
        });
    }

    for relation in T::relations() {
        if relation.cardinality == RelationCardinality::Many {
            continue;
        }
        let local = schema.get(relation.local_column).ok_or_else(|| {
            anyhow!(
                "relation {} references missing local column {}.{}",
                relation.field_name,
                T::table_name(),
                relation.local_column
            )
        })?;
        if relation.on_delete == OnDelete::SetNull && !local.nullable {
            return Err(anyhow!(
                "relation {} uses SET NULL but local column {}.{} is NOT NULL",
                relation.field_name,
                T::table_name(),
                relation.local_column
            ));
        }
        if relation.on_delete == OnDelete::SetDefault && local.default.is_none() {
            return Err(anyhow!(
                "relation {} uses SET DEFAULT but local column {}.{} has no database default",
                relation.field_name,
                T::table_name(),
                relation.local_column
            ));
        }
        let name = foreign_key_name(
            T::table_name(),
            relation.local_column,
            relation.target_table,
        )?;
        operations.push(MigrationOperation::AddForeignKey {
            name,
            table: table.clone(),
            local_column: relation.local_column.to_owned(),
            target_table: relation.target_table.to_owned(),
            remote_column: relation.remote_column.to_owned(),
            on_delete: relation.on_delete,
        });
    }

    Ok(operations)
}

fn foreign_key_name(table: &str, local_column: &str, target_table: &str) -> Result<String> {
    let name = format!("fk_{table}_{local_column}_{target_table}");
    if name.len() > 63 {
        return Err(anyhow!(
            "generated PostgreSQL foreign-key name exceeds 63 bytes: {name}"
        ));
    }
    // Reuse identifier validation so generated metadata never becomes unquoted SQL.
    let _ = ident(&name)?;
    Ok(name)
}

impl MigrationOperation {
    pub fn up_sql(&self) -> Result<Vec<String>> {
        match self {
            Self::CreateTable { table, columns } => Ok(vec![create_table_sql(table, columns)?]),
            Self::DropTable { table, .. } => Ok(vec![format!("DROP TABLE {};", ident(table)?)]),
            Self::AddColumn {
                table,
                column,
                spec,
            } => Ok(vec![format!(
                "ALTER TABLE {} ADD COLUMN {} {};",
                ident(table)?,
                ident(column)?,
                build_spec(spec)?
            )]),
            Self::DropColumn { table, column, .. } => Ok(vec![format!(
                "ALTER TABLE {} DROP COLUMN {};",
                ident(table)?,
                ident(column)?
            )]),
            Self::RenameColumn { table, from, to } => Ok(vec![format!(
                "ALTER TABLE {} RENAME COLUMN {} TO {};",
                ident(table)?,
                ident(from)?,
                ident(to)?
            )]),
            Self::RenameTable { from, to } => Ok(vec![format!(
                "ALTER TABLE {} RENAME TO {};",
                ident(from)?,
                ident(to)?
            )]),
            Self::AlterColumn {
                table,
                column,
                from,
                to,
            } => alter_column_sql(table, column, from, to),
            Self::CreateIndex {
                name,
                table,
                columns,
                unique,
                method,
            } => Ok(vec![create_index_sql(
                name, table, columns, *unique, *method,
            )?]),
            Self::DropIndex { name, .. } => Ok(vec![format!("DROP INDEX {};", ident(name)?)]),
            Self::AddUniqueConstraint {
                name,
                table,
                columns,
            } => Ok(vec![format!(
                "ALTER TABLE {} ADD CONSTRAINT {} UNIQUE ({});",
                ident(table)?,
                ident(name)?,
                column_list(columns)?
            )]),
            Self::DropUniqueConstraint { name, table, .. } => Ok(vec![format!(
                "ALTER TABLE {} DROP CONSTRAINT {};",
                ident(table)?,
                ident(name)?
            )]),
            Self::AddCheckConstraint {
                name,
                table,
                expression,
            } => Ok(vec![add_check_constraint_sql(name, table, expression)?]),
            Self::DropCheckConstraint { name, table, .. } => Ok(vec![format!(
                "ALTER TABLE {} DROP CONSTRAINT {};",
                ident(table)?,
                ident(name)?
            )]),
            Self::AddForeignKey {
                name,
                table,
                local_column,
                target_table,
                remote_column,
                on_delete,
            } => Ok(vec![foreign_key_sql(
                name,
                table,
                local_column,
                target_table,
                remote_column,
                *on_delete,
            )?]),
            Self::DropForeignKey { name, table, .. } => Ok(vec![format!(
                "ALTER TABLE {} DROP CONSTRAINT {};",
                ident(table)?,
                ident(name)?
            )]),
            Self::RunSql { up, down } => {
                validate_raw_pair(up, down)?;
                Ok(vec![up.clone()])
            }
        }
    }

    pub fn down_sql(&self) -> Result<Vec<String>> {
        match self {
            Self::CreateTable { table, .. } => Ok(vec![format!("DROP TABLE {};", ident(table)?)]),
            Self::DropTable { table, columns } => Ok(vec![create_table_sql(table, columns)?]),
            Self::AddColumn { table, column, .. } => Ok(vec![format!(
                "ALTER TABLE {} DROP COLUMN {};",
                ident(table)?,
                ident(column)?
            )]),
            Self::DropColumn {
                table,
                column,
                spec,
            } => Ok(vec![format!(
                "ALTER TABLE {} ADD COLUMN {} {};",
                ident(table)?,
                ident(column)?,
                build_spec(spec)?
            )]),
            Self::RenameColumn { table, from, to } => Ok(vec![format!(
                "ALTER TABLE {} RENAME COLUMN {} TO {};",
                ident(table)?,
                ident(to)?,
                ident(from)?
            )]),
            Self::RenameTable { from, to } => Ok(vec![format!(
                "ALTER TABLE {} RENAME TO {};",
                ident(to)?,
                ident(from)?
            )]),
            Self::AlterColumn {
                table,
                column,
                from,
                to,
            } => alter_column_sql(table, column, to, from),
            Self::CreateIndex { name, .. } => Ok(vec![format!("DROP INDEX {};", ident(name)?)]),
            Self::DropIndex {
                name,
                table,
                columns,
                unique,
                method,
            } => Ok(vec![create_index_sql(
                name, table, columns, *unique, *method,
            )?]),
            Self::AddUniqueConstraint { name, table, .. } => Ok(vec![format!(
                "ALTER TABLE {} DROP CONSTRAINT {};",
                ident(table)?,
                ident(name)?
            )]),
            Self::DropUniqueConstraint {
                name,
                table,
                columns,
            } => Ok(vec![format!(
                "ALTER TABLE {} ADD CONSTRAINT {} UNIQUE ({});",
                ident(table)?,
                ident(name)?,
                column_list(columns)?
            )]),
            Self::AddCheckConstraint { name, table, .. } => Ok(vec![format!(
                "ALTER TABLE {} DROP CONSTRAINT {};",
                ident(table)?,
                ident(name)?
            )]),
            Self::DropCheckConstraint {
                name,
                table,
                expression,
            } => Ok(vec![add_check_constraint_sql(name, table, expression)?]),
            Self::AddForeignKey { name, table, .. } => Ok(vec![format!(
                "ALTER TABLE {} DROP CONSTRAINT {};",
                ident(table)?,
                ident(name)?
            )]),
            Self::DropForeignKey {
                name,
                table,
                local_column,
                target_table,
                remote_column,
                on_delete,
            } => Ok(vec![foreign_key_sql(
                name,
                table,
                local_column,
                target_table,
                remote_column,
                *on_delete,
            )?]),
            Self::RunSql { up, down } => {
                validate_raw_pair(up, down)?;
                Ok(vec![down.clone()])
            }
        }
    }
}

fn create_table_sql(table: &str, columns: &HashMap<String, ColumnSpec>) -> Result<String> {
    if columns.is_empty() {
        return Err(anyhow!("cannot create a table with no columns"));
    }
    let mut columns = columns.iter().collect::<Vec<_>>();
    columns.sort_by(|left, right| left.0.cmp(right.0));
    let definitions = columns
        .into_iter()
        .map(|(name, spec)| Ok(format!("    {} {}", ident(name)?, build_spec(spec)?)))
        .collect::<Result<Vec<_>>>()?
        .join(",\n");
    Ok(format!(
        "CREATE TABLE {} (\n{definitions}\n);",
        ident(table)?
    ))
}

fn alter_column_sql(
    table: &str,
    column: &str,
    from: &ColumnSpec,
    to: &ColumnSpec,
) -> Result<Vec<String>> {
    let table = ident(table)?;
    let column = ident(column)?;
    if from.primary_key != to.primary_key {
        return Err(anyhow!(
            "AlterColumn cannot change primary-key ownership implicitly; use explicit constraint operations"
        ));
    }
    if from.unique != to.unique {
        return Err(anyhow!(
            "AlterColumn cannot change uniqueness implicitly; use AddUniqueConstraint/DropUniqueConstraint"
        ));
    }
    if from.encrypted != to.encrypted {
        return Err(anyhow!(
            "AlterColumn cannot toggle encryption metadata without an explicit reviewed data migration"
        ));
    }
    if !is_safe_sql_type(&from.sql_type) || !is_safe_sql_type(&to.sql_type) {
        return Err(anyhow!("AlterColumn contains an unsupported SQL type"));
    }
    if from
        .default
        .as_deref()
        .is_some_and(|value| !is_safe_default_expression(value))
        || to
            .default
            .as_deref()
            .is_some_and(|value| !is_safe_default_expression(value))
    {
        return Err(anyhow!(
            "AlterColumn contains an unsafe SQL default expression"
        ));
    }

    let type_changed = !from
        .sql_type
        .trim()
        .eq_ignore_ascii_case(to.sql_type.trim());
    let mut sql = Vec::new();
    // PostgreSQL may be unable to cast an existing default while changing the column type. Drop
    // it first, perform the explicit cast, then restore the target default deterministically.
    if type_changed && from.default.is_some() {
        sql.push(format!(
            "ALTER TABLE {table} ALTER COLUMN {column} DROP DEFAULT;"
        ));
    }
    if type_changed {
        sql.push(format!(
            "ALTER TABLE {table} ALTER COLUMN {column} TYPE {} USING {column}::{};",
            to.sql_type.trim(),
            to.sql_type.trim()
        ));
    }

    if from.nullable != to.nullable {
        sql.push(if to.nullable {
            format!("ALTER TABLE {table} ALTER COLUMN {column} DROP NOT NULL;")
        } else {
            format!("ALTER TABLE {table} ALTER COLUMN {column} SET NOT NULL;")
        });
    }

    if type_changed || from.default != to.default {
        match &to.default {
            Some(default) => sql.push(format!(
                "ALTER TABLE {table} ALTER COLUMN {column} SET DEFAULT {};",
                default.trim()
            )),
            None if !type_changed => sql.push(format!(
                "ALTER TABLE {table} ALTER COLUMN {column} DROP DEFAULT;"
            )),
            None => {}
        }
    }

    if sql.is_empty() {
        return Err(anyhow!("AlterColumn does not change database state"));
    }
    Ok(sql)
}

fn create_index_sql(
    name: &str,
    table: &str,
    columns: &[String],
    unique: bool,
    method: IndexMethod,
) -> Result<String> {
    if columns.is_empty() {
        return Err(anyhow!("index must contain at least one column"));
    }
    Ok(format!(
        "CREATE {}INDEX {} ON {} USING {} ({});",
        if unique { "UNIQUE " } else { "" },
        ident(name)?,
        ident(table)?,
        method.as_sql(),
        column_list(columns)?
    ))
}

fn add_check_constraint_sql(name: &str, table: &str, expression: &str) -> Result<String> {
    if !is_safe_check_expression(expression) {
        return Err(anyhow!("unsafe CHECK constraint expression"));
    }
    Ok(format!(
        "ALTER TABLE {} ADD CONSTRAINT {} CHECK ({});",
        ident(table)?,
        ident(name)?,
        expression.trim()
    ))
}

fn foreign_key_sql(
    name: &str,
    table: &str,
    local_column: &str,
    target_table: &str,
    remote_column: &str,
    on_delete: OnDelete,
) -> Result<String> {
    Ok(format!(
        "ALTER TABLE {} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({}) ON DELETE {};",
        ident(table)?,
        ident(name)?,
        ident(local_column)?,
        ident(target_table)?,
        ident(remote_column)?,
        on_delete.as_sql()
    ))
}

fn column_list(columns: &[String]) -> Result<String> {
    if columns.is_empty() {
        return Err(anyhow!("column list must not be empty"));
    }
    columns
        .iter()
        .map(|column| ident(column))
        .collect::<Result<Vec<_>>>()
        .map(|values| values.join(", "))
}

fn build_spec(spec: &ColumnSpec) -> Result<String> {
    if !is_safe_sql_type(&spec.sql_type) {
        return Err(anyhow!("unsupported SQL type: {}", spec.sql_type));
    }
    if spec
        .default
        .as_deref()
        .is_some_and(|value| !is_safe_default_expression(value))
    {
        return Err(anyhow!("unsafe SQL default expression"));
    }
    Ok(spec.build())
}

fn ident(value: &str) -> Result<String> {
    quote_identifier(value).ok_or_else(|| anyhow!("unsafe SQL identifier: {value}"))
}

fn validate_raw_pair(up: &str, down: &str) -> Result<()> {
    if up.trim().is_empty() || down.trim().is_empty() {
        return Err(anyhow!("RunSql requires non-empty forward and reverse SQL"));
    }
    if up.as_bytes().contains(&0) || down.as_bytes().contains(&0) {
        return Err(anyhow!("RunSql cannot contain NUL bytes"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text() -> ColumnSpec {
        ColumnSpec {
            sql_type: "TEXT".into(),
            nullable: false,
            unique: false,
            primary_key: false,
            default: None,
            encrypted: false,
        }
    }

    #[test]
    fn operations_are_reversible_and_quote_identifiers() {
        let operation = MigrationOperation::AddColumn {
            table: "users".into(),
            column: "display_name".into(),
            spec: text(),
        };
        assert_eq!(
            operation.up_sql().unwrap(),
            vec!["ALTER TABLE \"users\" ADD COLUMN \"display_name\" TEXT NOT NULL;"]
        );
        assert_eq!(
            operation.down_sql().unwrap(),
            vec!["ALTER TABLE \"users\" DROP COLUMN \"display_name\";"]
        );
    }

    #[test]
    fn rename_table_is_reversible() {
        let operation = MigrationOperation::RenameTable {
            from: "legacy_users".into(),
            to: "users".into(),
        };
        assert_eq!(
            operation.up_sql().unwrap(),
            vec!["ALTER TABLE \"legacy_users\" RENAME TO \"users\";"]
        );
        assert_eq!(
            operation.down_sql().unwrap(),
            vec!["ALTER TABLE \"users\" RENAME TO \"legacy_users\";"]
        );
    }

    #[test]
    fn alter_column_is_reversible_and_rejects_constraint_shortcuts() {
        let from = ColumnSpec {
            default: Some("'0'".into()),
            ..text()
        };
        let to = ColumnSpec {
            sql_type: "INTEGER".into(),
            nullable: true,
            default: None,
            ..text()
        };
        let operation = MigrationOperation::AlterColumn {
            table: "users".into(),
            column: "display_name".into(),
            from: from.clone(),
            to: to.clone(),
        };
        let up = operation.up_sql().unwrap();
        assert_eq!(up.len(), 3);
        assert!(up[0].contains("DROP DEFAULT"));
        assert!(up[1].contains("TYPE INTEGER"));
        assert!(up[2].contains("DROP NOT NULL"));
        assert!(!operation.down_sql().unwrap().is_empty());

        let unique = MigrationOperation::AlterColumn {
            table: "users".into(),
            column: "display_name".into(),
            from: text(),
            to: ColumnSpec {
                unique: true,
                ..text()
            },
        };
        assert!(unique.up_sql().is_err());
    }

    #[test]
    fn unsafe_identifiers_are_rejected() {
        let operation = MigrationOperation::AddColumn {
            table: "users;drop".into(),
            column: "name".into(),
            spec: text(),
        };
        assert!(operation.up_sql().is_err());
    }

    #[test]
    fn model_operations_materialize_singular_relations_as_foreign_keys() {
        let operations = operations_for_model::<models::Membership>()
            .unwrap_or_else(|error| panic!("model operations: {error}"));
        assert!(matches!(
            operations.first(),
            Some(MigrationOperation::CreateTable { .. })
        ));
        let foreign_keys = operations
            .iter()
            .filter(|operation| matches!(operation, MigrationOperation::AddForeignKey { .. }))
            .count();
        let indexes = operations
            .iter()
            .filter(|operation| matches!(operation, MigrationOperation::CreateIndex { .. }))
            .count();
        assert_eq!(foreign_keys, 2);
        assert_eq!(indexes, 2);
    }

    struct CompositeUniqueEntity;

    impl Entity for CompositeUniqueEntity {
        fn table_name() -> &'static str {
            "composite_unique_entity"
        }

        fn schema() -> HashMap<String, ColumnSpec> {
            HashMap::from([
                (
                    "tenant_id".to_owned(),
                    ColumnSpec {
                        sql_type: "UUID".to_owned(),
                        nullable: false,
                        unique: false,
                        primary_key: false,
                        default: None,
                        encrypted: false,
                    },
                ),
                (
                    "external_key".to_owned(),
                    ColumnSpec {
                        sql_type: "TEXT".to_owned(),
                        nullable: false,
                        unique: false,
                        primary_key: false,
                        default: None,
                        encrypted: false,
                    },
                ),
            ])
        }

        fn to_map(&self) -> HashMap<String, serde_json::Value> {
            HashMap::new()
        }

        fn unique_constraints() -> Vec<db_controller::UniqueConstraintSpec> {
            vec![db_controller::UniqueConstraintSpec {
                name: "uq_composite_unique_entity_tenant_external",
                columns: &["tenant_id", "external_key"],
            }]
        }

        fn check_constraints() -> Vec<db_controller::CheckConstraintSpec> {
            vec![db_controller::CheckConstraintSpec {
                name: "ck_composite_unique_entity_external_nonempty",
                expression: "char_length(external_key) > 0",
            }]
        }
    }

    #[test]
    fn model_operations_include_composite_unique_constraints() {
        let operations = operations_for_model::<CompositeUniqueEntity>()
            .unwrap_or_else(|error| panic!("model operations: {error}"));
        let constraints = operations
            .iter()
            .filter(|operation| {
                matches!(
                    operation,
                    MigrationOperation::AddUniqueConstraint {
                        name,
                        columns,
                        ..
                    } if name == "uq_composite_unique_entity_tenant_external"
                        && columns == &["tenant_id".to_owned(), "external_key".to_owned()]
                )
            })
            .count();
        assert_eq!(constraints, 1);
    }

    #[test]
    fn model_operations_include_check_constraints() {
        let operations = operations_for_model::<CompositeUniqueEntity>()
            .unwrap_or_else(|error| panic!("model operations: {error}"));
        assert!(operations.iter().any(|operation| {
            matches!(
                operation,
                MigrationOperation::AddCheckConstraint { name, expression, .. }
                    if name == "ck_composite_unique_entity_external_nonempty"
                        && expression == "char_length(external_key) > 0"
            )
        }));
    }

    #[test]
    fn unsafe_check_constraints_are_rejected() {
        let operation = MigrationOperation::AddCheckConstraint {
            name: "ck_users_safe".into(),
            table: "users".into(),
            expression: "true); DROP TABLE users; --".into(),
        };
        assert!(operation.up_sql().is_err());
    }

    #[test]
    fn foreign_keys_have_explicit_delete_behavior() {
        let operation = MigrationOperation::AddForeignKey {
            name: "fk_membership_user".into(),
            table: "memberships".into(),
            local_column: "user_id".into(),
            target_table: "users".into(),
            remote_column: "id".into(),
            on_delete: OnDelete::Restrict,
        };
        assert!(operation.up_sql().unwrap()[0].ends_with("ON DELETE RESTRICT;"));
    }
}
