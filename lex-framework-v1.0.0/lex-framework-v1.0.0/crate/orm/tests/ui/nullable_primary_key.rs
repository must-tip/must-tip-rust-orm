use db_controller::UUIDField;
use orm::Model;
#[derive(Model)]
struct Broken {
    #[field(primary_key)]
    id: Option<UUIDField>,
}
fn main() {}
