//! Transaction orchestration with savepoints, post-commit hooks, server-side deadlines and
//! bounded retries for PostgreSQL serialization/deadlock failures.

use std::{collections::BTreeSet, future::Future, pin::Pin, time::Duration};

use sqlx::{Error as SqlxError, PgPool, Postgres, Sqlite, SqlitePool, Transaction};
use uuid::Uuid;

use crate::{
    Entity, ModelError, QuerySet, SqliteTransactionExecutor, TransactionExecutor,
    clear_transaction_tenant, set_transaction_tenant,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationLevel {
    ReadCommitted,
    RepeatableRead,
    Serializable,
}

impl IsolationLevel {
    const fn as_sql(self) -> &'static str {
        match self {
            Self::ReadCommitted => "READ COMMITTED",
            Self::RepeatableRead => "REPEATABLE READ",
            Self::Serializable => "SERIALIZABLE",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TransactionOptions {
    pub isolation: IsolationLevel,
    pub read_only: bool,
    pub deferrable: bool,
    pub statement_timeout: Option<Duration>,
    pub lock_timeout: Option<Duration>,
    pub idle_in_transaction_timeout: Option<Duration>,
}

impl Default for TransactionOptions {
    fn default() -> Self {
        Self {
            isolation: IsolationLevel::ReadCommitted,
            read_only: false,
            deferrable: false,
            statement_timeout: Some(Duration::from_secs(30)),
            lock_timeout: Some(Duration::from_secs(5)),
            idle_in_transaction_timeout: Some(Duration::from_secs(60)),
        }
    }
}

impl TransactionOptions {
    fn validate(self) -> Result<Self, ModelError> {
        if self.deferrable && (!self.read_only || self.isolation != IsolationLevel::Serializable) {
            return Err(ModelError::InvalidQuery(
                "DEFERRABLE transactions must be SERIALIZABLE and READ ONLY".into(),
            ));
        }
        for (name, value) in [
            ("statement_timeout", self.statement_timeout),
            ("lock_timeout", self.lock_timeout),
            (
                "idle_in_transaction_timeout",
                self.idle_in_transaction_timeout,
            ),
        ] {
            if value.is_some_and(|value| value.is_zero()) {
                return Err(ModelError::InvalidQuery(format!("{name} must be non-zero")));
            }
        }
        Ok(self)
    }
}

type CommitFuture = Pin<Box<dyn Future<Output = Result<(), ModelError>> + Send + 'static>>;
type CommitCallback = Box<dyn FnOnce() -> CommitFuture + Send + 'static>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Savepoint {
    owner: Uuid,
    sequence: u32,
    name: String,
    callback_len: usize,
    active: bool,
}

pub struct AtomicTransaction<'pool> {
    transaction: Transaction<'pool, Postgres>,
    owner: Uuid,
    savepoint_sequence: u32,
    active_savepoints: BTreeSet<u32>,
    callbacks: Vec<CommitCallback>,
}

impl<'pool> AtomicTransaction<'pool> {
    pub async fn begin(
        pool: &'pool PgPool,
        options: TransactionOptions,
    ) -> Result<Self, ModelError> {
        let options = options.validate()?;
        let mut transaction = pool.begin().await?;

        let mut transaction_sql = format!(
            "SET TRANSACTION ISOLATION LEVEL {} {}",
            options.isolation.as_sql(),
            if options.read_only {
                "READ ONLY"
            } else {
                "READ WRITE"
            }
        );
        if options.deferrable {
            transaction_sql.push_str(" DEFERRABLE");
        } else {
            transaction_sql.push_str(" NOT DEFERRABLE");
        }
        sqlx::query(sqlx::AssertSqlSafe(transaction_sql))
            .execute(&mut *transaction)
            .await?;

        set_local_timeout(
            &mut transaction,
            "statement_timeout",
            options.statement_timeout,
        )
        .await?;
        set_local_timeout(&mut transaction, "lock_timeout", options.lock_timeout).await?;
        set_local_timeout(
            &mut transaction,
            "idle_in_transaction_session_timeout",
            options.idle_in_transaction_timeout,
        )
        .await?;

        Ok(Self {
            transaction,
            owner: Uuid::new_v4(),
            savepoint_sequence: 0,
            active_savepoints: BTreeSet::new(),
            callbacks: Vec::new(),
        })
    }

    pub fn queryset<T>(&mut self) -> QuerySet<T, TransactionExecutor<'_, 'pool>>
    where
        T: Entity,
    {
        QuerySet::from_executor(TransactionExecutor::new(&mut self.transaction))
    }

    pub fn raw(&mut self) -> &mut Transaction<'pool, Postgres> {
        &mut self.transaction
    }

    /// Bind the PostgreSQL row-level-security tenant context for this transaction.
    /// QuerySet tenant predicates remain enabled as defense in depth.
    pub async fn set_tenant(&mut self, tenant_id: Uuid) -> Result<(), ModelError> {
        set_transaction_tenant(&mut self.transaction, tenant_id).await
    }

    /// Clear any local tenant context previously assigned to this transaction.
    pub async fn clear_tenant(&mut self) -> Result<(), ModelError> {
        clear_transaction_tenant(&mut self.transaction).await
    }

    pub fn on_commit<F, Fut>(&mut self, callback: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), ModelError>> + Send + 'static,
    {
        self.callbacks.push(Box::new(move || Box::pin(callback())));
    }

    pub async fn create_savepoint(&mut self) -> Result<Savepoint, ModelError> {
        self.savepoint_sequence = self
            .savepoint_sequence
            .checked_add(1)
            .ok_or_else(|| ModelError::Other("savepoint sequence exhausted".into()))?;
        let sequence = self.savepoint_sequence;
        let name = format!("orm_sp_{sequence}");
        sqlx::query(sqlx::AssertSqlSafe(format!("SAVEPOINT \"{name}\"")))
            .execute(&mut *self.transaction)
            .await?;
        self.active_savepoints.insert(sequence);
        Ok(Savepoint {
            owner: self.owner,
            sequence,
            name,
            callback_len: self.callbacks.len(),
            active: true,
        })
    }

    pub async fn rollback_to(&mut self, savepoint: &Savepoint) -> Result<(), ModelError> {
        self.validate_savepoint(savepoint)?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "ROLLBACK TO SAVEPOINT \"{}\"",
            savepoint.name
        )))
        .execute(&mut *self.transaction)
        .await?;
        // PostgreSQL destroys savepoints created after the rollback target. Callbacks registered
        // after the target also belong to work that no longer exists and must not run on commit.
        self.active_savepoints
            .retain(|sequence| *sequence <= savepoint.sequence);
        self.callbacks.truncate(savepoint.callback_len);
        Ok(())
    }

    pub async fn release_savepoint(&mut self, savepoint: &mut Savepoint) -> Result<(), ModelError> {
        self.validate_savepoint(savepoint)?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "RELEASE SAVEPOINT \"{}\"",
            savepoint.name
        )))
        .execute(&mut *self.transaction)
        .await?;
        // RELEASE also releases savepoints established after the named savepoint.
        self.active_savepoints
            .retain(|sequence| *sequence < savepoint.sequence);
        savepoint.active = false;
        Ok(())
    }

    pub async fn commit(self) -> Result<(), ModelError> {
        let Self {
            transaction,
            callbacks,
            ..
        } = self;
        transaction.commit().await?;
        for callback in callbacks {
            if let Err(error) = callback().await {
                return Err(ModelError::PostCommit(error.to_string()));
            }
        }
        Ok(())
    }

    pub async fn rollback(self) -> Result<(), ModelError> {
        self.transaction.rollback().await.map_err(ModelError::from)
    }

    fn validate_savepoint(&self, savepoint: &Savepoint) -> Result<(), ModelError> {
        if savepoint.owner != self.owner {
            return Err(ModelError::InvalidQuery(
                "savepoint belongs to a different transaction".into(),
            ));
        }
        if !savepoint.active || !self.active_savepoints.contains(&savepoint.sequence) {
            return Err(ModelError::InvalidQuery(
                "savepoint is no longer active in this transaction".into(),
            ));
        }
        Ok(())
    }
}

