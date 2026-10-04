//! Create consumer workspaces without copying framework implementation crates.
use crate::{SettingsOptions, create_settings, validate_package_name};
use anyhow::{Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Dependency source shared by all Lex crates in a generated application.
#[derive(Debug, Clone)]
pub enum FrameworkSource {
    /// Version dependencies, available after the matching release has been published.
    Registry(String),
    /// Immutable Git revision or release tag.
    Git {
        url: String,
        revision: Option<String>,
        tag: Option<String>,
    },
    /// Local checkout for framework development and acceptance tests.
    Path(PathBuf),
}

/// Generate a new standalone application workspace. Existing directories are rejected to avoid
/// overwriting application files. The source declaration is TOML-serialized, never concatenated
/// from unescaped command-line input.
pub fn create_project(destination: &Path, name: &str, source: &FrameworkSource) -> Result<()> {
    validate_package_name(name)?;
    if destination.exists() {
        bail!(
            "project destination already exists: {}",
            destination.display()
        );
    }
    let mut dependencies = toml::map::Map::new();
    for name in [
        "db",
        "db_controller",
        "migration",
        "models",
        "lex_rest",
        "orm",
        "secure_input",
        "media",
        "lex_framework",
    ] {
        let mut entry = toml::map::Map::new();
        // Keep application imports stable while using unambiguous registry package names.
        let package = match name {
            "lex_rest" => "lex-rest".to_owned(),
            "lex_framework" => "lex-framework".to_owned(),
            other => format!("lex-{}", other.replace('_', "-")),
        };
        entry.insert("package".into(), package.into());
        match source {
            FrameworkSource::Registry(version) => {
                entry.insert("version".into(), version.clone().into());
            }
            FrameworkSource::Path(root) => {
                let root = fs::canonicalize(root)?;
                let crate_path = root.join("crate").join(name);
                if !crate_path.join("Cargo.toml").is_file() {
                    bail!("missing framework crate {name}");
                }
                entry.insert(
                    "path".into(),
                    crate_path.to_string_lossy().into_owned().into(),
                );
            }
            FrameworkSource::Git { url, revision, tag } => {
                if revision.is_some() == tag.is_some() {
                    bail!("Git source requires exactly one of --rev or --tag");
                }
                if url.trim().is_empty() {
                    bail!("Git URL cannot be empty");
                }
                entry.insert("git".into(), url.clone().into());
                if let Some(revision) = revision {
                    entry.insert("rev".into(), revision.clone().into());
                }
                if let Some(tag) = tag {
                    entry.insert("tag".into(), tag.clone().into());
                }
            }
        }
        dependencies.insert(name.into(), entry.into());
    }
    let mut manifest = r#"[workspace]
resolver = "3"
members = []

[workspace.package]
edition = "2024"
rust-version = "1.97.1"
license = "MIT OR Apache-2.0"

[workspace.dependencies]
anyhow = "1"
async-trait = "0.1"
axum = "0.8"
http = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "net"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
uuid = { version = "1", features = ["serde", "v4"] }

[workspace.lints.rust]
unsafe_code = "forbid"
unused_must_use = "deny"
"#
    .parse::<toml::Value>()?;
    let existing = manifest["workspace"]["dependencies"]
        .as_table_mut()
        .expect("constant workspace dependencies");
    existing.extend(dependencies);
    fs::create_dir_all(destination)?;
    fs::write(
        destination.join("Cargo.toml"),
        toml::to_string_pretty(&manifest)?,
    )?;
    fs::write(
        destination.join(".gitignore"),
        "/target/\n.env\n*.sqlite3\n*.sqlite3-*\n",
    )?;
    create_settings(
        destination,
        &SettingsOptions {
            project_name: name.into(),
            server_dir: "project".into(),
            force: false,
        },
    )
}
