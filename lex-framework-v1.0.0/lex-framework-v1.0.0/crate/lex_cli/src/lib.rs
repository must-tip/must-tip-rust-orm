pub mod ops;
mod project;
pub use project::{FrameworkSource, create_project};

use anyhow::{Context, Result, anyhow, bail};
use config::{Config, File};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    ffi::OsStr,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

const APPS_START: &str = "# lex:apps:start";
const APPS_END: &str = "# lex:apps:end";

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ProjectManifest {
    pub project_name: String,
    pub apps_dir: PathBuf,
    pub server_dir: PathBuf,
    pub server_package: String,
}

impl ProjectManifest {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join("lex.toml");
        if !path.is_file() {
            bail!(
                "{} is missing; run `lex create settings` first",
                path.display()
            );
        }
        let config = Config::builder()
            .add_source(File::from(path.clone()))
            .build()
            .with_context(|| format!("failed to parse {}", path.display()))?;
        let manifest: Self = config
            .try_deserialize()
            .with_context(|| format!("invalid project manifest {}", path.display()))?;
        validate_package_name(&manifest.server_package)?;
        validate_relative_path(&manifest.apps_dir, "apps_dir")?;
        validate_relative_path(&manifest.server_dir, "server_dir")?;
        Ok(manifest)
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct AppManifest {
    pub name: String,
    pub package: String,
    pub crate_ident: String,
    pub mount: String,
}

impl AppManifest {
    pub fn load(path: &Path) -> Result<Self> {
        let config = Config::builder()
            .add_source(File::from(path.to_path_buf()))
            .build()
            .with_context(|| format!("failed to parse {}", path.display()))?;
        let manifest: Self = config
            .try_deserialize()
            .with_context(|| format!("invalid app manifest {}", path.display()))?;
        validate_rust_ident(&manifest.crate_ident)?;
        validate_package_name(&manifest.package)?;
        validate_mount(&manifest.mount)?;
        Ok(manifest)
    }
}

#[derive(Debug, Clone)]
pub struct ScaffoldOptions {
    pub name: String,
    pub mount: Option<String>,
    pub force: bool,
}

#[derive(Debug, Clone)]
pub struct SettingsOptions {
    pub project_name: String,
    pub server_dir: PathBuf,
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    pub root: PathBuf,
    pub app_count: usize,
    pub issues: Vec<String>,
}

impl CheckReport {
    pub fn is_ok(&self) -> bool {
        self.issues.is_empty()
    }
}

pub fn find_workspace_root(start: impl AsRef<Path>) -> Result<PathBuf> {
    let start = start.as_ref();
    let mut current = if start.is_dir() {
        fs::canonicalize(start).with_context(|| format!("cannot access {}", start.display()))?
    } else {
        fs::canonicalize(start.parent().unwrap_or_else(|| Path::new(".")))?
    };
    loop {
        let cargo = current.join("Cargo.toml");
        if cargo.is_file() {
            let text = fs::read_to_string(&cargo)?;
            if text.contains("[workspace]") {
                return Ok(current);
            }
        }
        if !current.pop() {
            bail!("no Cargo workspace found from {}", start.display());
        }
    }
}

pub fn create_settings(root: &Path, options: &SettingsOptions) -> Result<()> {
    validate_project_name(&options.project_name)?;
    validate_relative_path(&options.server_dir, "server_dir")?;
    ensure_framework_workspace(root)?;

    let server_package = format!("{}_server", to_snake_case(&options.project_name));
    validate_package_name(&server_package)?;
    let manifest_path = root.join("lex.toml");
    let manifest = if manifest_path.exists() && !options.force {
        ProjectManifest::load(root)?
    } else {
        let manifest = ProjectManifest {
            project_name: options.project_name.clone(),
            apps_dir: PathBuf::from("apps"),
            server_dir: options.server_dir.clone(),
            server_package: server_package.clone(),
        };
        write_atomic(&manifest_path, &render_project_manifest(&manifest))?;
        manifest
    };

    let config_path = root.join("Config.toml");
    if options.force || !config_path.exists() {
        write_atomic(&config_path, default_config_toml())?;
    }
    let env_path = root.join(".env.example");
    if options.force || !env_path.exists() {
        write_atomic(&env_path, default_env_example())?;
    }
    fs::create_dir_all(root.join(&manifest.apps_dir))?;
    fs::create_dir_all(root.join("migrations"))?;
    generate_server_crate(root, &manifest, options.force)?;
    ensure_workspace_member(root, &path_to_manifest_string(&manifest.server_dir)?)?;
    ensure_framework_workspace_dependencies(root)?;
    refresh_generated_project(root, &manifest)?;
    Ok(())
}

pub fn create_crate(root: &Path, options: &ScaffoldOptions) -> Result<PathBuf> {
    let manifest = ProjectManifest::load(root)?;
    let crate_name = to_snake_case(&options.name);
    validate_rust_ident(&crate_name)?;
    if is_reserved_app_name(&crate_name) {
        bail!("{crate_name:?} is reserved by the framework");
    }
    let package = format!("lex_app_{crate_name}");
    let app_dir = safe_join(root, &manifest.apps_dir.join(&crate_name))?;
    if app_dir.exists() && !options.force {
        bail!(
            "{} already exists; use --force only if you intentionally want to replace framework-generated files",
            app_dir.display()
        );
    }
    fs::create_dir_all(app_dir.join("src"))?;
    fs::create_dir_all(root.join("migrations").join(&crate_name))?;

    let mount = options
        .mount
        .clone()
        .unwrap_or_else(|| format!("/{crate_name}"));
    validate_mount(&mount)?;
    let app_manifest = AppManifest {
        name: crate_name.clone(),
        package: package.clone(),
        crate_ident: package.clone(),
        mount,
    };

    write_guarded(
        &app_dir.join("Cargo.toml"),
        &render_app_cargo(&app_manifest),
        options.force,
    )?;
    write_guarded(
        &app_dir.join("lex_app.toml"),
        &render_app_manifest(&app_manifest),
        options.force,
    )?;
    // Developer-owned source is never overwritten by a repeated scaffold command.
    write_guarded(&app_dir.join("src/lib.rs"), &render_app_lib(), false)?;
    write_guarded(
        &app_dir.join("src/models.rs"),
        &render_app_models(&crate_name),
        false,
    )?;
    write_guarded(
        &app_dir.join("src/views.rs"),
        &render_app_views(&crate_name),
        false,
    )?;
    write_guarded(
        &app_dir.join("src/urls.rs"),
        &render_app_urls_actual(&crate_name),
        false,
    )?;
    write_guarded(
        &app_dir.join("README.md"),
        &render_app_readme(&app_manifest),
        false,
    )?;

    ensure_workspace_member(root, "apps/*")?;
    ensure_framework_workspace_dependencies(root)?;
    refresh_generated_project(root, &manifest)?;
    Ok(app_dir)
}

pub fn refresh(root: &Path) -> Result<Vec<AppManifest>> {
    let manifest = ProjectManifest::load(root)?;
    refresh_generated_project(root, &manifest)
}

pub fn project_check(root: &Path) -> Result<CheckReport> {
    let mut issues = Vec::new();
    let manifest = match ProjectManifest::load(root) {
        Ok(value) => value,
        Err(error) => {
            issues.push(error.to_string());
            return Ok(CheckReport {
                root: root.to_path_buf(),
                app_count: 0,
                issues,
            });
        }
    };
    let server_cargo = root.join(&manifest.server_dir).join("Cargo.toml");
    if !server_cargo.is_file() {
        issues.push(format!("missing server crate: {}", server_cargo.display()));
    }
    let config = root.join("Config.toml");
    if !config.is_file() {
        issues.push("Config.toml is missing".to_owned());
    }
    if !root
        .join(&manifest.server_dir)
        .join("src/apps.rs")
        .is_file()
    {
        issues.push("generated project/src/apps.rs is missing; run `lex refresh`".to_owned());
    }
    let apps = match discover_apps(root, &manifest) {
        Ok(apps) => apps,
        Err(error) => {
            issues.push(error.to_string());
            Vec::new()
        }
    };
    let mut mounts = BTreeSet::new();
    let mut packages = BTreeSet::new();
    for app in &apps {
        if !mounts.insert(app.mount.clone()) {
            issues.push(format!("duplicate URL mount: {}", app.mount));
        }
        if !packages.insert(app.package.clone()) {
            issues.push(format!("duplicate app package: {}", app.package));
        }
        let app_dir = root.join(&manifest.apps_dir).join(&app.name);
        for required in [
            "Cargo.toml",
            "src/lib.rs",
            "src/models.rs",
            "src/views.rs",
            "src/urls.rs",
        ] {
            if !app_dir.join(required).is_file() {
                issues.push(format!("{} is missing {required}", app.name));
            }
        }
    }
    Ok(CheckReport {
        root: root.to_path_buf(),
        app_count: apps.len(),
        issues,
    })
}

pub fn start_server(root: &Path, release: bool, cargo_args: &[String]) -> Result<ExitStatus> {
    let manifest = ProjectManifest::load(root)?;
    refresh_generated_project(root, &manifest)?;
    let mut command = Command::new("cargo");
    command.current_dir(root).arg("run");
    if release {
        command.arg("--release");
    }
    command
        .arg("-p")
        .arg(&manifest.server_package)
        .arg("--bin")
        .arg(&manifest.server_package);
    if !cargo_args.is_empty() {
        command.arg("--").args(cargo_args);
    }
    command
        .status()
        .with_context(|| "failed to launch Cargo; ensure Rust/Cargo is installed")
}

pub fn run_cargo_command(root: &Path, args: &[&str]) -> Result<ExitStatus> {
    Command::new("cargo")
        .current_dir(root)
        .args(args)
        .status()
        .with_context(|| "failed to launch Cargo; ensure Rust/Cargo is installed")
}

pub fn run_migration_command(root: &Path, args: &[String]) -> Result<ExitStatus> {
    let mut command = Command::new("cargo");
    command.current_dir(root).arg("run").arg("-p");
    if root.join("crate/migration/Cargo.toml").is_file() {
        command
            .arg("lex-migration")
            .arg("--bin")
            .arg("migration-cli");
    } else {
        let manifest = ProjectManifest::load(root)?;
        let runner = root
            .join(&manifest.server_dir)
            .join("src/bin/lex-migrate.rs");
        if !runner.is_file() {
            bail!(
                "generated migration runner is missing; run `lex create settings` to add it without overwriting source"
            );
        }
        command
            .arg(manifest.server_package)
            .arg("--bin")
            .arg("lex-migrate");
    }
    command.arg("--");
    command.args(args);
    command
        .status()
        .with_context(|| "failed to launch migration CLI")
}

fn generate_server_crate(root: &Path, manifest: &ProjectManifest, force: bool) -> Result<()> {
    let server_dir = safe_join(root, &manifest.server_dir)?;
    fs::create_dir_all(server_dir.join("src"))?;
    write_guarded(
        &server_dir.join("Cargo.toml"),
        &render_server_cargo(manifest),
        force,
    )?;
    write_guarded(
        &server_dir.join("src/main.rs"),
        &render_server_main(),
        force,
    )?;
    write_guarded(
        &server_dir.join("src/settings.rs"),
        &render_server_settings(),
        force,
    )?;
    write_guarded(
        &server_dir.join("src/bin/lex-migrate.rs"),
        "// @generated by Lex. Register application models before migration planning.\n#[path = \"../apps.rs\"] mod apps;\n#[tokio::main]\nasync fn main() -> anyhow::Result<()> {\n    apps::register_models();\n    lex_framework::migration::run_migration_cli().await\n}\n",
        false,
    )?;
    if force || !server_dir.join("src/urls.rs").exists() {
        write_atomic(&server_dir.join("src/urls.rs"), &render_root_urls(&[]))?;
    }
    if force || !server_dir.join("src/apps.rs").exists() {
        write_atomic(&server_dir.join("src/apps.rs"), &render_root_apps(&[]))?;
    }
    Ok(())
}

fn refresh_generated_project(root: &Path, manifest: &ProjectManifest) -> Result<Vec<AppManifest>> {
    let apps = discover_apps(root, manifest)?;
    let server_dir = safe_join(root, &manifest.server_dir)?;
    if !server_dir.join("Cargo.toml").is_file() {
        bail!(
            "server crate {} is missing; run `lex create settings --force` to recreate framework scaffolding",
            server_dir.display()
        );
    }
    update_server_dependencies(&server_dir.join("Cargo.toml"), root, manifest, &apps)?;
    write_atomic(&server_dir.join("src/urls.rs"), &render_root_urls(&apps))?;
    write_atomic(&server_dir.join("src/apps.rs"), &render_root_apps(&apps))?;
    Ok(apps)
}

fn discover_apps(root: &Path, manifest: &ProjectManifest) -> Result<Vec<AppManifest>> {
    let apps_dir = safe_join(root, &manifest.apps_dir)?;
    if !apps_dir.exists() {
        return Ok(Vec::new());
    }
    let mut apps = Vec::new();
    for entry in
        fs::read_dir(&apps_dir).with_context(|| format!("failed to read {}", apps_dir.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let manifest_path = entry.path().join("lex_app.toml");
        if !manifest_path.is_file() {
            continue;
        }
        let app = AppManifest::load(&manifest_path)?;
        if entry.file_name() != OsStr::new(&app.name) {
            bail!(
                "app manifest name {:?} does not match directory {}",
                app.name,
                entry.path().display()
            );
        }
        apps.push(app);
    }
    apps.sort_by(|left, right| left.name.cmp(&right.name));
    let mut mounts = BTreeSet::new();
    let mut packages = BTreeSet::new();
    for app in &apps {
        if !mounts.insert(app.mount.clone()) {
            bail!("duplicate URL mount {}", app.mount);
        }
        if !packages.insert(app.package.clone()) {
            bail!("duplicate app package {}", app.package);
        }
    }
    Ok(apps)
}

fn update_server_dependencies(
    cargo_path: &Path,
    root: &Path,
    project: &ProjectManifest,
    apps: &[AppManifest],
) -> Result<()> {
    let mut text = fs::read_to_string(cargo_path)
        .with_context(|| format!("failed to read {}", cargo_path.display()))?;
    let server_dir = safe_join(root, &project.server_dir)?;
    let mut generated = String::new();
    generated.push_str(APPS_START);
    generated.push('\n');
    for app in apps {
        let app_dir = safe_join(root, &project.apps_dir.join(&app.name))?;
        let relative = relative_path(&server_dir, &app_dir)?;
        generated.push_str(&format!(
            "{} = {{ package = {:?}, path = {:?} }}\n",
            app.crate_ident,
            app.package,
            path_to_manifest_string(&relative)?
        ));
    }
    generated.push_str(APPS_END);

    if let (Some(start), Some(end)) = (text.find(APPS_START), text.find(APPS_END)) {
        let end = end + APPS_END.len();
        text.replace_range(start..end, &generated);
    } else {
        text.push('\n');
        text.push_str(&generated);
        text.push('\n');
    }
    write_atomic(cargo_path, &text)
}

fn render_project_manifest(manifest: &ProjectManifest) -> String {
    format!(
        "# Lex project manifest. Managed by `lex`; safe to edit documented values.\nproject_name = {:?}\napps_dir = {:?}\nserver_dir = {:?}\nserver_package = {:?}\n",
        manifest.project_name,
        path_to_manifest_string(&manifest.apps_dir).unwrap_or_else(|_| "apps".to_owned()),
        path_to_manifest_string(&manifest.server_dir).unwrap_or_else(|_| "project".to_owned()),
        manifest.server_package,
    )
}

fn render_app_manifest(app: &AppManifest) -> String {
    format!(
        "# Lex application manifest.\nname = {:?}\npackage = {:?}\ncrate_ident = {:?}\nmount = {:?}\n",
        app.name, app.package, app.crate_ident, app.mount
    )
}

fn render_server_cargo(manifest: &ProjectManifest) -> String {
    format!(
        r#"[package]
name = {package:?}
default-run = {package:?}
version = "1.0.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish = false

[dependencies]
anyhow.workspace = true
async-trait.workspace = true
axum.workspace = true
db.workspace = true
http.workspace = true
lex_rest = {{ workspace = true, features = ["axum"] }}
lex_framework.workspace = true
media.workspace = true
serde_json.workspace = true
tokio.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true

# lex:apps:start
# lex:apps:end

[lints]
workspace = true
"#,
        package = manifest.server_package
    )
}

fn render_server_main() -> String {
    r#"mod apps;
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

    // Authentication and permission backends are framework-owned and selected in Config.toml.
    let (authentication, _auth_service) = settings
        .authentication(&database)
        .await
        .context("failed to initialize framework authentication")?;
    let permissions = settings
        .permission_backend(&database)
        .context("failed to initialize framework permissions")?;
    let runtime = RestRuntimeBuilder::new(permissions)
        .authentication(authentication)
        .middleware(settings.rest_middleware().context("invalid HTTP security settings")?)
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
"#
    .to_owned()
}

fn render_server_settings() -> String {
    r#"use anyhow::{Result, bail};
use lex_framework::FrameworkSettings;
use std::env;
use tracing_subscriber::EnvFilter;

pub fn init_tracing(){ let filter=env::var("RUST_LOG").unwrap_or_else(|_|"info".to_owned()); let _=tracing_subscriber::fmt().with_env_filter(EnvFilter::new(filter)).try_init(); }
pub fn load()->Result<FrameworkSettings>{FrameworkSettings::load()}
pub fn is_production()->bool{lex_framework::is_production()}
pub fn require_secure_transport()->bool{is_production()}
pub fn assert_trusted_tls_boundary()->Result<()> { if !is_production(){return Ok(())} let ok=env::var("LEX_TRUSTED_TLS_TERMINATION").map(|v|v.eq_ignore_ascii_case("true")||v=="1").unwrap_or(false); if !ok{bail!("production startup requires TLS; set LEX_TRUSTED_TLS_TERMINATION=true only behind a trusted TLS ingress/proxy")} Ok(()) }
"#.to_owned()
}

fn render_root_apps(apps: &[AppManifest]) -> String {
    let mut out = String::from(
        "// @generated by `lex refresh`. Do not edit this file directly.\n\npub fn register_models() {\n",
    );
    for app in apps {
        out.push_str(&format!("    {}::register_models();\n", app.crate_ident));
    }
    out.push_str("}\n");
    out
}

fn render_root_urls(apps: &[AppManifest]) -> String {
    let mut out = String::new();
    out.push_str(
        "// @generated by `lex refresh`. Do not edit this file directly.\n\
use lex_rest::rest_framework::{exceptions::{ApiError, ApiResult}, routing::{RouteSpec, RouteTable}};\n\n\
pub fn build_routes() -> ApiResult<RouteTable> {\n    let mut table = RouteTable::new();\n    table.register(RouteSpec::http(\"/health\", \"framework-health\", std::sync::Arc::new(HealthView)))?;\n",
    );
    for app in apps {
        out.push_str(&format!(
            "    register_mounted(&mut table, {:?}, {}::urls::routes()?)?;\n",
            app.mount, app.crate_ident
        ));
    }
    out.push_str("    Ok(table)\n}\n\n");
    out.push_str(
        r#"fn register_mounted(table: &mut RouteTable, mount: &str, routes: Vec<RouteSpec>) -> ApiResult<()> {
    for mut route in routes {
        route.path = mount_path(mount, &route.path)?;
        table.register(route)?;
    }
    Ok(())
}

fn mount_path(mount: &str, route: &str) -> ApiResult<String> {
    if !mount.starts_with('/') || (mount.len() > 1 && mount.ends_with('/')) {
        return Err(ApiError::configuration("invalid generated app mount"));
    }
    if !route.starts_with('/') {
        return Err(ApiError::configuration("app routes must start with '/'") );
    }
    if mount == "/" {
        return Ok(route.to_owned());
    }
    if route == "/" {
        return Ok(mount.to_owned());
    }
    Ok(format!("{mount}{route}"))
}

struct HealthView;

#[async_trait::async_trait]
impl lex_rest::rest_framework::views::APIView for HealthView {
    fn name(&self) -> &'static str { "framework-health" }
    fn allowed_methods(&self) -> Vec<http::Method> { vec![http::Method::GET] }
    fn access(&self) -> lex_rest::rest_framework::views::ViewAccess {
        lex_rest::rest_framework::views::ViewAccess::Public
    }
    async fn get(
        &self,
        _request: &lex_rest::rest_framework::request::ApiRequest,
        _context: &lex_rest::rest_framework::request::RequestContext,
        _services: &lex_rest::rest_framework::views::ViewServices,
    ) -> ApiResult<lex_rest::rest_framework::response::ApiResponse> {
        lex_rest::rest_framework::response::ApiResponse::json(
            http::StatusCode::OK,
            &serde_json::json!({"status": "ok", "framework": "lex"}),
        )
    }
}
"#,
    );
    out
}

fn render_app_cargo(app: &AppManifest) -> String {
    format!(
        r#"[package]
name = {package:?}
version = "1.0.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish = false

[dependencies]
async-trait.workspace = true
db_controller.workspace = true
http.workspace = true
lex_framework.workspace = true
lex_rest.workspace = true
orm.workspace = true
serde.workspace = true
serde_json.workspace = true
uuid.workspace = true

[lints]
workspace = true
"#,
        package = app.package
    )
}

fn render_app_lib() -> String {
    "pub mod models;\npub mod urls;\npub mod views;\n\npub fn register_models() {\n    models::register_models();\n}\n".to_owned()
}

fn render_app_models(name: &str) -> String {
    let type_name = to_pascal_case(name);
    format!(
        r#"use db_controller::{{TextField, UUIDField}};
use orm::Model;
use serde::{{Deserialize, Serialize}};

/// Starter model generated by `lex create crate`.
/// Replace or extend this model; the field metadata feeds the ORM/migration layers.
#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[table(name = "{name}_items")]
#[timestamps]
pub struct {type_name}Item {{
    #[field(primary_key)]
    pub id: UUIDField,

    #[field(not_empty, max_length = 200)]
    pub name: TextField,
}}

pub fn register_models() {{
    use db_controller::Entity;
    {type_name}Item::register();
}}
"#
    )
}

fn render_app_views(name: &str) -> String {
    let type_name = to_pascal_case(name);
    format!(
        r#"use async_trait::async_trait;
use http::{{Method, StatusCode}};
use lex_framework::{{
    permissions::{{Permissions, ViewPermissions}},
    views::{{ApiRequest, ApiResponse, ApiResult, APIView, RequestContext, ViewServices}},
}};
use serde_json::json;

pub struct {type_name}IndexView;

#[async_trait]
impl APIView for {type_name}IndexView {{
    fn name(&self) -> &'static str {{ "{name}-index" }}

    fn allowed_methods(&self) -> Vec<Method> {{
        vec![Method::GET]
    }}

    // Protected views are the framework default. This starter route is deliberately public.
    fn permissions(&self) -> ViewPermissions {{
        ViewPermissions::new(Permissions::public())
    }}

    async fn get(
        &self,
        _request: &ApiRequest,
        _context: &RequestContext,
        _services: &ViewServices,
    ) -> ApiResult<ApiResponse> {{
        ApiResponse::json(
            StatusCode::OK,
            &json!({{"app": "{name}", "status": "ready"}}),
        )
    }}
}}
"#
    )
}

fn render_app_urls_for(name: &str) -> String {
    let type_name = to_pascal_case(name);
    format!(
        r#"use crate::views::{type_name}IndexView;
use lex_rest::rest_framework::{{exceptions::ApiResult, routing::RouteSpec}};
use std::sync::Arc;

/// App-local URLs. Paths are relative to this app's mount from `lex_app.toml`.
/// `lex refresh` mounts them into the project route table automatically.
pub fn routes() -> ApiResult<Vec<RouteSpec>> {{
    Ok(vec![RouteSpec::http(
        "/",
        "{name}-index",
        Arc::new({type_name}IndexView),
    )])
}}
"#
    )
}

fn render_app_readme(app: &AppManifest) -> String {
    format!(
        "# {}\n\nGenerated Lex application crate.\n\n- Mounted at `{}`.\n- Put ORM models in `src/models.rs`.\n- Put API views in `src/views.rs`.\n- Register app-local routes in `src/urls.rs`.\n- Run `lex refresh` after changing `lex_app.toml`.\n- Run `lex start server` from the workspace root.\n",
        app.name, app.mount
    )
}

fn default_config_toml() -> &'static str {
    r###"# Lex project settings. Safe configuration belongs here; secrets belong in environment/system credentials.
debug = true

[database]
backend = "auto"
max_connections = 10
min_connections = 1
connect_timeout_secs = 10
connect_retries = 5
require_ssl = false
run_migrations = false
migrations_dir = "./migrations"
read_replica_max_connections = 5
read_replica_min_connections = 0
require_all_read_replicas = false
sqlite_create_if_missing = true
sqlite_foreign_keys = true
sqlite_wal = true
sqlite_busy_timeout_ms = 5000

[server]
host = "127.0.0.1"
port = 8080

[media]
enabled = true
# memory = test-only; filesystem = local storage; s3 runtime is incomplete and must not be deployed.
storage = "filesystem"
local_root = "./media"
temp_root = "./.lex/uploads"
max_upload_bytes = 26214400
allowed_content_types = ["image/jpeg", "image/png", "image/webp", "application/pdf"]
# Set to "clamav" after running clamd and configuring [media.clamav].
scanner = "disabled"

[media.s3]
# Backblaze example endpoint: https://s3.us-west-004.backblazeb2.com
# endpoint = "https://s3.us-west-004.backblazeb2.com"
# region = "us-west-004"
# bucket = "my-private-media"
# public_base_url = "https://cdn.example.com"
force_path_style = false
# Credentials are read from these environment variable NAMES, never stored here.
access_key_env = "LEX_MEDIA_S3_ACCESS_KEY_ID"
secret_key_env = "LEX_MEDIA_S3_SECRET_ACCESS_KEY"

[media.clamav]
host = "127.0.0.1"
port = 3310
timeout_secs = 15
# Recommended in production: reject uploads if the scanner is unavailable.
fail_closed = true


[security.authentication]
# Ordered authentication chain. Supported values: "session", "cookie", "bearer".
# Examples:
#   Browser/server-rendered app: ["session"]
#   JSON/mobile API:             ["bearer"]
#   Hybrid browser + API:        ["session", "bearer"]
#   Stateless signed cookie:     ["cookie"]
# Authentication is framework-owned. Enable the mechanisms you want; no provider code is required.
# Protected views are the default. Use `ViewPermissions` + `Permissions::public()` only for intentionally anonymous views.
backends = []

[security.authentication.session]
# Server-side session ID stored in a cookie. Lex creates and verifies the session store automatically.
cookie_name = "lex_session"
# Development may use HTTP; production validation requires true.
require_secure_transport = false
# Keep enabled for browser session authentication. Unsafe methods require X-CSRF-Token.
csrf_protection = true
csrf_header = "x-csrf-token"
ttl_secs = 86400

[security.authentication.cookie]
# Stateless HMAC-SHA256 signed authentication cookie managed by Lex.
# The signing key stays outside Config.toml and is loaded from secret_env.
cookie_name = "lex_auth"
require_secure_transport = false
csrf_protection = true
csrf_header = "x-csrf-token"
ttl_secs = 86400
secret_env = "LEX_AUTH_COOKIE_SECRET"

[security.authentication.bearer]
# Lex-managed opaque bearer tokens. Only token digests are stored.
scheme = "Bearer"
ttl_secs = 2592000

[security.permissions]
# `scopes` requires no extra wiring. Permissions::named("article.edit") checks the
# authenticated session/token for the matching scope. Use `database` on PostgreSQL or SQLite
# after installing migration::rbac's normalized tenant-role permission schema.
backend = "scopes"
global_admin_scope = "admin:global"
tenant_member_scope = "tenant:member"
tenant_admin_scope = "tenant:admin"

[security.cors]
enabled = true
allowed_origins = ["http://localhost:3000"]
allowed_methods = ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"]
allowed_headers = ["Authorization", "Content-Type", "Idempotency-Key", "X-Request-ID", "X-CSRF-Token"]
expose_headers = ["X-Request-ID"]
allow_credentials = false
max_age_secs = 600

[security.headers]
enabled = true
# Django-CSP style policy, emitted as Content-Security-Policy.
csp = "default-src 'none'; frame-ancestors 'none'; base-uri 'none'"
permissions_policy = "camera=(), microphone=(), geolocation=()"
# Set to an empty/omitted value only when HSTS is handled at a trusted edge.
hsts = "max-age=31536000; includeSubDomains"

[security.request_limits]
max_uri_bytes = 8192
max_header_count = 100
max_header_bytes = 65536
max_single_header_bytes = 16384

[security]
# Keep 0 unless your deployment explicitly trusts proxy-derived client metadata.
trusted_proxy_hops = 0
"###
}