/// SQLite transaction wrapper with the same QuerySet surface as AtomicTransaction.
/// PostgreSQL-only RLS/SET TRANSACTION features are intentionally not emulated.
pub struct SqliteAtomicTransaction<'pool> {
    transaction: Transaction<'pool, Sqlite>,
    owner: Uuid,
    savepoint_sequence: u32,
    active_savepoints: BTreeSet<u32>,
    callbacks: Vec<CommitCallback>,
}

impl<'pool> SqliteAtomicTransaction<'pool> {
    pub async fn begin(
        pool: &'pool SqlitePool,
        options: TransactionOptions,
    ) -> Result<Self, ModelError> {
        let options = options.validate()?;
        if options.deferrable {
            return Err(ModelError::InvalidQuery(
                "SQLite does not support DEFERRABLE transactions".into(),
            ));
        }
        if options.read_only {
            return Err(ModelError::InvalidQuery(
                "read_only transaction mode is PostgreSQL-specific in this framework; use a read-only SQLite connection when required".into(),
            ));
        }
        let transaction = pool.begin().await?;
        Ok(Self {
            transaction,
            owner: Uuid::new_v4(),
            savepoint_sequence: 0,
            active_savepoints: BTreeSet::new(),
            callbacks: Vec::new(),
        })
    }

