use serde_json::Value;
use std::{
    collections::HashMap,
    fmt::Write as _,
    sync::{LazyLock, Mutex, MutexGuard},
};

pub static TABLE_REGISTRY: LazyLock<Mutex<HashMap<String, TableMigration>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone)]
pub struct TableMigration {
    pub create_sql: String,
    pub drop_sql: String,
    pub migrate_sql: String,
    pub schema: HashMap<String, ColumnSpec>,
    pub relations: Vec<RelationSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnSpec {
    pub sql_type: String,
    pub nullable: bool,
    pub unique: bool,
    pub primary_key: bool,
    pub default: Option<String>,
    pub encrypted: bool,
}

impl ColumnSpec {
    pub fn build(&self) -> String {
        assert!(
            is_safe_sql_type(&self.sql_type),
            "ColumnSpec.sql_type must use a supported SQL type"
        );
        if let Some(default) = &self.default {
            assert!(
                is_safe_default_expression(default),
                "ColumnSpec.default contains an unsafe SQL fragment"
            );
        }

        let mut out = self.sql_type.clone();
        if self.primary_key {
            out.push_str(" PRIMARY KEY");
        } else if self.unique {
            out.push_str(" UNIQUE");
        }
        if !self.nullable && !self.primary_key {
            out.push_str(" NOT NULL");
        }
        if let Some(default) = &self.default {
            out.push_str(" DEFAULT ");
            out.push_str(default);
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationCardinality {
    One,
    Many,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnDelete {
    Cascade,
    Restrict,
    SetNull,
    SetDefault,
    NoAction,
}

impl OnDelete {
    pub const fn as_sql(self) -> &'static str {
        match self {
            Self::Cascade => "CASCADE",
            Self::Restrict => "RESTRICT",
            Self::SetNull => "SET NULL",
            Self::SetDefault => "SET DEFAULT",
            Self::NoAction => "NO ACTION",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationSpec {
    pub field_name: &'static str,
    pub target_table: &'static str,
    pub local_column: &'static str,
    pub remote_column: &'static str,
    pub cardinality: RelationCardinality,
    pub nullable: bool,
    pub related_name: Option<&'static str>,
    pub on_delete: OnDelete,
}

impl RelationSpec {
    pub const fn is_many(&self) -> bool {
        matches!(self.cardinality, RelationCardinality::Many)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexMethod {
    BTree,
    Hash,
    Gin,
    Gist,
    Brin,
}

impl IndexMethod {
    pub const fn as_sql(self) -> &'static str {
        match self {
            Self::BTree => "btree",
            Self::Hash => "hash",
            Self::Gin => "gin",
            Self::Gist => "gist",
            Self::Brin => "brin",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexSpec {
    pub name: &'static str,
    pub columns: &'static [&'static str],
    pub unique: bool,
    pub method: IndexMethod,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UniqueConstraintSpec {
    pub name: &'static str,
    pub columns: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckConstraintSpec {
    pub name: &'static str,
    pub expression: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelMetadata {
    pub table: &'static str,
    pub primary_key: &'static str,
    pub soft_delete_column: Option<&'static str>,
    pub tenant_column: Option<&'static str>,
    pub version_column: Option<&'static str>,
    pub default_ordering: Vec<&'static str>,
    pub relations: Vec<RelationSpec>,
    pub unique_constraints: Vec<UniqueConstraintSpec>,
    pub check_constraints: Vec<CheckConstraintSpec>,
}

pub fn is_safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.split('.').all(|segment| {
            if segment.is_empty() || segment.len() > 63 {
                return false;
            }
            let mut chars = segment.chars();
            matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
                && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
        })
}

pub fn quote_identifier(value: &str) -> Option<String> {
    if !is_safe_identifier(value) {
        return None;
    }
    Some(
        value
            .split('.')
            .map(|part| format!("\"{part}\""))
            .collect::<Vec<_>>()
            .join("."),
    )
}

pub fn regex_is_match(pattern: &str, input: &str) -> Result<bool, regex::Error> {
    regex::Regex::new(pattern).map(|compiled| compiled.is_match(input))
}

pub fn is_safe_sql_type(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_uppercase().as_str(),
        "SMALLINT"
            | "INTEGER"
            | "BIGINT"
            | "BIGSERIAL"
            | "REAL"
            | "DOUBLE PRECISION"
            | "NUMERIC"
            | "BOOLEAN"
            | "UUID"
            | "TIMESTAMP"
            | "TIMESTAMPTZ"
            | "DATE"
            | "TIME"
            | "INTERVAL"
            | "TEXT"
            | "CHARACTER VARYING"
            | "JSON"
            | "JSONB"
            | "BYTEA"
            | "INET"
            | "CIDR"
    )
}

pub fn is_safe_default_expression(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && !trimmed.contains(';')
        && !trimmed.contains("--")
        && !trimmed.contains("/*")
        && !trimmed.contains("*/")
        && !trimmed.contains('\0')
        && !trimmed.contains('\n')
        && !trimmed.contains('\r')
}

/// Conservatively validate developer-authored CHECK expressions before embedding them in DDL.
/// Complex expressions that require subqueries, DDL, procedural SQL, comments, or dollar-quoted
/// content must use an explicitly reviewed SQL migration instead. PostgreSQL itself remains the
/// final syntax/type validator when the migration is applied.
pub fn is_safe_check_expression(value: &str) -> bool {
    const MAX_CHECK_EXPRESSION_BYTES: usize = 4_096;
    const FORBIDDEN_KEYWORDS: &[&str] = &[
        "alter", "call", "copy", "create", "delete", "do", "drop", "execute", "grant", "insert",
        "revoke", "select", "truncate", "update",
    ];

    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.len() > MAX_CHECK_EXPRESSION_BYTES
        || trimmed.contains(';')
        || trimmed.contains("--")
        || trimmed.contains("/*")
        || trimmed.contains("*/")
        || trimmed.contains('\0')
        || trimmed.contains('\n')
        || trimmed.contains('\r')
        || trimmed.contains('$')
    {
        return false;
    }

    let mut token = String::new();
    let mut in_single_quote = false;
    let mut escaped_quote = false;
    for character in trimmed.chars() {
        if in_single_quote {
            if character == '\'' {
                escaped_quote = !escaped_quote;
            } else if escaped_quote {
                in_single_quote = false;
                escaped_quote = false;
            }
            continue;
        }

        if character == '\'' {
            if !token.is_empty() {
                if FORBIDDEN_KEYWORDS.contains(&token.to_ascii_lowercase().as_str()) {
                    return false;
                }
                token.clear();
            }
            in_single_quote = true;
            continue;
        }

        if character.is_ascii_alphanumeric() || character == '_' {
            token.push(character);
        } else if !token.is_empty() {
            if FORBIDDEN_KEYWORDS.contains(&token.to_ascii_lowercase().as_str()) {
                return false;
            }
            token.clear();
        }
    }
    if in_single_quote && !escaped_quote {
        return false;
    }
    if !token.is_empty() && FORBIDDEN_KEYWORDS.contains(&token.to_ascii_lowercase().as_str()) {
        return false;
    }
    true
}

fn lock_registry() -> MutexGuard<'static, HashMap<String, TableMigration>> {
    TABLE_REGISTRY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub trait Entity: Sized {
    fn table_name() -> &'static str;

    fn primary_key() -> &'static str {
        "id"
    }

    /// Fields explicitly represented by the Rust model.
    fn schema() -> HashMap<String, ColumnSpec>;
    fn to_map(&self) -> HashMap<String, Value>;

    /// Model-level validation generated by the ORM derive. Manual entities may override this.
    fn validate_model(&self) -> Result<(), String> {
        Ok(())
    }

    /// Canonical typed index metadata. New/generated models should prefer this to raw SQL.
    fn typed_indexes() -> Vec<IndexSpec> {
        Vec::new()
    }

    /// Compatibility hook for callers that expect rendered index SQL. Typed index metadata is
    /// rendered here by default; legacy manual entities may still override this method.
    fn indexes() -> Vec<String> {
        Self::typed_indexes()
            .iter()
            .map(|index| {
                render_index_spec::<Self>(index)
                    .unwrap_or_else(|error| panic!("invalid index metadata: {error}"))
            })
            .collect()
    }

    fn relations() -> Vec<RelationSpec> {
        Vec::new()
    }

    fn unique_constraints() -> Vec<UniqueConstraintSpec> {
        Vec::new()
    }

    fn check_constraints() -> Vec<CheckConstraintSpec> {
        Vec::new()
    }

    fn default_ordering() -> Vec<&'static str> {
        Vec::new()
    }

    fn soft_delete_column() -> Option<&'static str> {
        Self::schema()
            .contains_key("deleted_at")
            .then_some("deleted_at")
    }

    fn tenant_column() -> Option<&'static str> {
        Self::schema()
            .contains_key("tenant_id")
            .then_some("tenant_id")
    }

    fn version_column() -> Option<&'static str> {
        Self::schema().contains_key("version").then_some("version")
    }

    fn metadata() -> ModelMetadata {
        ModelMetadata {
            table: Self::table_name(),
            primary_key: Self::primary_key(),
            soft_delete_column: Self::soft_delete_column(),
            tenant_column: Self::tenant_column(),
            version_column: Self::version_column(),
            default_ordering: Self::default_ordering(),
            relations: Self::relations(),
            unique_constraints: Self::unique_constraints(),
            check_constraints: Self::check_constraints(),
        }
    }

    fn register() {
        let table = Self::table_name().to_owned();
        let quoted_table = quote_identifier(Self::table_name())
            .unwrap_or_else(|| panic!("Entity table_name must be a safe SQL identifier"));
        let create_sql = generate_create_table_sql::<Self>();
        let drop_sql = format!("DROP TABLE IF EXISTS {quoted_table};");
        let mut migrate_sql = create_sql.clone();
        for index in Self::indexes() {
            let _ = write!(migrate_sql, "\n{index}");
        }
        lock_registry().insert(
            table,
            TableMigration {
                create_sql,
                drop_sql,
                migrate_sql,
                schema: effective_schema::<Self>(),
                relations: Self::relations(),
            },
        );
    }

    fn create_sql() -> String {
        generate_create_table_sql::<Self>()
    }

    fn drop_sql() -> String {
        let table = quote_identifier(Self::table_name())
            .unwrap_or_else(|| panic!("Entity table_name must be a safe SQL identifier"));
        format!("DROP TABLE IF EXISTS {table};")
    }

    fn full_migration_sql() -> String {
        let mut sql = Self::create_sql();
        for index in Self::indexes() {
            let _ = write!(sql, "\n{index}");
        }
        sql
    }
}

pub trait EntityObject: Send + Sync + 'static {
    fn table_name(&self) -> &'static str;
    fn schema(&self) -> HashMap<String, ColumnSpec>;
    fn to_map_obj(&self) -> HashMap<String, Value>;
}

impl<T> EntityObject for T
where
    T: Entity + Send + Sync + 'static,
{
    fn table_name(&self) -> &'static str {
        T::table_name()
    }

    fn schema(&self) -> HashMap<String, ColumnSpec> {
        effective_schema::<T>()
    }

    fn to_map_obj(&self) -> HashMap<String, Value> {
        T::to_map(self)
    }
}

fn default_columns() -> HashMap<String, ColumnSpec> {
    HashMap::from([
        (
            "id".to_owned(),
            ColumnSpec {
                sql_type: "BIGSERIAL".to_owned(),
                nullable: false,
                unique: false,
                primary_key: true,
                default: None,
                encrypted: false,
            },
        ),
        (
            "created_at".to_owned(),
            ColumnSpec {
                sql_type: "TIMESTAMPTZ".to_owned(),
                nullable: false,
                unique: false,
                primary_key: false,
                default: Some("now()".to_owned()),
                encrypted: false,
            },
        ),
        (
            "updated_at".to_owned(),
            ColumnSpec {
                sql_type: "TIMESTAMPTZ".to_owned(),
                nullable: false,
                unique: false,
                primary_key: false,
                default: Some("now()".to_owned()),
                encrypted: false,
            },
        ),
    ])
}

/// Return the actual database schema used by generated DDL, including framework columns.
pub fn effective_schema<E: Entity>() -> HashMap<String, ColumnSpec> {
    let mut columns = E::schema();
    for (name, spec) in default_columns() {
        columns.entry(name).or_insert(spec);
    }
    columns
}

pub fn render_index_spec<E: Entity>(index: &IndexSpec) -> Result<String, String> {
    if index.columns.is_empty() {
        return Err(format!(
            "index {} must contain at least one column",
            index.name
        ));
    }
    if index.columns.len() > 32 {
        return Err(format!(
            "index {} exceeds the 32-column ORM safety limit",
            index.name
        ));
    }
    let name = quote_identifier(index.name)
        .ok_or_else(|| format!("unsafe index identifier: {}", index.name))?;
    let table = quote_identifier(E::table_name())
        .ok_or_else(|| format!("unsafe table identifier: {}", E::table_name()))?;
    let schema = effective_schema::<E>();
    let columns = index
        .columns
        .iter()
        .map(|column| {
            if !schema.contains_key(*column) {
                return Err(format!(
                    "index {} references unknown column {}.{}",
                    index.name,
                    E::table_name(),
                    column
                ));
            }
            quote_identifier(column)
                .ok_or_else(|| format!("unsafe index column identifier: {column}"))
        })
        .collect::<Result<Vec<_>, _>>()?
        .join(", ");
    Ok(format!(
        "CREATE {}INDEX IF NOT EXISTS {name} ON {table} USING {} ({columns});",
        if index.unique { "UNIQUE " } else { "" },
        index.method.as_sql()
    ))
}

pub fn generate_create_table_sql<E: Entity>() -> String {
    let table = quote_identifier(E::table_name())
        .unwrap_or_else(|| panic!("Entity table_name must be a safe SQL identifier"));
    let schema = effective_schema::<E>();
    let mut columns = schema.iter().collect::<Vec<_>>();
    columns.sort_by(|left, right| left.0.cmp(right.0));

    let mut definitions = columns
        .into_iter()
        .map(|(name, spec)| {
            let name = quote_identifier(name)
                .unwrap_or_else(|| panic!("Entity column name must be a safe SQL identifier"));
            format!("    {name} {}", spec.build())
        })
        .collect::<Vec<_>>();

    let mut unique_constraints = E::unique_constraints();
    unique_constraints.sort_by(|left, right| left.name.cmp(right.name));
    for constraint in unique_constraints {
        if constraint.columns.is_empty() {
            panic!(
                "unique constraint {} must contain at least one column",
                constraint.name
            );
        }
        // A single-column #[field(unique)] is already emitted by ColumnSpec::build().
        if constraint.columns.len() == 1
            && schema
                .get(constraint.columns[0])
                .is_some_and(|column| column.unique || column.primary_key)
        {
            continue;
        }
        let name = quote_identifier(constraint.name)
            .unwrap_or_else(|| panic!("unique constraint name must be a safe SQL identifier"));
        let columns = constraint
            .columns
            .iter()
            .map(|column| {
                if !schema.contains_key(*column) {
                    panic!(
                        "unique constraint {} references unknown column {}",
                        constraint.name, column
                    );
                }
                quote_identifier(column).unwrap_or_else(|| {
                    panic!("unique constraint column must be a safe SQL identifier")
                })
            })
            .collect::<Vec<_>>()
            .join(", ");
        definitions.push(format!("    CONSTRAINT {name} UNIQUE ({columns})"));
    }

    let mut check_constraints = E::check_constraints();
    check_constraints.sort_by(|left, right| left.name.cmp(right.name));
    for constraint in check_constraints {
        let name = quote_identifier(constraint.name)
            .unwrap_or_else(|| panic!("check constraint name must be a safe SQL identifier"));
        if !is_safe_check_expression(constraint.expression) {
            panic!(
                "check constraint {} contains an unsafe SQL expression",
                constraint.name
            );
        }
        definitions.push(format!(
            "    CONSTRAINT {name} CHECK ({})",
            constraint.expression.trim()
        ));
    }

    format!(
        "CREATE TABLE IF NOT EXISTS {table} (\n{}\n);",
        definitions.join(",\n")
    )
}

/// Render an Entity schema using SQLite-compatible affinity and defaults. This intentionally
/// keeps PostgreSQL as the canonical rich backend while allowing the same model metadata to be
/// used for local development and lightweight deployments.
pub fn generate_create_table_sql_sqlite<E: Entity>() -> String {
    let table = quote_identifier(E::table_name())
        .unwrap_or_else(|| panic!("Entity table_name must be a safe SQL identifier"));
    let schema = effective_schema::<E>();
    let mut columns = schema.iter().collect::<Vec<_>>();
    columns.sort_by(|left, right| left.0.cmp(right.0));

    let mut definitions = columns
        .into_iter()
        .map(|(name, spec)| {
            let name = quote_identifier(name)
                .unwrap_or_else(|| panic!("Entity column name must be a safe SQL identifier"));
            format!("    {name} {}", sqlite_column_spec(spec))
        })
        .collect::<Vec<_>>();

    let mut unique_constraints = E::unique_constraints();
    unique_constraints.sort_by(|left, right| left.name.cmp(right.name));
    for constraint in unique_constraints {
        if constraint.columns.is_empty() {
            panic!(
                "unique constraint {} must contain at least one column",
                constraint.name
            );
        }
        if constraint.columns.len() == 1
            && schema
                .get(constraint.columns[0])
                .is_some_and(|column| column.unique || column.primary_key)
        {
            continue;
        }
        let name = quote_identifier(constraint.name)
            .unwrap_or_else(|| panic!("unique constraint name must be a safe SQL identifier"));
        let columns = constraint
            .columns
            .iter()
            .map(|column| {
                quote_identifier(column).unwrap_or_else(|| {
                    panic!("unique constraint column must be a safe SQL identifier")
                })
            })
            .collect::<Vec<_>>()
            .join(", ");
        definitions.push(format!("    CONSTRAINT {name} UNIQUE ({columns})"));
    }

    let mut check_constraints = E::check_constraints();
    check_constraints.sort_by(|left, right| left.name.cmp(right.name));
    for constraint in check_constraints {
        let name = quote_identifier(constraint.name)
            .unwrap_or_else(|| panic!("check constraint name must be a safe SQL identifier"));
        if !is_safe_check_expression(constraint.expression) {
            panic!(
                "check constraint {} contains an unsafe SQL expression",
                constraint.name
            );
        }
        definitions.push(format!(
            "    CONSTRAINT {name} CHECK ({})",
            constraint.expression.trim()
        ));
    }

    // SQLite cannot add foreign-key constraints with PostgreSQL-style ALTER TABLE after table
    // creation, so singular relation metadata is emitted inline in CREATE TABLE.
    let mut relations = E::relations();
    relations.sort_by(|left, right| left.field_name.cmp(right.field_name));
    for relation in relations {
        if relation.cardinality == RelationCardinality::Many {
            continue;
        }
        let local = quote_identifier(relation.local_column)
            .unwrap_or_else(|| panic!("relation local column must be a safe SQL identifier"));
        let target = quote_identifier(relation.target_table)
            .unwrap_or_else(|| panic!("relation target table must be a safe SQL identifier"));
        let remote = quote_identifier(relation.remote_column)
            .unwrap_or_else(|| panic!("relation remote column must be a safe SQL identifier"));
        let constraint_name = format!(
            "fk_{}_{}_{}",
            E::table_name(),
            relation.local_column,
            relation.target_table
        );
        let constraint = quote_identifier(&constraint_name)
            .unwrap_or_else(|| panic!("generated SQLite foreign-key name is invalid"));
        definitions.push(format!(
            "    CONSTRAINT {constraint} FOREIGN KEY ({local}) REFERENCES {target} ({remote}) ON DELETE {}",
            relation.on_delete.as_sql()
        ));
    }

    format!(
        "CREATE TABLE IF NOT EXISTS {table} (\n{}\n);",
        definitions.join(",\n")
    )
}

fn sqlite_column_spec(spec: &ColumnSpec) -> String {
    let mut out = match spec.sql_type.trim().to_ascii_uppercase().as_str() {
        "SMALLINT" | "INTEGER" | "BIGINT" | "BIGSERIAL" | "BOOLEAN" => "INTEGER".to_owned(),
        "REAL" | "DOUBLE PRECISION" => "REAL".to_owned(),
        // Preserve exact decimal text instead of routing NUMERIC through IEEE-754 REAL.
        // The SQLite executor binds Decimal values as strings for the same reason.
        kind if kind.starts_with("NUMERIC") || kind.starts_with("DECIMAL") => "TEXT".to_owned(),
        "BYTEA" => "BLOB".to_owned(),
        "UUID" | "TIMESTAMP" | "TIMESTAMPTZ" | "DATE" | "TIME" | "INTERVAL" | "TEXT"
        | "CHARACTER VARYING" | "JSON" | "JSONB" | "INET" | "CIDR" => "TEXT".to_owned(),
        other => other.to_owned(),
    };
    if spec.primary_key {
        if spec.sql_type.eq_ignore_ascii_case("BIGSERIAL") {
            out = "INTEGER PRIMARY KEY AUTOINCREMENT".to_owned();
        } else {
            out.push_str(" PRIMARY KEY");
        }
    } else if spec.unique {
        out.push_str(" UNIQUE");
    }
    if !spec.nullable && !spec.primary_key {
        out.push_str(" NOT NULL");
    }
    if let Some(default) = &spec.default {
        let default = sqlite_default_expression(default);
        out.push_str(" DEFAULT ");
        out.push_str(&default);
    }
    out
}

fn sqlite_default_expression(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("now()") {
        return "CURRENT_TIMESTAMP".to_owned();
    }
    trimmed
        .replace("::jsonb", "")
        .replace("::json", "")
        .replace("::uuid", "")
}

pub fn render_index_spec_sqlite<E: Entity>(index: &IndexSpec) -> Result<String, String> {
    if !matches!(index.method, IndexMethod::BTree) {
        return Err(format!(
            "SQLite supports only the framework's default BTree index semantics; index {} requests {:?}",
            index.name, index.method
        ));
    }
    if index.columns.is_empty() || index.columns.len() > 32 {
        return Err(format!(
            "invalid SQLite index column count for {}",
            index.name
        ));
    }
    let name = quote_identifier(index.name)
        .ok_or_else(|| format!("unsafe index identifier: {}", index.name))?;
    let table = quote_identifier(E::table_name())
        .ok_or_else(|| format!("unsafe table identifier: {}", E::table_name()))?;
    let schema = effective_schema::<E>();
    let columns = index
        .columns
        .iter()
        .map(|column| {
            if !schema.contains_key(*column) {
                return Err(format!(
                    "index {} references unknown column {}.{}",
                    index.name,
                    E::table_name(),
                    column
                ));
            }
            quote_identifier(column)
                .ok_or_else(|| format!("unsafe index column identifier: {column}"))
        })
        .collect::<Result<Vec<_>, _>>()?
        .join(", ");
    Ok(format!(
        "CREATE {}INDEX IF NOT EXISTS {name} ON {table} ({columns});",
        if index.unique { "UNIQUE " } else { "" }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Demo;
    impl Entity for Demo {
        fn table_name() -> &'static str {
            "demo"
        }

        fn schema() -> HashMap<String, ColumnSpec> {
            HashMap::from([(
                "name".to_owned(),
                ColumnSpec {
                    sql_type: "TEXT".to_owned(),
                    nullable: false,
                    unique: true,
                    primary_key: false,
                    default: None,
                    encrypted: false,
                },
            )])
        }

        fn to_map(&self) -> HashMap<String, Value> {
            HashMap::new()
        }
        fn typed_indexes() -> Vec<IndexSpec> {
            vec![IndexSpec {
                name: "idx_demo_name",
                columns: &["name"],
                unique: false,
                method: IndexMethod::BTree,
            }]
        }
    }

    struct TenantScopedUnique;
    impl Entity for TenantScopedUnique {
        fn table_name() -> &'static str {
            "tenant_scoped_unique"
        }
        fn schema() -> HashMap<String, ColumnSpec> {
            HashMap::from([
                (
                    "tenant_id".into(),
                    ColumnSpec {
                        sql_type: "UUID".into(),
                        nullable: false,
                        unique: false,
                        primary_key: false,
                        default: None,
                        encrypted: false,
                    },
                ),
                (
                    "external_key".into(),
                    ColumnSpec {
                        sql_type: "TEXT".into(),
                        nullable: false,
                        unique: false,
                        primary_key: false,
                        default: None,
                        encrypted: false,
                    },
                ),
            ])
        }
        fn to_map(&self) -> HashMap<String, Value> {
            HashMap::new()
        }
        fn unique_constraints() -> Vec<UniqueConstraintSpec> {
            vec![UniqueConstraintSpec {
                name: "uq_tenant_scoped_unique_tenant_external",
                columns: &["tenant_id", "external_key"],
            }]
        }
        fn check_constraints() -> Vec<CheckConstraintSpec> {
            vec![CheckConstraintSpec {
                name: "ck_tenant_scoped_unique_external_nonempty",
                expression: "char_length(external_key) > 0",
            }]
        }
    }

    #[test]
    fn create_sql_emits_model_constraints_once() {
        let sql = TenantScopedUnique::create_sql();
        assert!(sql.contains(
            "CONSTRAINT \"uq_tenant_scoped_unique_tenant_external\" UNIQUE (\"tenant_id\", \"external_key\")"
        ));
        assert!(sql.contains(
            "CONSTRAINT \"ck_tenant_scoped_unique_external_nonempty\" CHECK (char_length(external_key) > 0)"
        ));
    }

    #[test]
    fn check_expression_validator_rejects_statement_injection() {
        assert!(is_safe_check_expression("char_length(name) > 0"));
        assert!(is_safe_check_expression("status IN ('active', 'pending')"));
        assert!(!is_safe_check_expression("true); DROP TABLE users; --"));
        assert!(!is_safe_check_expression("id IN (SELECT id FROM admins)"));
    }

    #[test]
    fn typed_indexes_validate_columns_and_render_deterministically() {
        assert_eq!(
            render_index_spec::<Demo>(&Demo::typed_indexes()[0])
                .unwrap_or_else(|error| panic!("index SQL: {error}")),
            "CREATE INDEX IF NOT EXISTS \"idx_demo_name\" ON \"demo\" USING btree (\"name\");"
        );
    }

    #[test]
    fn identifiers_are_restricted() {
        assert!(is_safe_identifier("public.users"));
        assert!(!is_safe_identifier("users; DROP TABLE users"));
        assert!(!is_safe_identifier("public.\"users\""));
        assert!(!is_safe_identifier(&"a".repeat(64)));
    }

    #[test]
    fn sql_types_and_defaults_are_restricted() {
        assert!(is_safe_sql_type("TIMESTAMPTZ"));
        assert!(is_safe_sql_type("INET"));
        assert!(!is_safe_sql_type("TEXT); DROP TABLE users; --"));
        assert!(is_safe_default_expression("'{}'::jsonb"));
        assert!(!is_safe_default_expression("false; DROP TABLE users"));
    }

    #[test]
    fn effective_schema_contains_framework_columns() {
        let schema = effective_schema::<Demo>();
        assert!(schema.contains_key("id"));
        assert!(schema.contains_key("created_at"));
        assert!(schema.contains_key("updated_at"));
        assert!(schema.contains_key("name"));
    }

    #[test]
    fn metadata_infers_framework_policies() {
        let metadata = Demo::metadata();
        assert_eq!(metadata.table, "demo");
        assert_eq!(metadata.primary_key, "id");
        assert_eq!(metadata.soft_delete_column, None);
        assert_eq!(metadata.tenant_column, None);
    }
}