fn default_env_example() -> &'static str {
    r###"APP_PROFILE=development
DATABASE_URL=sqlite://db.sqlite3
RUST_LOG=info

# S3 / Backblaze B2 S3-compatible storage. Uncomment only when [media].storage = "s3".
# LEX_MEDIA_S3_ACCESS_KEY_ID=replace-me
# LEX_MEDIA_S3_SECRET_ACCESS_KEY=replace-me

# Required only when cookie authentication is enabled. Use at least 32 random bytes.
# LEX_AUTH_COOKIE_SECRET=replace-with-a-long-random-secret

# Production examples:
# APP_PROFILE=production
# DATABASE_URL=postgresql://user:password@db-host/app
# LEX_TRUSTED_TLS_TERMINATION=true
"###
}

fn render_app_urls_actual(name: &str) -> String {
    render_app_urls_for(name)
}

fn ensure_framework_workspace(root: &Path) -> Result<()> {
    let cargo = root.join("Cargo.toml");
    let text = fs::read_to_string(&cargo)
        .with_context(|| format!("failed to read {}", cargo.display()))?;
    if !text.contains("[workspace]") {
        bail!("{} is not a Cargo workspace", cargo.display());
    }
    let manifest: toml::Value = text.parse()?;
    for required in [
        "crate/db",
        "crate/db_controller",
        "crate/orm",
        "crate/lex_rest",
    ] {
        let name = required.strip_prefix("crate/").unwrap_or(required);
        if !root.join(required).is_dir()
            && manifest
                .get("workspace")
                .and_then(|w| w.get("dependencies"))
                .and_then(|d| d.get(name))
                .is_none()
        {
            bail!(
                "framework component {required} is missing from {}",
                root.display()
            );
        }
    }
    Ok(())
}