    pub fn queryset<T>(&mut self) -> QuerySet<T, SqliteTransactionExecutor<'_, 'pool>>
    where
        T: Entity,
    {
        QuerySet::from_executor(SqliteTransactionExecutor::new(&mut self.transaction))
    }

    pub fn raw(&mut self) -> &mut Transaction<'pool, Sqlite> {
        &mut self.transaction
    }

    pub fn on_commit<F, Fut>(&mut self, callback: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), ModelError>> + Send + 'static,
    {
        self.callbacks.push(Box::new(move || Box::pin(callback())));
    }

    pub async fn create_savepoint(&mut self) -> Result<Savepoint, ModelError> {
        self.savepoint_sequence = self
            .savepoint_sequence
            .checked_add(1)
            .ok_or_else(|| ModelError::Other("savepoint sequence exhausted".into()))?;
        let sequence = self.savepoint_sequence;
        let name = format!("orm_sp_{sequence}");
        sqlx::query(sqlx::AssertSqlSafe(format!("SAVEPOINT \"{name}\"")))
            .execute(&mut *self.transaction)
            .await?;
        self.active_savepoints.insert(sequence);
        Ok(Savepoint {
            owner: self.owner,
            sequence,
            name,
            callback_len: self.callbacks.len(),
            active: true,
        })
    }

    pub async fn rollback_to(&mut self, savepoint: &Savepoint) -> Result<(), ModelError> {
        self.validate_savepoint(savepoint)?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "ROLLBACK TO SAVEPOINT \"{}\"",
            savepoint.name
        )))
        .execute(&mut *self.transaction)
        .await?;
        self.active_savepoints
            .retain(|sequence| *sequence <= savepoint.sequence);
        self.callbacks.truncate(savepoint.callback_len);
        Ok(())
    }

    pub async fn release_savepoint(&mut self, savepoint: &mut Savepoint) -> Result<(), ModelError> {
        self.validate_savepoint(savepoint)?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "RELEASE SAVEPOINT \"{}\"",
            savepoint.name
        )))
        .execute(&mut *self.transaction)
        .await?;
        self.active_savepoints
            .retain(|sequence| *sequence < savepoint.sequence);
        savepoint.active = false;
        Ok(())
    }

    pub async fn commit(self) -> Result<(), ModelError> {
        let Self {
            transaction,
            callbacks,
            ..
        } = self;
        transaction.commit().await?;
        for callback in callbacks {
            if let Err(error) = callback().await {
                return Err(ModelError::PostCommit(error.to_string()));
            }
        }
        Ok(())
    }

    pub async fn rollback(self) -> Result<(), ModelError> {
        self.transaction.rollback().await.map_err(ModelError::from)
    }

    fn validate_savepoint(&self, savepoint: &Savepoint) -> Result<(), ModelError> {
        if savepoint.owner != self.owner {
            return Err(ModelError::InvalidQuery(
                "savepoint belongs to a different transaction".into(),
            ));
        }
        if !savepoint.active || !self.active_savepoints.contains(&savepoint.sequence) {
            return Err(ModelError::InvalidQuery(
                "savepoint is no longer active in this transaction".into(),
            ));
        }
        Ok(())
    }
}

