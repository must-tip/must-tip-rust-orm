//! Constrained normalized RBAC schema and a conservative legacy PostgreSQL conversion.
//!
//! Legacy grants remain intact. Conversion refuses differing per-member permission sets for
//! the same role: their union would grant privileges that some members previously lacked.

use db_controller::{Entity, generate_create_table_sql, generate_create_table_sql_sqlite};
use models::{
    Membership, MembershipRole, PermissionDefinition, RolePermission, Tenant, TenantRole, User,
};
use sqlx::Row;

// Verify structural keys independently of index names. CREATE IF NOT EXISTS can otherwise
// accept a pre-existing table/index that does not enforce the advertised RBAC invariant.
const UNIQUE_KEYS: &[(&str, &[&str])] = &[
    ("users", &["id"]),
    ("tenants", &["id"]),
    ("memberships", &["id"]),
    ("memberships", &["user_id", "tenant_id"]),
    ("memberships", &["id", "tenant_id"]),
    ("tenant_roles", &["id"]),
    ("tenant_roles", &["tenant_id", "name"]),
    ("tenant_roles", &["id", "tenant_id"]),
    ("permission_definitions", &["id"]),
    ("permission_definitions", &["name"]),
    ("membership_roles", &["id"]),
    ("membership_roles", &["membership_id", "role_id"]),
    ("role_permissions", &["id"]),
    ("role_permissions", &["role_id", "permission_id"]),
];

fn constrained(mut ddl: String, constraints: &[&str]) -> String {
    if let Some(end) = ddl.rfind(')') {
        let additions = constraints
            .iter()
            .map(|value| format!(",\n    {value}"))
            .collect::<String>();
        ddl.insert_str(end, &additions);
    }
    ddl
}

fn schema(sqlite: bool) -> Vec<String> {
    fn table<T: Entity>(sqlite: bool) -> String {
        if sqlite {
            generate_create_table_sql_sqlite::<T>()
        } else {
            generate_create_table_sql::<T>()
        }
    }
    vec![
        table::<User>(sqlite), table::<Tenant>(sqlite),
        constrained(table::<Membership>(sqlite), &[
            "CONSTRAINT membership_user_fk FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE RESTRICT",
            "CONSTRAINT membership_tenant_fk FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE RESTRICT",
        ]),
        constrained(table::<TenantRole>(sqlite), &[
            "CONSTRAINT tenant_role_tenant_fk FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE RESTRICT",
        ]),
        table::<PermissionDefinition>(sqlite),
        // These indexes also upgrade pre-existing PostgreSQL parent tables before composite FKs.
        "CREATE UNIQUE INDEX IF NOT EXISTS membership_user_tenant ON memberships(user_id,tenant_id)".into(),
        "CREATE UNIQUE INDEX IF NOT EXISTS membership_id_tenant ON memberships(id,tenant_id)".into(),
        "CREATE UNIQUE INDEX IF NOT EXISTS tenant_role_name ON tenant_roles(tenant_id,name)".into(),
        "CREATE UNIQUE INDEX IF NOT EXISTS tenant_role_id_tenant ON tenant_roles(id,tenant_id)".into(),
        constrained(table::<MembershipRole>(sqlite), &[
            "CONSTRAINT assignment_membership_tenant_fk FOREIGN KEY (membership_id,tenant_id) REFERENCES memberships(id,tenant_id) ON DELETE RESTRICT",
            "CONSTRAINT assignment_role_tenant_fk FOREIGN KEY (role_id,tenant_id) REFERENCES tenant_roles(id,tenant_id) ON DELETE RESTRICT",
        ]),
        constrained(table::<RolePermission>(sqlite), &[
            "CONSTRAINT grant_role_tenant_fk FOREIGN KEY (role_id,tenant_id) REFERENCES tenant_roles(id,tenant_id) ON DELETE RESTRICT",
            "CONSTRAINT grant_permission_fk FOREIGN KEY (permission_id) REFERENCES permission_definitions(id) ON DELETE RESTRICT",
        ]),
        "CREATE INDEX IF NOT EXISTS membership_roles_tenant_member ON membership_roles(tenant_id,membership_id)".into(),
        "CREATE INDEX IF NOT EXISTS role_permissions_tenant_role ON role_permissions(tenant_id,role_id)".into(),
    ]
}

/// Install normalized RBAC in a PostgreSQL migration transaction. Parent deletion is restricted:
/// revoke assignments and grants explicitly before deleting identities or tenant roles.
pub async fn install_rbac_postgres(pool: &sqlx::PgPool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    install_postgres(&mut tx).await?;
    tx.commit().await
}