fn ensure_framework_workspace_dependencies(root: &Path) -> Result<()> {
    let cargo = root.join("Cargo.toml");
    let mut text = fs::read_to_string(&cargo)?;
    let deps = [
        ("db", "{ package = \"lex-db\", path = \"crate/db\" }"),
        (
            "db_controller",
            "{ package = \"lex-db-controller\", path = \"crate/db_controller\" }",
        ),
        (
            "migration",
            "{ package = \"lex-migration\", path = \"crate/migration\" }",
        ),
        (
            "models",
            "{ package = \"lex-models\", path = \"crate/models\" }",
        ),
        (
            "lex_rest",
            "{ package = \"lex-rest\", path = \"crate/lex_rest\" }",
        ),
        ("orm", "{ package = \"lex-orm\", path = \"crate/orm\" }"),
        (
            "secure_input",
            "{ package = \"lex-secure-input\", path = \"crate/secure_input\" }",
        ),
        (
            "media",
            "{ package = \"lex-media\", path = \"crate/media\" }",
        ),
        (
            "lex_framework",
            "{ package = \"lex-framework\", path = \"crate/lex_framework\" }",
        ),
    ];
    for (name, value) in deps {
        text = ensure_key_in_section(&text, "workspace.dependencies", name, value)?;
    }
    write_atomic(&cargo, &text)
}

