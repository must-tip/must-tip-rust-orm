# Models and QuerySets

Models are declared with the framework's derive layer and become the source of metadata used by persistence, relations, schema generation, and higher-level framework facilities.

## Model example

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "customers")]
#[timestamps]
pub struct Customer {
    #[field(primary_key)]
    pub id: i64,

    #[field(unique, index)]
    pub email: String,

    pub active: bool,
}
```

## QuerySet

The QuerySet API is composable and defers SQL construction until execution.

```rust
let customers = Customer::objects(&pool)
    .filter(Customer::ACTIVE.eq(true))
    .order_by(Customer::ID.desc())
    .limit(50)
    .all()
    .await?;
```

Common operations include filtering, exclusion, ordering, pagination, projections, counts, existence checks, creates, bulk operations, updates, deletes, soft-delete workflows, relation loading, aggregates, and transaction-aware querying.

## Relations

Use explicit relationship loading rather than implicit lazy queries. This keeps query behavior visible and avoids accidental N+1 request patterns.

## Tenant-aware models

Models can expose tenant metadata so QuerySets can enforce application-level tenant scope. PostgreSQL deployments can additionally use row-level security for defense in depth.
