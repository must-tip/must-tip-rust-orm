use anyhow::{Result, bail};
use lex_framework::FrameworkSettings;
use std::env;
use tracing_subscriber::EnvFilter;

pub fn init_tracing() {
    let filter = env::var("RUST_LOG").unwrap_or_else(|_| "info".to_owned());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .try_init();
}
pub fn load() -> Result<FrameworkSettings> {
    FrameworkSettings::load()
}
pub fn is_production() -> bool {
    lex_framework::is_production()
}
pub fn require_secure_transport() -> bool {
    is_production()
}
pub fn assert_trusted_tls_boundary() -> Result<()> {
    if !is_production() {
        return Ok(());
    }
    let ok = env::var("LEX_TRUSTED_TLS_TERMINATION")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false);
    if !ok {
        bail!(
            "production startup requires TLS; set LEX_TRUSTED_TLS_TERMINATION=true only behind a trusted TLS ingress/proxy"
        )
    }
    Ok(())
}