fn ensure_workspace_member(root: &Path, member: &str) -> Result<()> {
    if member.contains('\n') || member.contains('\r') || member.contains('"') {
        bail!("invalid workspace member {member:?}");
    }
    let cargo = root.join("Cargo.toml");
    let mut text = fs::read_to_string(&cargo)?;
    if workspace_member_present(&text, member) {
        return Ok(());
    }
    let mut document: toml_edit::DocumentMut = text.parse()?;
    let members = document
        .get_mut("workspace")
        .and_then(|workspace| workspace.get_mut("members"))
        .and_then(toml_edit::Item::as_array_mut)
        .ok_or_else(|| anyhow!("workspace Cargo.toml has no valid members array"))?;
    members.push(member);
    text = document.to_string();
    write_atomic(&cargo, &text)
}

fn workspace_member_present(text: &str, member: &str) -> bool {
    text.parse::<toml::Value>()
        .ok()
        .and_then(|value| value.get("workspace")?.get("members")?.as_array().cloned())
        .is_some_and(|members| members.iter().any(|value| value.as_str() == Some(member)))
}

fn ensure_key_in_section(text: &str, section: &str, key: &str, value: &str) -> Result<String> {
    let parsed: toml::Value = text.parse()?;
    let existing = section
        .split('.')
        .try_fold(&parsed, |value, component| value.get(component));
    if existing.and_then(|value| value.get(key)).is_some() {
        return Ok(text.to_owned());
    }

    let header = format!("[{section}]");
    let Some(start) = text.find(&header) else {
        bail!("Cargo.toml is missing {header}");
    };
    let body_start = start + header.len();
    let next = text[body_start..]
        .find("\n[")
        .map(|offset| body_start + offset + 1)
        .unwrap_or(text.len());
    let body = &text[body_start..next];
    if body.lines().any(|line| {
        line.trim_start()
            .strip_prefix(key)
            .is_some_and(|rest| rest.trim_start().starts_with('='))
    }) {
        return Ok(text.to_owned());
    }
    let mut out = text.to_owned();
    let insertion = format!("{key} = {value}\n");
    out.insert_str(next, &insertion);
    Ok(out)
}

