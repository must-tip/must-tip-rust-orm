//! Live normalized RBAC lookups. No token snapshot or process cache can retain removed grants.

use super::{MembershipSecurityRecord, PermissionRepository, UserSecurityRecord};
use crate::rest_framework::exceptions::{ApiError, ApiResult};
use async_trait::async_trait;
use sqlx::{FromRow, postgres::PgRow, sqlite::SqliteRow};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Clone)]
enum Pool {
    Postgres(sqlx::PgPool),
    Sqlite(sqlx::SqlitePool),
}

/// Database-backed normalized roles for PostgreSQL and SQLite.
/// Install `migration::rbac` first. Missing schema fails closed as a server error.
#[derive(Clone)]
pub struct RbacPermissionRepository {
    pool: Pool,
}

impl RbacPermissionRepository {
    /// Use the primary PostgreSQL pool for fresh authorization decisions.
    pub fn postgres(pool: sqlx::PgPool) -> Self {
        Self {
            pool: Pool::Postgres(pool),
        }
    }
    /// Use a SQLite pool configured with foreign keys enabled on every connection.
    pub fn sqlite(pool: sqlx::SqlitePool) -> Self {
        Self {
            pool: Pool::Sqlite(pool),
        }
    }

    async fn rows<T>(&self, sql: &str, values: &[String]) -> ApiResult<Vec<T>>
    where
        T: for<'r> FromRow<'r, PgRow> + for<'r> FromRow<'r, SqliteRow> + Send + Unpin,
    {
        match &self.pool {
            Pool::Postgres(pool) => {
                let mut query = sqlx::query_as::<_, T>(sqlx::AssertSqlSafe(sql));
                for value in values {
                    query = query.bind(value);
                }
                query.fetch_all(pool).await.map_err(ApiError::from_storage)
            }
            Pool::Sqlite(pool) => {
                let sql = sql.replace("::uuid", "");
                let mut query = sqlx::query_as::<_, T>(sqlx::AssertSqlSafe(sql));
                for value in values {
                    query = query.bind(value);
                }
                query.fetch_all(pool).await.map_err(ApiError::from_storage)
            }
        }
    }
}

#[async_trait]
impl PermissionRepository for RbacPermissionRepository {
    async fn user_security(&self, user_id: Uuid) -> ApiResult<Option<UserSecurityRecord>> {
        let rows: Vec<(bool,)> = self
            .rows(
                "SELECT is_global_admin FROM users WHERE id=$1::uuid AND deleted_at IS NULL",
                &[user_id.to_string()],
            )
            .await?;
        Ok(rows
            .into_iter()
            .next()
            .map(|(is_global_admin,)| UserSecurityRecord { is_global_admin }))
    }
    async fn membership(
        &self,
        user_id: Uuid,
        tenant_id: Uuid,
    ) -> ApiResult<Option<MembershipSecurityRecord>> {
        let rows: Vec<(String, bool)> = self.rows(
            "SELECT CAST(m.id AS TEXT),m.is_tenant_admin FROM memberships m JOIN users u ON u.id=m.user_id JOIN tenants t ON t.id=m.tenant_id WHERE m.user_id=$1::uuid AND m.tenant_id=$2::uuid AND m.deleted_at IS NULL AND u.deleted_at IS NULL AND t.deleted_at IS NULL",
            &[user_id.to_string(), tenant_id.to_string()]
        ).await?;
        if rows.len() > 1 {
            return Err(ApiError::configuration(
                "duplicate membership violates RBAC integrity",
            ));
        }
        rows.into_iter()
            .next()
            .map(|(id, is_tenant_admin)| {
                let id = Uuid::parse_str(&id)
                    .map_err(|_| ApiError::internal("invalid stored membership identifier"))?;
                Ok(MembershipSecurityRecord {
                    id,
                    is_tenant_admin,
                })
            })
            .transpose()
    }
    async fn permission_names(
        &self,
        membership_id: Uuid,
        tenant_id: Uuid,
    ) -> ApiResult<BTreeSet<String>> {
        let rows: Vec<(String,)> = self.rows(
            "SELECT DISTINCT p.name FROM memberships m JOIN users u ON u.id=m.user_id JOIN tenants t ON t.id=m.tenant_id JOIN membership_roles a ON a.membership_id=m.id AND a.tenant_id=m.tenant_id JOIN tenant_roles r ON r.id=a.role_id AND r.tenant_id=a.tenant_id JOIN role_permissions g ON g.role_id=r.id AND g.tenant_id=r.tenant_id JOIN permission_definitions p ON p.id=g.permission_id WHERE m.id=$1::uuid AND m.tenant_id=$2::uuid AND m.deleted_at IS NULL AND u.deleted_at IS NULL AND t.deleted_at IS NULL",
            &[membership_id.to_string(), tenant_id.to_string()]
        ).await?;
        Ok(rows.into_iter().map(|(name,)| name).collect())
    }
}
