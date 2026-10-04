//! An application using only Lex's public facade and normal ecosystem dependencies.
use lex_framework::db_controller::{DateTimeField, TextField, UUIDField};
use lex_framework::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "acceptance_projects")]
pub struct Project {
    #[field(primary_key)]
    pub id: UUIDField,
    pub tenant_id: UUIDField,
    #[field(not_empty, max_length = 200)]
    pub title: TextField,
    #[field(soft_delete)]
    pub deleted_at: Option<DateTimeField>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "acceptance_budgets")]
#[model(global)]
pub struct Budget {
    #[field(primary_key)]
    pub id: UUIDField,
    pub amount: lex_framework::db_controller::DecimalField,
    pub adjustment: Option<lex_framework::db_controller::DecimalField>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "acceptance_tasks")]
#[model(global, version = "version")]
pub struct Task {
    #[field(primary_key)]
    pub id: UUIDField,
    pub title: TextField,
    #[field(default = 1)]
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "acceptance_project_profiles")]
#[model(unique(name = "profile_tenant_project", fields = "tenant_id,project_id"))]
pub struct ProjectProfile {
    #[field(primary_key)]
    pub id: UUIDField,
    pub tenant_id: UUIDField,
    pub project_id: UUIDField,
    #[relation(model = "Project", local = "project_id", remote = "id")]
    pub project: Option<Project>,
    pub active: lex_framework::db_controller::BooleanField,
    pub metadata: lex_framework::db_controller::JsonField<serde_json::Value>,
    pub amount: lex_framework::db_controller::DecimalField,
}