fn write_guarded(path: &Path, content: &str, force: bool) -> Result<()> {
    if path.exists() && !force {
        return Ok(());
    }
    write_atomic(path, content)
}

fn write_atomic(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file_name = path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| anyhow!("invalid output path {}", path.display()))?;
    let tmp = path.with_file_name(format!(".{file_name}.lex-tmp-{}", std::process::id()));
    {
        let mut file = fs::File::create(&tmp)
            .with_context(|| format!("failed to create {}", tmp.display()))?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path).with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

fn safe_join(root: &Path, relative: &Path) -> Result<PathBuf> {
    validate_relative_path(relative, "path")?;
    let candidate = root.join(relative);
    let parent = candidate.parent().unwrap_or(root);
    fs::create_dir_all(parent)?;
    let canonical_root = fs::canonicalize(root)?;
    let canonical_parent = fs::canonicalize(parent)?;
    if !canonical_parent.starts_with(&canonical_root) {
        bail!("path escapes workspace root: {}", relative.display());
    }
    Ok(candidate)
}

fn validate_relative_path(path: &Path, label: &str) -> Result<()> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        bail!("{label} must be a non-empty relative path");
    }
    for component in path.components() {
        use std::path::Component;
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("{label} may not escape the workspace")
            }
        }
    }
    Ok(())
}

