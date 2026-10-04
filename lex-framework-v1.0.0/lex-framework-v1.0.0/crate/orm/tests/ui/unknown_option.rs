use orm::Model;

#[derive(Model)]
struct Invalid {
    #[field(magic)]
    id: i64,
}

fn main() {}
