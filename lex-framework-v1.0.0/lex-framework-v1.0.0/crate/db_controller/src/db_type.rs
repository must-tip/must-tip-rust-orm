use sqlx::PgPool;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Aggregate {
    Count,
    Sum,
    Avg,
    Max,
    Min,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operator {
    Eq,
    Neq,
    Gt,
    Gte,
    Lt,
    Lte,
    Like,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinType {
    Inner,
    Left,
    Right,
    FullOuter,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Join {
    pub join_type: JoinType,
    pub table: String,
    pub on_left: String,
    pub on_right: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlHook {
    pub clause: String,
}

#[derive(Clone)]
pub struct ModelQuery<'a> {
    pub table: &'a str,
    pub filters: Vec<(String, Operator, String)>,
    pub having_filters: Vec<(String, Operator, String)>,
    pub joins: Vec<Join>,
    pub group_by_fields: Vec<String>,
    pub select_fields: Option<Vec<String>>,
    pub order: Option<(String, bool)>,
    pub page: Option<(i64, i64)>,
    pub limit: Option<i64>,
    pub custom_hooks: Vec<SqlHook>,
    pub pool: &'a PgPool,
}

pub struct InsertBuilder<'a> {
    pub table: &'a str,
    pub fields: Vec<String>,
    pub values: Vec<String>,
    pub pool: &'a PgPool,
}

pub struct UpsertBuilder<'a> {
    pub base: InsertBuilder<'a>,
    pub conflict_field: String,
    pub update_fields: Vec<String>,
}

pub struct DeleteQuery<'a> {
    pub table: &'a str,
    pub ids: Vec<String>,
    pub pool: &'a PgPool,
    pub filters: Vec<(String, Operator, String)>,
}

pub struct BatchUpdate<'a> {
    pub table: &'a str,
    pub key_field: &'a str,
    pub updates: HashMap<String, HashMap<String, String>>,
    pub pool: &'a PgPool,
}

pub struct BulkInsert<'a> {
    pub table: &'a str,
    pub fields: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub pool: &'a PgPool,
}

pub async fn save_row(
    pool: &PgPool,
    table: &str,
    primary_key: &str,
    row: &HashMap<&str, &str>,
) -> Result<(), sqlx::Error> {
    crate::query_builder::save_row(pool, table, primary_key, row).await
}