fn relative_path(from: &Path, to: &Path) -> Result<PathBuf> {
    let from = fs::canonicalize(from)?;
    let to = fs::canonicalize(to)?;
    let from_components = from.components().collect::<Vec<_>>();
    let to_components = to.components().collect::<Vec<_>>();
    let mut common = 0;
    while common < from_components.len()
        && common < to_components.len()
        && from_components[common] == to_components[common]
    {
        common += 1;
    }
    let mut result = PathBuf::new();
    for _ in common..from_components.len() {
        result.push("..");
    }
    for component in &to_components[common..] {
        result.push(component.as_os_str());
    }
    Ok(result)
}

fn path_to_manifest_string(path: &Path) -> Result<String> {
    path.to_str()
        .map(|value| value.replace('\\', "/"))
        .ok_or_else(|| anyhow!("path is not valid UTF-8: {}", path.display()))
}

fn validate_mount(value: &str) -> Result<()> {
    if !value.starts_with('/')
        || value.contains('?')
        || value.contains('#')
        || value.contains("//")
        || (value.len() > 1 && value.ends_with('/'))
        || value.len() > 512
    {
        bail!("invalid URL mount {value:?}; use forms like /blog or /api/v1/blog");
    }
    if value
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        bail!("URL mount may not contain dot segments");
    }
    Ok(())
}

