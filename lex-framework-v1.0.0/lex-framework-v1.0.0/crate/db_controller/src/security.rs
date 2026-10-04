//! PostgreSQL defense-in-depth helpers for multi-tenant row-level security (RLS).
//!
//! Application QuerySets still apply tenant predicates. RLS is an independent database boundary
//! and should be enabled through reviewed migrations for sensitive tenant-owned tables.

use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{Entity, ModelError, quote_identifier};

pub const TENANT_GUC: &str = "app.tenant_id";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantRlsPolicy {
    pub table: &'static str,
    pub tenant_column: &'static str,
    pub force: bool,
}

impl TenantRlsPolicy {
    pub fn for_model<T: Entity>(force: bool) -> Result<Self, ModelError> {
        let tenant_column = T::tenant_column().ok_or_else(|| {
            ModelError::InvalidQuery(format!(
                "model {} does not declare tenant metadata",
                T::table_name()
            ))
        })?;
        Ok(Self {
            table: T::table_name(),
            tenant_column,
            force,
        })
    }

    pub fn enable_sql(&self) -> Result<Vec<String>, ModelError> {
        let table = ident(self.table)?;
        let tenant_column = ident(self.tenant_column)?;
        let policy_name = policy_name(self.table)?;
        let policy = ident(&policy_name)?;
        let predicate =
            format!("{tenant_column}::text = NULLIF(current_setting('{TENANT_GUC}', true), '')");
        let mut statements = vec![
            format!("ALTER TABLE {table} ENABLE ROW LEVEL SECURITY;"),
            format!("DROP POLICY IF EXISTS {policy} ON {table};"),
            format!(
                "CREATE POLICY {policy} ON {table} USING ({predicate}) WITH CHECK ({predicate});"
            ),
        ];
        if self.force {
            statements.push(format!("ALTER TABLE {table} FORCE ROW LEVEL SECURITY;"));
        }
        Ok(statements)
    }

    pub fn disable_sql(&self) -> Result<Vec<String>, ModelError> {
        let table = ident(self.table)?;
        let policy = ident(&policy_name(self.table)?)?;
        Ok(vec![
            format!("DROP POLICY IF EXISTS {policy} ON {table};"),
            format!("ALTER TABLE {table} DISABLE ROW LEVEL SECURITY;"),
            format!("ALTER TABLE {table} NO FORCE ROW LEVEL SECURITY;"),
        ])
    }
}

pub async fn set_transaction_tenant(
    transaction: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
) -> Result<(), ModelError> {
    sqlx::query("SELECT set_config($1, $2, true)")
        .bind(TENANT_GUC)
        .bind(tenant_id.to_string())
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

pub async fn clear_transaction_tenant(
    transaction: &mut Transaction<'_, Postgres>,
) -> Result<(), ModelError> {
    sqlx::query("SELECT set_config($1, '', true)")
        .bind(TENANT_GUC)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

fn policy_name(table: &str) -> Result<String, ModelError> {
    if quote_identifier(table).is_none() {
        return Err(ModelError::SqlBuildError(format!(
            "unsafe SQL identifier: {table}"
        )));
    }
    let digest = Sha256::digest(table.as_bytes());
    let suffix = digest[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut base = table.replace('.', "_");
    // PostgreSQL identifiers are limited to 63 bytes. Entity identifiers are ASCII-only, so byte
    // truncation cannot split a UTF-8 code point. Leave room for prefix, separator, and hash.
    const PREFIX: &str = "rls_tenant_";
    let max_base = 63usize
        .checked_sub(PREFIX.len() + 1 + suffix.len())
        .ok_or_else(|| ModelError::SqlBuildError("invalid RLS policy-name bounds".into()))?;
    if base.len() > max_base {
        base.truncate(max_base);
    }
    let value = format!("{PREFIX}{base}_{suffix}");
    if !crate::is_safe_identifier(&value) {
        return Err(ModelError::SqlBuildError(
            "generated PostgreSQL RLS policy name is invalid".into(),
        ));
    }
    Ok(value)
}

fn ident(value: &str) -> Result<String, ModelError> {
    quote_identifier(value)
        .ok_or_else(|| ModelError::SqlBuildError(format!("unsafe SQL identifier: {value}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ColumnSpec;
    use serde_json::Value;
    use std::collections::HashMap;

    struct TenantModel;
    impl Entity for TenantModel {
        fn table_name() -> &'static str {
            "tenant_records"
        }
        fn schema() -> HashMap<String, ColumnSpec> {
            HashMap::from([(
                "tenant_id".into(),
                ColumnSpec {
                    sql_type: "UUID".into(),
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

    #[test]
    fn rls_policy_is_tenant_scoped_and_reversible() {
        let policy = TenantRlsPolicy::for_model::<TenantModel>(true)
            .unwrap_or_else(|error| panic!("policy: {error}"));
        let sql = policy
            .enable_sql()
            .unwrap_or_else(|error| panic!("sql: {error}"));
        assert!(
            sql.iter()
                .any(|statement| statement.contains("ENABLE ROW LEVEL SECURITY"))
        );
        assert!(sql.iter().any(|statement| statement.contains(TENANT_GUC)));
        assert!(
            sql.iter()
                .any(|statement| statement.contains("FORCE ROW LEVEL SECURITY"))
        );
        assert_eq!(policy.disable_sql().unwrap().len(), 3);
    }

    #[test]
    fn schema_qualified_policy_names_are_valid_bounded_and_collision_resistant() {
        let qualified = policy_name("public.tenant_records")
            .unwrap_or_else(|error| panic!("qualified policy: {error}"));
        let flat = policy_name("public_tenant_records")
            .unwrap_or_else(|error| panic!("flat policy: {error}"));
        assert!(crate::is_safe_identifier(&qualified));
        assert!(qualified.len() <= 63);
        assert_ne!(qualified, flat);
    }
}
