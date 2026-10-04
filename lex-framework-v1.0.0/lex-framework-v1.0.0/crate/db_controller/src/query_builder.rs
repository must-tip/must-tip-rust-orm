use crate::{
    db_type::{
        Aggregate, BatchUpdate, BulkInsert, DeleteQuery, InsertBuilder, Join, JoinType, ModelQuery,
        Operator, SqlHook, UpsertBuilder,
    },
    quote_identifier,
};
use sqlx::{AssertSqlSafe, FromRow, PgPool, postgres::PgRow, query, query_as};
use std::collections::{HashMap, HashSet};

fn protocol(message: impl Into<String>) -> sqlx::Error {
    sqlx::Error::Protocol(message.into())
}

fn ident(value: &str) -> Result<String, sqlx::Error> {
    quote_identifier(value).ok_or_else(|| protocol(format!("unsafe SQL identifier: {value}")))
}

fn projection(value: &str) -> Result<String, sqlx::Error> {
    if value == "*" {
        return Ok("*".to_owned());
    }
    if let Some((func, rest)) = value.split_once('(') {
        let func = func.trim().to_ascii_uppercase();
        if !matches!(func.as_str(), "COUNT" | "SUM" | "AVG" | "MIN" | "MAX") {
            return Err(protocol("unsupported SELECT expression"));
        }
        let close = rest
            .find(')')
            .ok_or_else(|| protocol("invalid SELECT expression"))?;
        let arg = rest[..close].trim();
        let tail = rest[close + 1..].trim();
        let arg_sql = if arg == "*" {
            "*".to_owned()
        } else {
            ident(arg)?
        };
        let alias_sql = if tail.is_empty() {
            String::new()
        } else {
            let alias = tail
                .strip_prefix("as ")
                .or_else(|| tail.strip_prefix("AS "))
                .ok_or_else(|| protocol("SELECT expression aliases must use AS"))?;
            format!(" AS {}", ident(alias.trim())?)
        };
        return Ok(format!("{func}({arg_sql}){alias_sql}"));
    }
    ident(value)
}

fn validated_hook(value: &str) -> Result<&str, sqlx::Error> {
    const ALLOWED: &[&str] = &[
        "FOR UPDATE",
        "FOR UPDATE NOWAIT",
        "FOR UPDATE SKIP LOCKED",
        "FOR NO KEY UPDATE",
        "FOR NO KEY UPDATE NOWAIT",
        "FOR NO KEY UPDATE SKIP LOCKED",
        "FOR SHARE",
        "FOR SHARE NOWAIT",
        "FOR SHARE SKIP LOCKED",
        "FOR KEY SHARE",
        "FOR KEY SHARE NOWAIT",
        "FOR KEY SHARE SKIP LOCKED",
    ];
    let normalized = value.trim().to_ascii_uppercase();
    ALLOWED
        .iter()
        .copied()
        .find(|allowed| *allowed == normalized)
        .ok_or_else(|| protocol("unsupported raw SQL hook"))
}

impl Aggregate {
    fn to_sql(&self) -> &'static str {
        match self {
            Self::Count => "COUNT",
            Self::Sum => "SUM",
            Self::Avg => "AVG",
            Self::Max => "MAX",
            Self::Min => "MIN",
        }
    }
}
impl Operator {
    fn to_sql(&self) -> &'static str {
        match self {
            Self::Eq => "=",
            Self::Neq => "!=",
            Self::Gt => ">",
            Self::Gte => ">=",
            Self::Lt => "<",
            Self::Lte => "<=",
            Self::Like => "LIKE",
        }
    }
}
impl JoinType {
    fn to_sql(&self) -> &'static str {
        match self {
            Self::Inner => "INNER JOIN",
            Self::Left => "LEFT JOIN",
            Self::Right => "RIGHT JOIN",
            Self::FullOuter => "FULL OUTER JOIN",
        }
    }
}

