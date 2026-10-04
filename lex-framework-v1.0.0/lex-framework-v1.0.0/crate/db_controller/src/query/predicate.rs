//! Typed predicates, field descriptors and update expressions.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonOperator {
    Eq,
    NotEq,
    Gt,
    Gte,
    Lt,
    Lte,
}

impl ComparisonOperator {
    pub(super) const fn as_sql(self) -> &'static str {
        match self {
            Self::Eq => "=",
            Self::NotEq => "<>",
            Self::Gt => ">",
            Self::Gte => ">=",
            Self::Lt => "<",
            Self::Lte => "<=",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextLookup {
    Contains,
    IContains,
    StartsWith,
    IStartsWith,
    EndsWith,
    IEndsWith,
    Exact,
    IExact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatePart {
    Year,
    Quarter,
    Month,
    Week,
    Day,
    Hour,
    Minute,
    Second,
}

impl DatePart {
    pub(super) const fn as_sql(self) -> &'static str {
        match self {
            Self::Year => "YEAR",
            Self::Quarter => "QUARTER",
            Self::Month => "MONTH",
            Self::Week => "WEEK",
            Self::Day => "DAY",
            Self::Hour => "HOUR",
            Self::Minute => "MINUTE",
            Self::Second => "SECOND",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithmeticOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
}

impl ArithmeticOperator {
    pub(super) const fn as_sql(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Subtract => "-",
            Self::Multiply => "*",
            Self::Divide => "/",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum UpdateExpression {
    Set {
        field: String,
        value: DbValue,
    },
    Copy {
        field: String,
        source: String,
    },
    Arithmetic {
        field: String,
        operator: ArithmeticOperator,
        operand: DbValue,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Predicate {
    Always(bool),
    Comparison {
        field: String,
        operator: ComparisonOperator,
        value: DbValue,
    },
    FieldComparison {
        left: String,
        operator: ComparisonOperator,
        right: String,
    },
    IsNull {
        field: String,
        is_null: bool,
    },
    In {
        field: String,
        values: Vec<DbValue>,
        negated: bool,
    },
    Between {
        field: String,
        lower: DbValue,
        upper: DbValue,
    },
    Text {
        field: String,
        lookup: TextLookup,
        value: String,
    },
    DatePart {
        field: String,
        part: DatePart,
        operator: ComparisonOperator,
        value: i32,
    },
    JsonContains {
        field: String,
        value: Value,
    },
    JsonHasKey {
        field: String,
        key: String,
    },
    And(Vec<Predicate>),
    Or(Vec<Predicate>),
    Not(Box<Predicate>),
}

impl Predicate {
    pub fn and(self, other: Self) -> Self {
        match self {
            Self::And(mut values) => {
                values.push(other);
                Self::And(values)
            }
            value => Self::And(vec![value, other]),
        }
    }

    pub fn or(self, other: Self) -> Self {
        match self {
            Self::Or(mut values) => {
                values.push(other);
                Self::Or(values)
            }
            value => Self::Or(vec![value, other]),
        }
    }

    pub fn negate(self) -> Self {
        Self::Not(Box::new(self))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field<T> {
    name: &'static str,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Field<T> {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            _marker: PhantomData,
        }
    }

    pub const fn name(self) -> &'static str {
        self.name
    }

    pub fn eq<V: Into<DbValue>>(self, value: V) -> Predicate {
        self.compare(ComparisonOperator::Eq, value)
    }

    pub fn ne<V: Into<DbValue>>(self, value: V) -> Predicate {
        self.compare(ComparisonOperator::NotEq, value)
    }

    pub fn gt<V: Into<DbValue>>(self, value: V) -> Predicate {
        self.compare(ComparisonOperator::Gt, value)
    }

    pub fn gte<V: Into<DbValue>>(self, value: V) -> Predicate {
        self.compare(ComparisonOperator::Gte, value)
    }

    pub fn lt<V: Into<DbValue>>(self, value: V) -> Predicate {
        self.compare(ComparisonOperator::Lt, value)
    }

    pub fn lte<V: Into<DbValue>>(self, value: V) -> Predicate {
        self.compare(ComparisonOperator::Lte, value)
    }

    pub fn compare<V: Into<DbValue>>(self, operator: ComparisonOperator, value: V) -> Predicate {
        Predicate::Comparison {
            field: self.name.to_owned(),
            operator,
            value: value.into(),
        }
    }

    /// Compare this database column to another model column (Django F-expression equivalent).
    pub fn compare_field<U>(self, operator: ComparisonOperator, other: Field<U>) -> Predicate {
        Predicate::FieldComparison {
            left: self.name.to_owned(),
            operator,
            right: other.name.to_owned(),
        }
    }

    pub fn is_null(self) -> Predicate {
        Predicate::IsNull {
            field: self.name.to_owned(),
            is_null: true,
        }
    }

    pub fn is_not_null(self) -> Predicate {
        Predicate::IsNull {
            field: self.name.to_owned(),
            is_null: false,
        }
    }

    pub fn is_in<V: Into<DbValue>>(self, values: impl IntoIterator<Item = V>) -> Predicate {
        Predicate::In {
            field: self.name.to_owned(),
            values: values.into_iter().map(Into::into).collect(),
            negated: false,
        }
    }

    pub fn not_in<V: Into<DbValue>>(self, values: impl IntoIterator<Item = V>) -> Predicate {
        Predicate::In {
            field: self.name.to_owned(),
            values: values.into_iter().map(Into::into).collect(),
            negated: true,
        }
    }

    pub fn between<L: Into<DbValue>, U: Into<DbValue>>(self, lower: L, upper: U) -> Predicate {
        Predicate::Between {
            field: self.name.to_owned(),
            lower: lower.into(),
            upper: upper.into(),
        }
    }

    pub fn exact(self, value: impl Into<String>) -> Predicate {
        self.text(TextLookup::Exact, value)
    }
    pub fn iexact(self, value: impl Into<String>) -> Predicate {
        self.text(TextLookup::IExact, value)
    }
    pub fn contains(self, value: impl Into<String>) -> Predicate {
        self.text(TextLookup::Contains, value)
    }
    pub fn icontains(self, value: impl Into<String>) -> Predicate {
        self.text(TextLookup::IContains, value)
    }
    pub fn starts_with(self, value: impl Into<String>) -> Predicate {
        self.text(TextLookup::StartsWith, value)
    }
    pub fn istarts_with(self, value: impl Into<String>) -> Predicate {
        self.text(TextLookup::IStartsWith, value)
    }
    pub fn ends_with(self, value: impl Into<String>) -> Predicate {
        self.text(TextLookup::EndsWith, value)
    }
    pub fn iends_with(self, value: impl Into<String>) -> Predicate {
        self.text(TextLookup::IEndsWith, value)
    }

    /// Compare a DATE/TIMESTAMP field component without interpolating request SQL.
    pub fn date_part(self, part: DatePart, operator: ComparisonOperator, value: i32) -> Predicate {
        Predicate::DatePart {
            field: self.name.to_owned(),
            part,
            operator,
            value,
        }
    }

    /// PostgreSQL JSONB containment (`@>`). The query compiler rejects non-JSON fields.
    pub fn json_contains(self, value: Value) -> Predicate {
        Predicate::JsonContains {
            field: self.name.to_owned(),
            value,
        }
    }

    /// PostgreSQL JSONB top-level key existence (`?`).
    pub fn json_has_key(self, key: impl Into<String>) -> Predicate {
        Predicate::JsonHasKey {
            field: self.name.to_owned(),
            key: key.into(),
        }
    }

    pub fn set<V: Into<DbValue>>(self, value: V) -> UpdateExpression {
        UpdateExpression::Set {
            field: self.name.to_owned(),
            value: value.into(),
        }
    }

    pub fn copy_from<U>(self, source: Field<U>) -> UpdateExpression {
        UpdateExpression::Copy {
            field: self.name.to_owned(),
            source: source.name.to_owned(),
        }
    }

    /// Construct a database update expression, rather than performing Rust arithmetic.
    #[allow(
        clippy::should_implement_trait,
        reason = "query DSL returns a deferred SQL expression; preserve its named public builder"
    )]
    pub fn add<V: Into<DbValue>>(self, operand: V) -> UpdateExpression {
        self.arithmetic(ArithmeticOperator::Add, operand)
    }

    pub fn subtract<V: Into<DbValue>>(self, operand: V) -> UpdateExpression {
        self.arithmetic(ArithmeticOperator::Subtract, operand)
    }

    pub fn multiply<V: Into<DbValue>>(self, operand: V) -> UpdateExpression {
        self.arithmetic(ArithmeticOperator::Multiply, operand)
    }

    pub fn divide<V: Into<DbValue>>(self, operand: V) -> UpdateExpression {
        self.arithmetic(ArithmeticOperator::Divide, operand)
    }

    /// Order ascending by this typed field descriptor.
    pub fn asc(self) -> OrderBy {
        OrderBy::asc(self.name)
    }

    /// Order descending by this typed field descriptor.
    pub fn desc(self) -> OrderBy {
        OrderBy::desc(self.name)
    }

    fn arithmetic<V: Into<DbValue>>(
        self,
        operator: ArithmeticOperator,
        operand: V,
    ) -> UpdateExpression {
        UpdateExpression::Arithmetic {
            field: self.name.to_owned(),
            operator,
            operand: operand.into(),
        }
    }

    fn text(self, lookup: TextLookup, value: impl Into<String>) -> Predicate {
        Predicate::Text {
            field: self.name.to_owned(),
            lookup,
            value: value.into(),
        }
    }
}
