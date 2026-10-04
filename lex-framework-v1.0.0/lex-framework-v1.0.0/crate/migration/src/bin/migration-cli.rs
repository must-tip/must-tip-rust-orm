#[tokio::main]
async fn main() -> anyhow::Result<()> {
    migration::run_migration_cli().await
}