impl<'a> ModelQuery<'a> {
    pub fn new(pool: &'a PgPool, table: &'a str) -> Self {
        Self {
            pool,
            table,
            filters: Vec::new(),
            having_filters: Vec::new(),
            joins: Vec::new(),
            group_by_fields: Vec::new(),
            select_fields: None,
            order: None,
            page: None,
            limit: None,
            custom_hooks: Vec::new(),
        }
    }
    pub fn filter(mut self, field: &str, op: Operator, value: &str) -> Self {
        self.filters.push((field.to_owned(), op, value.to_owned()));
        self
    }
    pub fn having(mut self, field: &str, op: Operator, value: &str) -> Self {
        self.having_filters
            .push((field.to_owned(), op, value.to_owned()));
        self
    }
    pub fn join(mut self, join_type: JoinType, table: &str, on_left: &str, on_right: &str) -> Self {
        self.joins.push(Join {
            join_type,
            table: table.to_owned(),
            on_left: on_left.to_owned(),
            on_right: on_right.to_owned(),
        });
        self
    }
    pub fn group_by(mut self, fields: &[&str]) -> Self {
        self.group_by_fields = fields.iter().map(|v| (*v).to_owned()).collect();
        self
    }
    pub fn select(mut self, fields: &[&str]) -> Self {
        self.select_fields = Some(fields.iter().map(|v| (*v).to_owned()).collect());
        self
    }
    pub fn order_by(mut self, field: &str, desc: bool) -> Self {
        self.order = Some((field.to_owned(), desc));
        self
    }
    pub fn paginate(mut self, page: i64, page_size: i64) -> Self {
        self.page = Some((page, page_size));
        self
    }
    pub fn limit(mut self, count: i64) -> Self {
        self.limit = Some(count);
        self
    }
    pub fn with_hook(mut self, clause: &str) -> Self {
        self.custom_hooks.push(SqlHook {
            clause: clause.to_owned(),
        });
        self
    }

    fn build_query(
        &self,
        aggregate: Option<(&str, &Aggregate)>,
    ) -> Result<(String, Vec<String>), sqlx::Error> {
        let fields_sql = if let Some((field, func)) = aggregate {
            let field = if field == "*" {
                "*".to_owned()
            } else {
                ident(field)?
            };
            format!("{}({})", func.to_sql(), field)
        } else if let Some(fields) = &self.select_fields {
            fields
                .iter()
                .map(|field| projection(field))
                .collect::<Result<Vec<_>, _>>()?
                .join(", ")
        } else {
            "*".to_owned()
        };
        let mut sql = format!("SELECT {fields_sql} FROM {}", ident(self.table)?);
        let mut binds = Vec::new();
        let mut idx = 1usize;
        for join in &self.joins {
            sql.push_str(&format!(
                " {} {} ON {} = {}",
                join.join_type.to_sql(),
                ident(&join.table)?,
                ident(&join.on_left)?,
                ident(&join.on_right)?
            ));
        }
        if !self.filters.is_empty() {
            let mut clauses = Vec::new();
            for (field, op, value) in &self.filters {
                clauses.push(format!("{} {} ${idx}", ident(field)?, op.to_sql()));
                binds.push(value.clone());
                idx += 1;
            }
            sql.push_str(&format!(" WHERE {}", clauses.join(" AND ")));
        }
        if !self.group_by_fields.is_empty() {
            let fields = self
                .group_by_fields
                .iter()
                .map(|field| ident(field))
                .collect::<Result<Vec<_>, _>>()?;
            sql.push_str(&format!(" GROUP BY {}", fields.join(", ")));
        }
        if !self.having_filters.is_empty() {
            let mut clauses = Vec::new();
            for (field, op, value) in &self.having_filters {
                clauses.push(format!("{} {} ${idx}", ident(field)?, op.to_sql()));
                binds.push(value.clone());
                idx += 1;
            }
            sql.push_str(&format!(" HAVING {}", clauses.join(" AND ")));
        }
        if let Some((field, desc)) = &self.order {
            sql.push_str(&format!(
                " ORDER BY {} {}",
                ident(field)?,
                if *desc { "DESC" } else { "ASC" }
            ));
        }
        if let Some((page, size)) = self.page {
            if page < 1 || !(1..=10_000).contains(&size) {
                return Err(protocol(
                    "pagination requires page >= 1 and page_size 1..=10000",
                ));
            }
            sql.push_str(&format!(
                " LIMIT {size} OFFSET {}",
                (page - 1).saturating_mul(size)
            ));
        } else if let Some(limit) = self.limit {
            if !(1..=10_000).contains(&limit) {
                return Err(protocol("limit must be 1..=10000"));
            }
            sql.push_str(&format!(" LIMIT {limit}"));
        }
        for hook in &self.custom_hooks {
            sql.push(' ');
            sql.push_str(validated_hook(&hook.clause)?);
        }
        Ok((sql, binds))
    }