async fn set_local_timeout(
    transaction: &mut Transaction<'_, Postgres>,
    key: &str,
    value: Option<Duration>,
) -> Result<(), ModelError> {
    let Some(value) = value else {
        return Ok(());
    };
    let millis = value.as_millis().min(i64::MAX as u128) as i64;
    sqlx::query("SELECT set_config($1, $2, true)")
        .bind(key)
        .bind(format!("{millis}ms"))
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(25),
            max_backoff: Duration::from_millis(500),
        }
    }
}

impl RetryPolicy {
    fn validate(self) -> Result<Self, ModelError> {
        if self.max_attempts == 0 || self.max_attempts > 10 {
            return Err(ModelError::InvalidQuery(
                "max_attempts must be 1..=10".into(),
            ));
        }
        if self.initial_backoff.is_zero() || self.max_backoff.is_zero() {
            return Err(ModelError::InvalidQuery(
                "retry backoff must be non-zero".into(),
            ));
        }
        if self.initial_backoff > self.max_backoff {
            return Err(ModelError::InvalidQuery(
                "initial_backoff cannot exceed max_backoff".into(),
            ));
        }
        Ok(self)
    }
}

pub type TransactionFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, ModelError>> + Send + 'a>>;

/// Retry a complete transaction only for PostgreSQL serialization failures (40001) and deadlocks
/// (40P01). The operation must therefore be idempotent with respect to external side effects.
pub async fn retry_transaction<T, F>(
    pool: &PgPool,
    options: TransactionOptions,
    policy: RetryPolicy,
    mut operation: F,
) -> Result<T, ModelError>
where
    T: Send,
    F: for<'a, 'tx> FnMut(&'a mut Transaction<'tx, Postgres>) -> TransactionFuture<'a, T> + Send,
{
    let policy = policy.validate()?;
    let options = options.validate()?;
    let mut backoff = policy.initial_backoff;

    for attempt in 1..=policy.max_attempts {
        let mut atomic = AtomicTransaction::begin(pool, options).await?;
        let result = operation(atomic.raw()).await;
        match result {
            Ok(value) => match atomic.commit().await {
                Ok(()) => return Ok(value),
                Err(error)
                    if is_retryable_database_error(&error) && attempt < policy.max_attempts => {}
                Err(error) => return Err(error),
            },
            Err(error) => {
                let retryable = is_retryable_database_error(&error);
                let rollback_result = atomic.rollback().await;
                if let Err(rollback_error) = rollback_result {
                    return Err(ModelError::Other(format!(
                        "transaction operation failed ({error}); rollback also failed ({rollback_error})"
                    )));
                }
                if !retryable || attempt == policy.max_attempts {
                    return Err(error);
                }
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = backoff.saturating_mul(2).min(policy.max_backoff);
    }

    Err(ModelError::Other("transaction retry loop exhausted".into()))
}

fn is_retryable_database_error(error: &ModelError) -> bool {
    let ModelError::Database(SqlxError::Database(database)) = error else {
        return false;
    };
    matches!(database.code().as_deref(), Some("40001" | "40P01"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transaction_options_reject_invalid_deferrable_mode() {
        let options = TransactionOptions {
            deferrable: true,
            ..TransactionOptions::default()
        };
        assert!(options.validate().is_err());

        let options = TransactionOptions {
            isolation: IsolationLevel::Serializable,
            read_only: true,
            deferrable: true,
            ..TransactionOptions::default()
        };
        assert!(options.validate().is_ok());
    }

    #[test]
    fn retry_policy_is_bounded() {
        assert!(
            RetryPolicy {
                max_attempts: 0,
                ..RetryPolicy::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            RetryPolicy {
                max_attempts: 11,
                ..RetryPolicy::default()
            }
            .validate()
            .is_err()
        );
        assert!(RetryPolicy::default().validate().is_ok());
    }
}
