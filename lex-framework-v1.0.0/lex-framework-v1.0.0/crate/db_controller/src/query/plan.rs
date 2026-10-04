//! Internal query plan and safe defaults.

use super::*;

#[derive(Debug, Clone)]
pub(super) struct QueryPlan<T> {
    pub(super) predicate: Option<Predicate>,
    pub(super) user_predicate: bool,
    pub(super) ordering: Vec<OrderBy>,
    pub(super) use_default_ordering: bool,
    pub(super) limit: Option<u32>,
    pub(super) offset: u64,
    pub(super) distinct: bool,
    pub(super) include_deleted: bool,
    pub(super) only_deleted: bool,
    pub(super) tenant: Option<DbValue>,
    pub(super) global_scope: bool,
    pub(super) authorized_global_models: BTreeSet<&'static str>,
    pub(super) lock: Option<(RowLock, LockWait)>,
    pub(super) allow_all_mutation: bool,
    pub(super) timeout: Option<Duration>,
    pub(super) force_empty: bool,
    pub(super) _marker: PhantomData<T>,
}

impl<T> Default for QueryPlan<T> {
    fn default() -> Self {
        Self {
            predicate: None,
            user_predicate: false,
            ordering: Vec::new(),
            use_default_ordering: true,
            limit: None,
            offset: 0,
            distinct: false,
            include_deleted: false,
            only_deleted: false,
            tenant: None,
            global_scope: false,
            authorized_global_models: BTreeSet::new(),
            lock: None,
            allow_all_mutation: false,
            timeout: None,
            force_empty: false,
            _marker: PhantomData,
        }
    }
}