    pub async fn all<T>(self) -> Result<Vec<T>, sqlx::Error>
    where
        T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
    {
        let (sql, params) = self.build_query(None)?;
        let mut q = query_as::<_, T>(AssertSqlSafe(sql));
        for param in params {
            q = q.bind(param);
        }
        q.fetch_all(self.pool).await
    }
    pub async fn first<T>(self) -> Result<Option<T>, sqlx::Error>
    where
        T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
    {
        self.limit(1).all().await.map(|mut rows| rows.pop())
    }
    pub async fn count(self) -> Result<i64, sqlx::Error> {
        let (sql, params) = self.build_query(Some(("*", &Aggregate::Count)))?;
        let mut q = query_as::<_, (i64,)>(AssertSqlSafe(sql));
        for p in params {
            q = q.bind(p);
        }
        Ok(q.fetch_one(self.pool).await?.0)
    }
    pub async fn aggregate(self, field: &str, func: Aggregate) -> Result<Option<f64>, sqlx::Error> {
        let (sql, params) = self.build_query(Some((field, &func)))?;
        let mut q = query_as::<_, (Option<f64>,)>(AssertSqlSafe(sql));
        for p in params {
            q = q.bind(p);
        }
        Ok(q.fetch_one(self.pool).await?.0)
    }
    pub fn insert(pool: &'a PgPool, table: &'a str) -> InsertBuilder<'a> {
        InsertBuilder::new(pool, table)
    }
}

impl<'a> InsertBuilder<'a> {
    pub fn new(pool: &'a PgPool, table: &'a str) -> Self {
        Self {
            pool,
            table,
            fields: Vec::new(),
            values: Vec::new(),
        }
    }
    pub fn set(mut self, field: &str, value: &str) -> Self {
        self.fields.push(field.to_owned());
        self.values.push(value.to_owned());
        self
    }
    pub async fn execute(self) -> Result<(), sqlx::Error> {
        if self.fields.is_empty() {
            return Err(protocol("insert requires at least one field"));
        }
        let fields = self
            .fields
            .iter()
            .map(|f| ident(f))
            .collect::<Result<Vec<_>, _>>()?;
        let placeholders = (1..=fields.len())
            .map(|i| format!("${i}"))
            .collect::<Vec<_>>();
        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            ident(self.table)?,
            fields.join(", "),
            placeholders.join(", ")
        );
        let mut q = query(AssertSqlSafe(sql));
        for value in self.values {
            q = q.bind(value);
        }
        q.execute(self.pool).await?;
        Ok(())
    }
    pub fn upsert_on(self, conflict_field: &str, update_fields: &[&str]) -> UpsertBuilder<'a> {
        UpsertBuilder {
            base: self,
            conflict_field: conflict_field.to_owned(),
            update_fields: update_fields.iter().map(|v| (*v).to_owned()).collect(),
        }
    }
}

