//! Portable ordering descriptors.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NullOrdering {
    DatabaseDefault,
    First,
    Last,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderBy {
    pub field: String,
    pub direction: SortDirection,
    pub nulls: NullOrdering,
}

impl OrderBy {
    pub fn asc(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            direction: SortDirection::Asc,
            nulls: NullOrdering::DatabaseDefault,
        }
    }

    pub fn desc(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            direction: SortDirection::Desc,
            nulls: NullOrdering::DatabaseDefault,
        }
    }

    pub fn nulls_first(mut self) -> Self {
        self.nulls = NullOrdering::First;
        self
    }

    pub fn nulls_last(mut self) -> Self {
        self.nulls = NullOrdering::Last;
        self
    }
}
