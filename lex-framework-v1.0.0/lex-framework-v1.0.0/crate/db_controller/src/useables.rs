use crate::entity::{Entity, quote_identifier};

fn trusted_identifier(value: &str) -> String {
    quote_identifier(value)
        .unwrap_or_else(|| panic!("trait-provided SQL identifier must be safe: {value}"))
}

pub trait SoftDelete: Entity {
    fn deleted_at_column() -> &'static str {
        "deleted_at"
    }

    fn soft_delete_sql() -> String {
        let table = trusted_identifier(Self::table_name());
        let deleted_at = trusted_identifier(Self::deleted_at_column());
        let primary_key = trusted_identifier(Self::primary_key());
        format!(
            "UPDATE {table} SET {deleted_at} = now() WHERE {primary_key}::text = $1 AND {deleted_at} IS NULL"
        )
    }

    fn hard_delete_sql() -> String {
        let table = trusted_identifier(Self::table_name());
        let primary_key = trusted_identifier(Self::primary_key());
        format!("DELETE FROM {table} WHERE {primary_key}::text = $1")
    }
}

pub trait Versioned: Entity {
    fn version_column() -> &'static str {
        "version"
    }

    fn version_check_clause() -> String {
        format!(
            "{} = $1",
            trusted_identifier(<Self as Versioned>::version_column())
        )
    }
}

pub trait Auditable: Entity {
    fn audit_table() -> &'static str {
        "audit_log"
    }

    fn audit_insert_sql() -> String {
        let table = trusted_identifier(Self::audit_table());
        format!(
            "INSERT INTO {table} (\"table_name\", \"action\", \"actor_id\", \"timestamp\", \"before\", \"after\") VALUES ($1, $2, $3, $4, $5, $6)"
        )
    }
}

pub trait MultiTenant: Entity {
    fn tenant_column() -> &'static str {
        "tenant_id"
    }

    fn tenant_scope_clause() -> String {
        format!(
            "{} = $1",
            trusted_identifier(<Self as MultiTenant>::tenant_column())
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ColumnSpec;
    use serde_json::Value;
    use std::collections::HashMap;

    struct Demo;
    impl Entity for Demo {
        fn table_name() -> &'static str {
            "demo"
        }

        fn schema() -> HashMap<String, ColumnSpec> {
            HashMap::new()
        }

        fn to_map(&self) -> HashMap<String, Value> {
            HashMap::new()
        }
    }
    impl SoftDelete for Demo {}
    impl Versioned for Demo {}
    impl Auditable for Demo {}
    impl MultiTenant for Demo {}

    #[test]
    fn generated_trait_sql_quotes_identifiers() {
        assert_eq!(
            Demo::soft_delete_sql(),
            "UPDATE \"demo\" SET \"deleted_at\" = now() WHERE \"id\"::text = $1 AND \"deleted_at\" IS NULL"
        );
        assert_eq!(Demo::version_check_clause(), "\"version\" = $1");
        assert_eq!(Demo::tenant_scope_clause(), "\"tenant_id\" = $1");
        assert!(Demo::audit_insert_sql().starts_with("INSERT INTO \"audit_log\""));
    }
}
