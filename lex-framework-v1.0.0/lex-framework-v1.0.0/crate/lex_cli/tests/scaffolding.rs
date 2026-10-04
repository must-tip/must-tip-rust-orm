use lex_cli::{
    ScaffoldOptions, SettingsOptions, create_crate, create_settings, project_check, refresh,
};
use std::{fs, path::Path};

fn workspace(root: &Path) {
    for dir in [
        "crate/db",
        "crate/db_controller",
        "crate/orm",
        "crate/lex_rest",
    ] {
        fs::create_dir_all(root.join(dir)).expect("component");
    }
    fs::write(
        root.join("Cargo.toml"),
        r#"[workspace]
members = ["crate/lex_cli"]

[workspace.dependencies]
anyhow = "1"
async-trait = "0.1"
axum = "0.8"
http = "1"
serde = "1"
serde_json = "1"
tokio = "1"
tracing = "0.1"
tracing-subscriber = "0.3"
uuid = "1"
"#,
    )
    .expect("workspace Cargo.toml");
}

#[test]
fn generated_application_is_registered_with_server_without_manual_url_wiring() {
    let temp = tempfile::tempdir().expect("tempdir");
    workspace(temp.path());
    create_settings(
        temp.path(),
        &SettingsOptions {
            project_name: "Payments Platform".to_owned(),
            server_dir: "project".into(),
            force: false,
        },
    )
    .expect("settings");
    create_crate(
        temp.path(),
        &ScaffoldOptions {
            name: "ledger".to_owned(),
            mount: Some("/api/v1/ledger".to_owned()),
            force: false,
        },
    )
    .expect("crate");

    let urls = fs::read_to_string(temp.path().join("project/src/urls.rs")).expect("urls");
    assert!(urls.contains("lex_app_ledger::urls::routes()?"));
    assert!(urls.contains("\"/api/v1/ledger\""));
    let server_cargo = fs::read_to_string(temp.path().join("project/Cargo.toml")).expect("cargo");
    assert!(server_cargo.contains("package = \"lex_app_ledger\""));
    assert!(server_cargo.contains("../apps/ledger"));
}

#[test]
fn refresh_preserves_developer_owned_view_source() {
    let temp = tempfile::tempdir().expect("tempdir");
    workspace(temp.path());
    create_settings(
        temp.path(),
        &SettingsOptions {
            project_name: "Example".to_owned(),
            server_dir: "project".into(),
            force: false,
        },
    )
    .expect("settings");
    let app = create_crate(
        temp.path(),
        &ScaffoldOptions {
            name: "orders".to_owned(),
            mount: None,
            force: false,
        },
    )
    .expect("crate");
    let views = app.join("src/views.rs");
    fs::write(&views, "// developer code\n").expect("customize");
    refresh(temp.path()).expect("refresh");
    assert_eq!(
        fs::read_to_string(views).expect("read"),
        "// developer code\n"
    );
}

#[test]
fn structural_check_accepts_a_generated_project() {
    let temp = tempfile::tempdir().expect("tempdir");
    workspace(temp.path());
    create_settings(
        temp.path(),
        &SettingsOptions {
            project_name: "Example".to_owned(),
            server_dir: "project".into(),
            force: false,
        },
    )
    .expect("settings");
    create_crate(
        temp.path(),
        &ScaffoldOptions {
            name: "accounts".to_owned(),
            mount: None,
            force: false,
        },
    )
    .expect("crate");
    let report = project_check(temp.path()).expect("check");
    assert!(report.is_ok(), "{:?}", report.issues);
    assert_eq!(report.app_count, 1);
}

#[test]
fn cli_creates_a_standalone_project_without_copying_framework_crates() {
    let directory = tempfile::tempdir().unwrap();
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_lex"))
        .current_dir(directory.path())
        .args([
            "create",
            "project",
            "my_api",
            "--git",
            "https://example.invalid/lex.git",
            "--rev",
            "0123456789abcdef",
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let root = directory.path().join("my_api");
    assert!(!root.join("crate/db").exists());
    let manifest: toml::Value = fs::read_to_string(root.join("Cargo.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        manifest["workspace"]["dependencies"]["lex_framework"]["git"].as_str(),
        Some("https://example.invalid/lex.git")
    );
    assert_eq!(
        manifest["workspace"]["dependencies"]["lex_framework"]["rev"].as_str(),
        Some("0123456789abcdef")
    );
    assert!(root.join("project/src/bin/lex-migrate.rs").is_file());
    assert!(
        std::process::Command::new(env!("CARGO_BIN_EXE_lex"))
            .current_dir(&root)
            .args(["create", "crate", "projects"])
            .status()
            .unwrap()
            .success()
    );
    assert!(project_check(&root).unwrap().is_ok());
    assert!(
        !std::process::Command::new(env!("CARGO_BIN_EXE_lex"))
            .current_dir(directory.path())
            .args([
                "create",
                "project",
                "my_api",
                "--git",
                "https://example.invalid/lex.git",
                "--rev",
                "0123456789abcdef"
            ])
            .status()
            .unwrap()
            .success()
    );
}