fn validate_project_name(value: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > 80 {
        bail!("project name must contain 1..=80 characters");
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == ' ')
    {
        bail!("project name contains unsupported characters");
    }
    Ok(())
}

fn validate_package_name(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!("invalid Cargo package name {value:?}");
    }
    Ok(())
}

fn validate_rust_ident(value: &str) -> Result<()> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        bail!("name cannot be empty");
    };
    if !(first.is_ascii_alphabetic() || first == '_')
        || !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        bail!("{value:?} cannot be used as a Rust identifier");
    }
    if matches!(
        value,
        "crate"
            | "self"
            | "super"
            | "mod"
            | "fn"
            | "struct"
            | "enum"
            | "trait"
            | "type"
            | "use"
            | "pub"
            | "impl"
            | "async"
            | "await"
            | "move"
            | "where"
            | "match"
            | "loop"
            | "for"
            | "while"
            | "return"
    ) {
        bail!("{value:?} is a reserved Rust keyword");
    }
    Ok(())
}

fn is_reserved_app_name(value: &str) -> bool {
    matches!(
        value,
        "project" | "framework" | "lex" | "migration" | "db" | "orm"
    )
}

pub fn to_snake_case(value: &str) -> String {
    let mut out = String::new();
    let mut separator = false;
    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            if ch.is_ascii_uppercase() && !out.is_empty() && !separator {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
            separator = false;
        } else if !out.is_empty() {
            separator = true;
        }
        if separator && !out.ends_with('_') {
            out.push('_');
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    out
}

fn to_pascal_case(value: &str) -> String {
    value
        .split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_workspace(root: &Path) {
        fs::create_dir_all(root.join("crate/db")).expect("db");
        fs::create_dir_all(root.join("crate/db_controller")).expect("controller");
        fs::create_dir_all(root.join("crate/orm")).expect("orm");
        fs::create_dir_all(root.join("crate/lex_rest")).expect("rest");
        fs::write(
            root.join("Cargo.toml"),
            r#"[workspace]
members = ["crate/lex_cli"]

[workspace.dependencies]
anyhow = "1"
"#,
        )
        .expect("workspace");
    }

    #[test]
    fn normalizes_developer_names() {
        assert_eq!(to_snake_case("Billing Service"), "billing_service");
        assert_eq!(to_snake_case("BlogAPI"), "blog_a_p_i");
    }

    #[test]
    fn rejects_unsafe_mounts() {
        assert!(validate_mount("/api/v1/blog").is_ok());
        assert!(validate_mount("api").is_err());
        assert!(validate_mount("/api/../admin").is_err());
        assert!(validate_mount("/api//admin").is_err());
    }

    #[test]
    fn settings_and_app_scaffolding_are_idempotent() {
        let temp = tempfile::tempdir().expect("tempdir");
        minimal_workspace(temp.path());
        let options = SettingsOptions {
            project_name: "Example".to_owned(),
            server_dir: PathBuf::from("project"),
            force: false,
        };
        create_settings(temp.path(), &options).expect("settings");
        create_settings(temp.path(), &options).expect("settings second run");
        let generated_config = fs::read_to_string(temp.path().join("Config.toml")).expect("config");
        assert!(generated_config.contains("[security.authentication]"));
        assert!(generated_config.contains("backends = []"));
        assert!(generated_config.contains("secret_env = \"LEX_AUTH_COOKIE_SECRET\""));
        let app = create_crate(
            temp.path(),
            &ScaffoldOptions {
                name: "billing".to_owned(),
                mount: Some("/api/billing".to_owned()),
                force: false,
            },
        )
        .expect("app");
        assert!(app.join("src/models.rs").is_file());
        assert!(app.join("src/views.rs").is_file());
        assert!(app.join("src/urls.rs").is_file());
        let urls = fs::read_to_string(temp.path().join("project/src/urls.rs")).expect("urls");
        assert!(urls.contains("lex_app_billing::urls::routes"));
        assert!(urls.contains("\"/api/billing\""));
        let cargo = fs::read_to_string(temp.path().join("project/Cargo.toml")).expect("cargo");
        assert!(cargo.contains("lex_app_billing"));
    }

    #[test]
    fn project_check_detects_duplicate_mounts() {
        let temp = tempfile::tempdir().expect("tempdir");
        minimal_workspace(temp.path());
        create_settings(
            temp.path(),
            &SettingsOptions {
                project_name: "Example".to_owned(),
                server_dir: PathBuf::from("project"),
                force: false,
            },
        )
        .expect("settings");
        for name in ["one", "two"] {
            create_crate(
                temp.path(),
                &ScaffoldOptions {
                    name: name.to_owned(),
                    mount: Some(format!("/{name}")),
                    force: false,
                },
            )
            .expect("app");
        }
        fs::write(
            temp.path().join("apps/two/lex_app.toml"),
            "name = \"two\"\npackage = \"lex_app_two\"\ncrate_ident = \"lex_app_two\"\nmount = \"/one\"\n",
        )
        .expect("rewrite");
        let report = project_check(temp.path()).expect("check");
        assert!(!report.is_ok());
    }

    #[test]
    fn generated_app_urls_use_actual_view_type() {
        let source = render_app_urls_actual("billing");
        assert!(source.contains("BillingIndexView"));
        assert!(!source.contains("APP_INDEX_VIEW_PLACEHOLDER"));
    }
}