async fn install_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), sqlx::Error> {
    for statement in schema(false) {
        sqlx::raw_sql(sqlx::AssertSqlSafe(statement))
            .execute(&mut **tx)
            .await?;
    }
    for (table, name, definition) in [
        (
            "memberships",
            "membership_user_fk",
            "FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE RESTRICT",
        ),
        (
            "memberships",
            "membership_tenant_fk",
            "FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE RESTRICT",
        ),
        (
            "tenant_roles",
            "tenant_role_tenant_fk",
            "FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE RESTRICT",
        ),
        (
            "membership_roles",
            "assignment_membership_tenant_fk",
            "FOREIGN KEY (membership_id, tenant_id) REFERENCES memberships(id, tenant_id) ON DELETE RESTRICT",
        ),
        (
            "membership_roles",
            "assignment_role_tenant_fk",
            "FOREIGN KEY (role_id, tenant_id) REFERENCES tenant_roles(id, tenant_id) ON DELETE RESTRICT",
        ),
        (
            "role_permissions",
            "grant_role_tenant_fk",
            "FOREIGN KEY (role_id, tenant_id) REFERENCES tenant_roles(id, tenant_id) ON DELETE RESTRICT",
        ),
        (
            "role_permissions",
            "grant_permission_fk",
            "FOREIGN KEY (permission_id) REFERENCES permission_definitions(id) ON DELETE RESTRICT",
        ),
    ] {
        // Identifiers and DDL below are fixed framework constants, never caller input.
        let statement = format!(
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conrelid = '{table}'::regclass AND conname = '{name}') THEN ALTER TABLE {table} ADD CONSTRAINT {name} {definition}; END IF; END $$"
        );
        sqlx::raw_sql(sqlx::AssertSqlSafe(statement))
            .execute(&mut **tx)
            .await?;
        // A matching name is insufficient: an existing schema could contain a weaker
        // constraint under that name. Refuse it instead of certifying unsafe RBAC.
        let actual: String = sqlx::query_scalar(
            "SELECT pg_get_constraintdef(oid) FROM pg_constraint WHERE conrelid = to_regclass($1) AND conname = $2 AND convalidated AND NOT condeferrable",
        )
        .bind(table)
        .bind(name)
        .fetch_one(&mut **tx)
        .await?;
        let canonical = |sql: &str| {
            sql.chars()
                .filter(|character| !character.is_ascii_whitespace())
                .collect::<String>()
                .to_ascii_lowercase()
        };
        if canonical(&actual) != canonical(definition) {
            return Err(sqlx::Error::Protocol(format!(
                "PostgreSQL RBAC constraint {name} on {table} requires a reviewed repair migration"
            )));
        }
    }
    for (table, columns) in UNIQUE_KEYS {
        let expected = columns
            .iter()
            .map(|column| (*column).to_owned())
            .collect::<Vec<_>>();
        let valid: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid = to_regclass($1) AND i.indisunique AND i.indisvalid AND i.indimmediate AND i.indpred IS NULL AND i.indexprs IS NULL AND (SELECT array_agg(a.attname::text ORDER BY k.position) FROM unnest(i.indkey) WITH ORDINALITY k(number,position) JOIN pg_attribute a ON a.attrelid=i.indrelid AND a.attnum=k.number WHERE k.position <= i.indnkeyatts) = $2::text[])",
        ).bind(table).bind(expected).fetch_one(&mut **tx).await?;
        if !valid {
            return Err(sqlx::Error::Protocol(format!(
                "PostgreSQL RBAC table {table} requires a reviewed unique-key repair for {}",
                columns.join(",")
            )));
        }
    }
    Ok(())
}

