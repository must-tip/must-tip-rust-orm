mod apps;
mod settings;
mod urls;

use anyhow::{Context, Result};
use db::Database;
use lex_rest::rest_framework::{
    adapters::axum::AxumAdapter,
    middleware::{RestApplication, RestRuntimeBuilder},
    request::TransportSecurity,
};
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    settings::init_tracing();
    let settings = settings::load()?;
    apps::register_models();
    let database = Database::initialize_with(&settings.core).await?;
    let routes = urls::build_routes()?;

    let (authentication, _auth_service) = settings
        .authentication(&database)
        .await
        .context("failed to initialize framework authentication")?;
    let permissions = settings
        .permission_backend(&database)
        .context("failed to initialize framework permissions")?;
    let runtime = RestRuntimeBuilder::new(permissions)
        .authentication(authentication)
        .middleware(
            settings
                .rest_middleware()
                .context("invalid HTTP security settings")?,
        )
        .require_secure_transport(settings::require_secure_transport())
        .build()
        .context("failed to construct REST runtime")?;
    let application = Arc::new(RestApplication::new(routes, runtime));
    let security = if settings::require_secure_transport() {
        settings::assert_trusted_tls_boundary()?;
        TransportSecurity::Secure
    } else {
        TransportSecurity::Insecure
    };
    let router = AxumAdapter::new(application)
        .transport_security(security)
        .router()
        .context("failed to build HTTP router")?;
    let address = settings.core.server_addr()?;
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("failed to bind {address}"))?;

    info!(%address, backend = ?database.backend(), "Lex server started");
    let result = axum::serve(listener, router)
        .await
        .context("HTTP server failed");
    database.close().await;
    result
}
