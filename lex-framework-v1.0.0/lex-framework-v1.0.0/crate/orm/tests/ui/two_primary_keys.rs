use orm::Model;

#[derive(Model)]
struct Invalid {
    #[field(primary_key)]
    id: i64,
    #[field(primary_key)]
    other: i64,
}

fn main() {}