/// Install a fresh SQLite RBAC schema. Existing unconstrained SQLite identity tables are
/// rejected; they require an explicit table-rebuild migration instead of silently weak integrity.
pub async fn install_rbac_sqlite(pool: &sqlx::SqlitePool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&mut *tx)
        .await?;
    if foreign_keys != 1 {
        return Err(sqlx::Error::Protocol(
            "RBAC requires SQLite foreign_keys=ON on every connection".into(),
        ));
    }
    for statement in schema(true) {
        sqlx::raw_sql(sqlx::AssertSqlSafe(statement))
            .execute(&mut *tx)
            .await?;
    }
    for (table, target, columns) in [
        ("memberships", "users", &[("user_id", "id")][..]),
        ("memberships", "tenants", &[("tenant_id", "id")][..]),
        ("tenant_roles", "tenants", &[("tenant_id", "id")][..]),
        (
            "membership_roles",
            "memberships",
            &[("membership_id", "id"), ("tenant_id", "tenant_id")][..],
        ),
        (
            "membership_roles",
            "tenant_roles",
            &[("role_id", "id"), ("tenant_id", "tenant_id")][..],
        ),
        (
            "role_permissions",
            "tenant_roles",
            &[("role_id", "id"), ("tenant_id", "tenant_id")][..],
        ),
        (
            "role_permissions",
            "permission_definitions",
            &[("permission_id", "id")][..],
        ),
    ] {
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "PRAGMA foreign_key_list({table})"
        )))
        .fetch_all(&mut *tx)
        .await?;
        // Composite keys must belong to the same constraint, not merely exist independently.
        let valid = rows.iter().any(|candidate| {
            let id: i64 = candidate.get("id");
            let mut actual = rows
                .iter()
                .filter(|row| row.get::<i64, _>("id") == id)
                .collect::<Vec<_>>();
            actual.sort_by_key(|row| row.get::<i64, _>("seq"));
            actual.len() == columns.len()
                && actual.iter().zip(columns).all(|(row, (local, remote))| {
                    row.get::<String, _>("table") == target
                        && row.get::<String, _>("from") == *local
                        && row.get::<String, _>("to") == *remote
                        && row.get::<String, _>("on_delete") == "RESTRICT"
                })
        });
        if !valid {
            return Err(sqlx::Error::Protocol(format!(
                "SQLite RBAC table {table} requires a reviewed constraint-rebuild migration"
            )));
        }
    }
    for (table, columns) in UNIQUE_KEYS {
        let indexes =
            sqlx::query("SELECT name FROM pragma_index_list($1) WHERE \"unique\"=1 AND partial=0")
                .bind(table)
                .fetch_all(&mut *tx)
                .await?;
        let mut valid = false;
        for index in indexes {
            // Table-valued PRAGMAs bind index names; even unusual pre-existing names are data.
            let name: String = index.try_get("name")?;
            let actual: Vec<Option<String>> =
                sqlx::query_scalar("SELECT name FROM pragma_index_info($1) ORDER BY seqno")
                    .bind(name)
                    .fetch_all(&mut *tx)
                    .await?;
            if actual.len() == columns.len()
                && actual
                    .iter()
                    .zip(*columns)
                    .all(|(actual, expected)| actual.as_deref() == Some(*expected))
            {
                valid = true;
                break;
            }
        }
        if !valid {
            return Err(sqlx::Error::Protocol(format!(
                "SQLite RBAC table {table} requires a reviewed unique-key rebuild for {}",
                columns.join(",")
            )));
        }
    }
    tx.commit().await
}

/// Convert consistent legacy per-membership grants without dropping or rewriting source data.
/// Run while moving the application to the normalized permission repository. Invalid references,
/// cross-tenant assignments and permission-set disagreement abort the entire transaction.
pub async fn normalize_legacy_rbac_postgres(pool: &sqlx::PgPool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("LOCK TABLE tenant_role_permissions, memberships, tenant_roles, permission_definitions IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *tx).await?;
    let invalid: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM tenant_role_permissions g LEFT JOIN memberships m ON m.id=g.membership_id LEFT JOIN tenant_roles r ON r.id=g.role_id LEFT JOIN permission_definitions p ON p.id=g.permission_id WHERE m.id IS NULL OR r.id IS NULL OR p.id IS NULL OR m.tenant_id <> r.tenant_id)"
    ).fetch_one(&mut *tx).await?;
    let inconsistent: bool = sqlx::query_scalar(
        "WITH permission_sets AS (SELECT role_id,membership_id,array_agg(DISTINCT permission_id ORDER BY permission_id) AS permissions FROM tenant_role_permissions GROUP BY role_id,membership_id) SELECT EXISTS (SELECT 1 FROM permission_sets GROUP BY role_id HAVING count(DISTINCT permissions)>1)"
    ).fetch_one(&mut *tx).await?;
    if invalid || inconsistent {
        return Err(sqlx::Error::Protocol("legacy RBAC contains invalid references, tenant mismatch, or differing member permission sets; split roles and repair source grants before conversion".into()));
    }
    install_postgres(&mut tx).await?;
    let populated: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM membership_roles) OR EXISTS(SELECT 1 FROM role_permissions)",
    )
    .fetch_one(&mut *tx)
    .await?;
    if populated {
        return Err(sqlx::Error::Protocol(
            "normalized RBAC is already populated; conversion must not merge authorization state"
                .into(),
        ));
    }
    sqlx::query("INSERT INTO membership_roles (id,tenant_id,membership_id,role_id) SELECT gen_random_uuid(),tenant_id,membership_id,role_id FROM (SELECT DISTINCT m.tenant_id,g.membership_id,g.role_id FROM tenant_role_permissions g JOIN memberships m ON m.id=g.membership_id) assignments")
        .execute(&mut *tx).await?;
    sqlx::query("INSERT INTO role_permissions (id,tenant_id,role_id,permission_id) SELECT gen_random_uuid(),tenant_id,role_id,permission_id FROM (SELECT DISTINCT r.tenant_id,g.role_id,g.permission_id FROM tenant_role_permissions g JOIN tenant_roles r ON r.id=g.role_id) grants")
        .execute(&mut *tx).await?;
    tx.commit().await
}