impl<'a> UpsertBuilder<'a> {
    pub async fn execute(self) -> Result<(), sqlx::Error> {
        if self.base.fields.is_empty() {
            return Err(protocol("upsert requires at least one field"));
        }
        if self.update_fields.is_empty() {
            return Err(protocol("upsert requires update fields"));
        }
        let fields = self
            .base
            .fields
            .iter()
            .map(|f| ident(f))
            .collect::<Result<Vec<_>, _>>()?;
        let placeholders = (1..=fields.len())
            .map(|i| format!("${i}"))
            .collect::<Vec<_>>();
        let updates = self
            .update_fields
            .iter()
            .map(|f| {
                let q = ident(f)?;
                Ok(format!("{q} = EXCLUDED.{q}"))
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?;
        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({}) ON CONFLICT ({}) DO UPDATE SET {}",
            ident(self.base.table)?,
            fields.join(", "),
            placeholders.join(", "),
            ident(&self.conflict_field)?,
            updates.join(", ")
        );
        let mut q = query(AssertSqlSafe(sql));
        for value in self.base.values {
            q = q.bind(value);
        }
        q.execute(self.base.pool).await?;
        Ok(())
    }
}

impl<'a> DeleteQuery<'a> {
    pub fn new(pool: &'a PgPool, table: &'a str) -> Self {
        Self {
            pool,
            table,
            ids: Vec::new(),
            filters: Vec::new(),
        }
    }
    pub fn where_id(mut self, id: &str) -> Self {
        self.ids.push(id.to_owned());
        self
    }
    pub fn filter(mut self, field: &str, op: Operator, value: &str) -> Self {
        self.filters.push((field.to_owned(), op, value.to_owned()));
        self
    }
    pub async fn execute(self) -> Result<(), sqlx::Error> {
        if self.ids.is_empty() && self.filters.is_empty() {
            return Err(protocol("refusing unscoped DELETE"));
        }
        let mut sql = format!("DELETE FROM {}", ident(self.table)?);
        let mut clauses = Vec::new();
        let mut binds = Vec::new();
        if !self.ids.is_empty() {
            let ph = (1..=self.ids.len())
                .map(|i| format!("${i}"))
                .collect::<Vec<_>>()
                .join(", ");
            clauses.push(format!("{} IN ({ph})", ident("id")?));
            binds.extend(self.ids);
        }
        let offset = binds.len();
        for (i, (field, op, value)) in self.filters.into_iter().enumerate() {
            clauses.push(format!(
                "{} {} ${}",
                ident(&field)?,
                op.to_sql(),
                offset + i + 1
            ));
            binds.push(value);
        }
        sql.push_str(&format!(" WHERE {}", clauses.join(" AND ")));
        let mut q = query(AssertSqlSafe(sql));
        for value in binds {
            q = q.bind(value);
        }
        q.execute(self.pool).await?;
        Ok(())
    }
}

impl<'a> BatchUpdate<'a> {
    pub fn new(pool: &'a PgPool, table: &'a str, key_field: &'a str) -> Self {
        Self {
            pool,
            table,
            key_field,
            updates: HashMap::new(),
        }
    }
    pub fn set(mut self, key_val: &str, field: &str, value: &str) -> Self {
        self.updates
            .entry(key_val.to_owned())
            .or_default()
            .insert(field.to_owned(), value.to_owned());
        self
    }
    pub async fn execute(self) -> Result<(), sqlx::Error> {
        if self.updates.is_empty() {
            return Ok(());
        }
        let table = ident(self.table)?;
        let key_field = ident(self.key_field)?;
        let mut keys = self.updates.keys().cloned().collect::<Vec<_>>();
        keys.sort();
        let mut fields = HashSet::new();
        for values in self.updates.values() {
            fields.extend(values.keys().cloned());
        }
        let mut fields = fields.into_iter().collect::<Vec<_>>();
        fields.sort();
        let mut binds = Vec::new();
        let mut clauses = Vec::new();
        for field in fields {
            let field_sql = ident(&field)?;
            let mut case = format!("{field_sql} = CASE");
            for key in &keys {
                if let Some(value) = self.updates.get(key).and_then(|v| v.get(&field)) {
                    let a = binds.len() + 1;
                    binds.push(key.clone());
                    let b = binds.len() + 1;
                    binds.push(value.clone());
                    case.push_str(&format!(" WHEN {key_field} = ${a} THEN ${b}"));
                }
            }
            case.push_str(&format!(" ELSE {field_sql} END"));
            clauses.push(case);
        }
        let key_params = keys
            .iter()
            .enumerate()
            .map(|(i, _)| format!("${}", binds.len() + i + 1))
            .collect::<Vec<_>>();
        binds.extend(keys);
        let sql = format!(
            "UPDATE {table} SET {} WHERE {key_field} IN ({})",
            clauses.join(", "),
            key_params.join(", ")
        );
        let mut q = query(AssertSqlSafe(sql));
        for value in binds {
            q = q.bind(value);
        }
        q.execute(self.pool).await?;
        Ok(())
    }
}

impl<'a> BulkInsert<'a> {
    pub fn new(pool: &'a PgPool, table: &'a str, fields: &[&str]) -> Self {
        Self {
            pool,
            table,
            fields: fields.iter().map(|v| (*v).to_owned()).collect(),
            rows: Vec::new(),
        }
    }
    pub fn add_row(mut self, values: &[&str]) -> Self {
        self.rows
            .push(values.iter().map(|v| (*v).to_owned()).collect());
        self
    }
    pub async fn execute(self) -> Result<(), sqlx::Error> {
        if self.fields.is_empty() || self.rows.is_empty() {
            return Err(protocol("bulk insert requires fields and rows"));
        }
        if self.rows.iter().any(|r| r.len() != self.fields.len()) {
            return Err(protocol("bulk insert row width does not match field count"));
        }
        let fields = self
            .fields
            .iter()
            .map(|f| ident(f))
            .collect::<Result<Vec<_>, _>>()?;
        let mut binds = Vec::new();
        let mut rows_sql = Vec::new();
        let mut idx = 1;
        for row in self.rows {
            let mut ph = Vec::new();
            for value in row {
                ph.push(format!("${idx}"));
                idx += 1;
                binds.push(value);
            }
            rows_sql.push(format!("({})", ph.join(", ")));
        }
        let sql = format!(
            "INSERT INTO {} ({}) VALUES {}",
            ident(self.table)?,
            fields.join(", "),
            rows_sql.join(", ")
        );
        let mut q = query(AssertSqlSafe(sql));
        for value in binds {
            q = q.bind(value);
        }
        q.execute(self.pool).await?;
        Ok(())
    }
}

pub async fn save_row(
    pool: &PgPool,
    table: &str,
    primary_key: &str,
    row: &HashMap<&str, &str>,
) -> Result<(), sqlx::Error> {
    if row.is_empty() {
        return Err(protocol("save_row requires at least one field"));
    }
    let mut entries = row.iter().collect::<Vec<_>>();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    let mut insert = InsertBuilder::new(pool, table);
    for (field, value) in &entries {
        insert = insert.set(field, value);
    }
    let update_fields = entries
        .iter()
        .map(|(field, _)| **field)
        .filter(|field| *field != primary_key)
        .collect::<Vec<_>>();
    if update_fields.is_empty() {
        return insert.execute().await;
    }
    insert
        .upsert_on(primary_key, &update_fields)
        .execute()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pool() -> PgPool {
        PgPool::connect_lazy("postgres://localhost/testdb")
            .unwrap_or_else(|e| panic!("pool init: {e}"))
    }
    #[tokio::test]
    async fn rejects_identifier_injection() {
        let p = pool();
        let q = ModelQuery::new(&p, "users; DROP TABLE users");
        assert!(q.build_query(None).is_err());
    }
    #[tokio::test]
    async fn rejects_unsafe_hook() {
        let p = pool();
        let q = ModelQuery::new(&p, "users").with_hook("; DROP TABLE users");
        assert!(q.build_query(None).is_err());
    }
    #[tokio::test]
    async fn refuses_unbounded_pagination() {
        let p = pool();
        let q = ModelQuery::new(&p, "users").paginate(1, 10001);
        assert!(q.build_query(None).is_err());
    }
    #[tokio::test]
    async fn quotes_safe_query_identifiers() {
        let p = pool();
        let q = ModelQuery::new(&p, "public.users")
            .filter("email", Operator::Eq, "a@example.com")
            .order_by("created_at", true);
        let (sql, binds) = q.build_query(None).unwrap_or_else(|e| panic!("build: {e}"));
        assert_eq!(
            sql,
            "SELECT * FROM \"public\".\"users\" WHERE \"email\" = $1 ORDER BY \"created_at\" DESC"
        );
        assert_eq!(binds, vec!["a@example.com"]);
    }
}
