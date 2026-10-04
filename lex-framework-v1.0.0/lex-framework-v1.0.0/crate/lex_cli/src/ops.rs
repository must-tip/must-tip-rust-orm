use anyhow::{Context, Result, anyhow, bail};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use regex::Regex;
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
};
use toml::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditKind {
    All,
    Workspace,
    Sql,
    Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyKind {
    Framework,
    Sqlite,
    Postgres,
    All,
}

#[derive(Debug, Clone)]
pub struct PostgresTestOptions {
    pub database: String,
    pub user: String,
    pub host: String,
    pub port: u16,
    pub password: Option<String>,
}

impl Default for PostgresTestOptions {
    fn default() -> Self {
        Self {
            database: "rust_database_test".to_owned(),
            user: "rust_test_user".to_owned(),
            host: "127.0.0.1".to_owned(),
            port: 5432,
            password: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PostgresTestConfig {
    pub database_url: String,
    pub database: String,
    pub user: String,
    pub host: String,
    pub port: u16,
}

pub fn audit(root: &Path, kind: AuditKind) -> Result<()> {
    match kind {
        AuditKind::All => {
            audit_workspace(root)?;
            audit_dynamic_sql(root)?;
            audit_rust_source(root)?;
        }
        AuditKind::Workspace => audit_workspace(root)?,
        AuditKind::Sql => audit_dynamic_sql(root)?,
        AuditKind::Source => audit_rust_source(root)?,
    }
    Ok(())
}

pub fn verify(root: &Path, kind: VerifyKind, include_postgres: bool) -> Result<()> {
    match kind {
        VerifyKind::Framework => verify_framework(root),
        VerifyKind::Sqlite => verify_sqlite(root),
        VerifyKind::Postgres => verify_postgres(root),
        VerifyKind::All => {
            verify_framework(root)?;
            verify_sqlite(root)?;
            if include_postgres {
                verify_postgres(root)?;
            } else {
                println!(
                    "postgres verification: SKIPPED (use `lex verify all --postgres` to include destructive disposable-database tests)"
                );
            }
            Ok(())
        }
    }
}

pub fn verify_framework(root: &Path) -> Result<()> {
    audit(root, AuditKind::All)?;
    require_command("rustc")?;
    require_command("cargo")?;
    print_command_version(root, "rustc", &["--version"])?;
    print_command_version(root, "cargo", &["--version"])?;

    if !root.join("Cargo.lock").is_file() {
        run_checked(root, "cargo", &["generate-lockfile"])?;
    }

    run_checked(root, "cargo", &["fmt", "--all", "--", "--check"])?;
    run_checked(
        root,
        "cargo",
        &["check", "--workspace", "--all-targets", "--locked"],
    )?;
    run_checked(
        root,
        "cargo",
        &[
            "check",
            "-p",
            "lex-rest",
            "--all-targets",
            "--features",
            "axum-rustls",
            "--locked",
        ],
    )?;
    run_checked(
        root,
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run_checked(
        root,
        "cargo",
        &[
            "clippy",
            "-p",
            "lex-rest",
            "--all-targets",
            "--features",
            "axum-rustls",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run_checked(
        root,
        "cargo",
        &["test", "--workspace", "--all-targets", "--locked"],
    )?;
    run_checked(
        root,
        "cargo",
        &[
            "test",
            "-p",
            "lex-rest",
            "--all-targets",
            "--features",
            "axum-rustls",
            "--locked",
        ],
    )?;
    run_checked(root, "cargo", &["test", "--workspace", "--doc", "--locked"])?;
    run_checked(
        root,
        "cargo",
        &["build", "--release", "--workspace", "--locked"],
    )?;
    run_checked(
        root,
        "cargo",
        &[
            "build",
            "--release",
            "-p",
            "lex-rest",
            "--features",
            "axum-rustls",
            "--locked",
        ],
    )?;

    println!("dependency duplicates (review; duplicates are not automatically unsafe):");
    run_checked(root, "cargo", &["tree", "--duplicates", "--locked"])?;
    println!("framework verification: OK");
    Ok(())
}

pub fn verify_sqlite(root: &Path) -> Result<()> {
    require_command("cargo")?;
    run_checked(root, "cargo", &["test", "-p", "lex-db", "--locked"])?;
    run_checked(
        root,
        "cargo",
        &[
            "test",
            "-p",
            "lex-db-controller",
            "--test",
            "sqlite_orm",
            "--locked",
        ],
    )?;
    run_checked(
        root,
        "cargo",
        &[
            "test",
            "-p",
            "lex-migration",
            "sqlite_command_handlers",
            "--locked",
        ],
    )?;
    println!("sqlite verification: OK");
    Ok(())
}

pub fn verify_postgres(root: &Path) -> Result<()> {
    require_command("cargo")?;
    let database_url = postgres_test_database_url(root)?;
    let mut envs = BTreeMap::new();
    envs.insert("TEST_DATABASE_URL".to_owned(), database_url);
    envs.insert("CONFIRM_DISPOSABLE_DATABASE".to_owned(), "yes".to_owned());

    run_checked_env(
        root,
        "cargo",
        &[
            "test",
            "-p",
            "lex-db-controller",
            "--test",
            "postgres_orm",
            "--locked",
            "--",
            "--ignored",
            "--test-threads=1",
        ],
        &envs,
    )?;
    run_checked_env(
        root,
        "cargo",
        &[
            "test",
            "-p",
            "lex-rest",
            "--test",
            "postgres_permissions",
            "--locked",
            "--",
            "--ignored",
            "--test-threads=1",
        ],
        &envs,
    )?;
    run_checked_env(
        root,
        "cargo",
        &[
            "test",
            "-p",
            "lex-rest",
            "--test",
            "postgres_mutation",
            "--locked",
            "--",
            "--ignored",
            "--test-threads=1",
        ],
        &envs,
    )?;
    println!("postgres verification: OK");
    Ok(())
}

pub fn setup_test_postgres(
    root: &Path,
    options: &PostgresTestOptions,
) -> Result<PostgresTestConfig> {
    validate_pg_ident(&options.database, "database")?;
    validate_pg_ident(&options.user, "role")?;
    if !options.database.to_ascii_lowercase().contains("test")
        && !options.database.to_ascii_lowercase().contains("disposable")
    {
        bail!(
            "refusing to manage database {:?}; test database names must contain 'test' or 'disposable'",
            options.database
        );
    }
    if matches!(
        options.database.as_str(),
        "postgres" | "template0" | "template1"
    ) {
        bail!(
            "refusing to use protected PostgreSQL database {:?}",
            options.database
        );
    }
    for command in [
        "psql",
        "createdb",
        "dropdb",
        "pg_isready",
        "openssl",
        "sudo",
    ] {
        require_command(command)?;
    }

    if !command_success(
        root,
        "pg_isready",
        &["-h", &options.host, "-p", &options.port.to_string()],
    )? {
        if command_available("systemctl") {
            let _ = Command::new("sudo")
                .args(["systemctl", "start", "postgresql"])
                .status();
        }
        if !command_success(
            root,
            "pg_isready",
            &["-h", &options.host, "-p", &options.port.to_string()],
        )? && command_available("service")
        {
            let _ = Command::new("sudo")
                .args(["service", "postgresql", "start"])
                .status();
        }
    }
    if !command_success(
        root,
        "pg_isready",
        &["-h", &options.host, "-p", &options.port.to_string()],
    )? {
        bail!(
            "PostgreSQL is not accepting connections on {}:{}",
            options.host,
            options.port
        );
    }

    let password = match &options.password {
        Some(value) => value.clone(),
        None => capture_checked(root, "openssl", &["rand", "-hex", "32"])?
            .trim()
            .to_owned(),
    };
    if password.contains('\'') || password.contains('\n') || password.contains('\r') {
        bail!("test database password contains unsupported characters");
    }

    let role_exists = sudo_psql_capture(
        root,
        &[
            "--tuples-only",
            "--no-align",
            "--command",
            &format!("SELECT 1 FROM pg_roles WHERE rolname = '{}';", options.user),
        ],
    )?;
    let role_sql = if role_exists.trim() == "1" {
        format!(
            "ALTER ROLE \"{}\" WITH LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION CONNECTION LIMIT 20 PASSWORD '{}';",
            options.user, password
        )
    } else {
        format!(
            "CREATE ROLE \"{}\" WITH LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION CONNECTION LIMIT 20 PASSWORD '{}';",
            options.user, password
        )
    };
    sudo_psql(root, &["--set=ON_ERROR_STOP=1", "--command", &role_sql])?;

    let db_exists = sudo_psql_capture(
        root,
        &[
            "--tuples-only",
            "--no-align",
            "--command",
            &format!(
                "SELECT 1 FROM pg_database WHERE datname = '{}';",
                options.database
            ),
        ],
    )?;
    if db_exists.trim() == "1" {
        run_checked(
            root,
            "sudo",
            &[
                "-u",
                "postgres",
                "dropdb",
                "--if-exists",
                "--force",
                &options.database,
            ],
        )?;
    }
    run_checked(
        root,
        "sudo",
        &[
            "-u",
            "postgres",
            "createdb",
            &format!("--owner={}", options.user),
            "--encoding=UTF8",
            "--template=template0",
            &options.database,
        ],
    )?;
    let database_sql = format!(
        "ALTER DATABASE \"{}\" SET timezone TO 'UTC'; REVOKE CONNECT ON DATABASE \"{}\" FROM PUBLIC; GRANT CONNECT, TEMPORARY ON DATABASE \"{}\" TO \"{}\"; GRANT ALL ON SCHEMA public TO \"{}\";",
        options.database, options.database, options.database, options.user, options.user
    );
    sudo_psql(
        root,
        &[
            "--set=ON_ERROR_STOP=1",
            "--dbname",
            &options.database,
            "--command",
            &database_sql,
        ],
    )?;

    let encoded_password = utf8_percent_encode(&password, NON_ALPHANUMERIC).to_string();
    let database_url = format!(
        "postgresql://{}:{}@{}:{}/{}",
        options.user, encoded_password, options.host, options.port, options.database
    );
    let config = PostgresTestConfig {
        database_url: database_url.clone(),
        database: options.database.clone(),
        user: options.user.clone(),
        host: options.host.clone(),
        port: options.port,
    };
    persist_postgres_test_config(root, &config)?;

    let mut command = Command::new("psql");
    command.current_dir(root)
        .env("PGPASSWORD", &password)
        .args(["--no-psqlrc", "--set=ON_ERROR_STOP=1", "--host", &options.host, "--port", &options.port.to_string(), "--username", &options.user, "--dbname", &options.database, "--command", "SELECT current_database() AS database, current_user AS database_user, current_setting('TimeZone') AS timezone;"]);
    let status = command
        .status()
        .context("failed to verify PostgreSQL test login")?;
    if !status.success() {
        bail!("failed to verify PostgreSQL test database login");
    }

    println!("PostgreSQL disposable test database is ready");
    println!("database: {}", config.database);
    println!("user: {}", config.user);
    println!("host: {}", config.host);
    println!("port: {}", config.port);
    println!("credentials: .lex/test-postgres.toml (mode 0600 on Unix)");
    println!("next: lex verify postgres");
    Ok(config)
}

pub fn postgres_test_status(root: &Path) -> Result<PostgresTestConfig> {
    let config = load_postgres_test_config(root)?;
    println!("database: {}", config.database);
    println!("user: {}", config.user);
    println!("host: {}", config.host);
    println!("port: {}", config.port);
    println!("configuration: present");
    Ok(config)
}

fn audit_workspace(root: &Path) -> Result<()> {
    let root_manifest = parse_toml(&root.join("Cargo.toml"))?;
    let workspace = root_manifest
        .get("workspace")
        .and_then(Value::as_table)
        .ok_or_else(|| anyhow!("root Cargo.toml is missing [workspace]"))?;
    let mut failures = Vec::new();
    if workspace.get("resolver").and_then(Value::as_str) != Some("3") {
        failures.push("workspace resolver must be Cargo resolver 3".to_owned());
    }
    let workspace_package = workspace.get("package").and_then(Value::as_table);
    if workspace_package
        .and_then(|value| value.get("edition"))
        .and_then(Value::as_str)
        != Some("2024")
    {
        failures.push("workspace edition must be Rust 2024".to_owned());
    }
    let members = workspace
        .get("members")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("workspace.members is missing"))?;
    let workspace_deps = root_manifest
        .get("workspace")
        .and_then(Value::as_table)
        .and_then(|table| table.get("dependencies"))
        .and_then(Value::as_table)
        .cloned()
        .unwrap_or_default();

    let mut member_paths = BTreeMap::new();
    let mut manifests = BTreeMap::new();
    for raw in members.iter().filter_map(Value::as_str) {
        if raw.contains('*') {
            for expanded in expand_workspace_glob(root, raw)? {
                register_workspace_manifest(
                    &expanded,
                    raw,
                    &mut member_paths,
                    &mut manifests,
                    &mut failures,
                )?;
            }
        } else {
            register_workspace_manifest(
                &root.join(raw),
                raw,
                &mut member_paths,
                &mut manifests,
                &mut failures,
            )?;
        }
    }

    let package_by_dir: BTreeMap<PathBuf, String> = member_paths
        .iter()
        .map(|(name, path)| (canonicalize_lossy(path), name.clone()))
        .collect();
    let mut graph: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (package_name, manifest) in &manifests {
        let manifest_dir = member_paths.get(package_name).expect("registered manifest");
        for table_name in ["dependencies", "dev-dependencies", "build-dependencies"] {
            let Some(table) = manifest.get(table_name).and_then(Value::as_table) else {
                continue;
            };
            for (dep_name, spec) in table {
                if let Some(path_value) = spec
                    .as_table()
                    .and_then(|table| table.get("path"))
                    .and_then(Value::as_str)
                {
                    let target = canonicalize_lossy(&manifest_dir.join(path_value));
                    match package_by_dir.get(&target) {
                        Some(local_name) => { graph.entry(package_name.clone()).or_default().insert(local_name.clone()); }
                        None => failures.push(format!("{package_name}: local dependency {dep_name:?} points outside declared workspace: {path_value}")),
                    }
                } else if spec
                    .as_table()
                    .and_then(|table| table.get("workspace"))
                    .and_then(Value::as_bool)
                    == Some(true)
                {
                    if !workspace_deps.contains_key(dep_name) {
                        failures.push(format!("{package_name}: {dep_name} inherits from workspace but root has no matching dependency"));
                    }
                } else {
                    failures.push(format!("{package_name}: external dependency {dep_name:?} must inherit its version/features from [workspace.dependencies]"));
                }
            }
        }
    }
    detect_cycles(&graph, &mut failures);
    finish_audit(
        failures,
        &format!(
            "workspace audit: OK ({} crates, acyclic local dependency graph)",
            manifests.len()
        ),
    )
}

fn audit_dynamic_sql(root: &Path) -> Result<()> {
    let call = Regex::new(
        r"sqlx::(?:query|raw_sql|query_as(?:::[^\n(]+)?|query_scalar(?:::[^\n(]+)?)\s*\(",
    )?;
    let mut failures = Vec::new();
    for path in rust_files(&root.join("crate"))? {
        let source = fs::read_to_string(&path)?;
        for mat in call.find_iter(&source) {
            let prefix = first_argument_prefix(&source, mat.end());
            if prefix.starts_with('"')
                || prefix.starts_with("r\"")
                || prefix.starts_with("r#")
                || prefix.starts_with("sqlx::AssertSqlSafe(")
            {
                continue;
            }
            let line = source[..mat.start()]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                + 1;
            failures.push(format!("{}:{line}: dynamic SQLx call must use sqlx::AssertSqlSafe(...) at the reviewed compiler/migration boundary", relative_display(root, &path)));
        }
    }
    finish_audit(failures, "dynamic SQL audit: OK")
}

fn audit_rust_source(root: &Path) -> Result<()> {
    let forbidden = [
        (Regex::new(r"\bunsafe\s*\{")?, "explicit unsafe block"),
        (Regex::new(r"\btodo!\s*\(")?, "todo! macro"),
        (
            Regex::new(r"\bunimplemented!\s*\(")?,
            "unimplemented! macro",
        ),
        (
            Regex::new(r"Arc::from_raw\s*\(")?,
            "Arc::from_raw ownership reconstruction",
        ),
    ];
    let files = rust_files(&root.join("crate"))?;
    let mut failures = Vec::new();
    for path in &files {
        let source = fs::read_to_string(path)?;
        for (pattern, description) in &forbidden {
            if let Some(mat) = pattern.find(&source) {
                let line = source[..mat.start()]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count()
                    + 1;
                failures.push(format!(
                    "{}:{line}: forbidden {description}",
                    relative_display(root, path)
                ));
            }
        }
        if let Err(detail) = balanced_delimiters(&source) {
            failures.push(format!(
                "{}: source integrity: {detail}",
                relative_display(root, path)
            ));
        }
    }
    finish_audit(
        failures,
        &format!("Rust source audit: OK ({} source files)", files.len()),
    )
}

fn finish_audit(failures: Vec<String>, success: &str) -> Result<()> {
    if failures.is_empty() {
        println!("{success}");
        return Ok(());
    }
    for failure in failures {
        eprintln!("{failure}");
    }
    bail!("audit failed")
}

fn parse_toml(path: &Path) -> Result<Value> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    text.parse::<Value>()
        .with_context(|| format!("failed to parse {}", path.display()))
}

fn register_workspace_manifest(
    member_dir: &Path,
    raw_member: &str,
    member_paths: &mut BTreeMap<String, PathBuf>,
    manifests: &mut BTreeMap<String, Value>,
    failures: &mut Vec<String>,
) -> Result<()> {
    let manifest_path = member_dir.join("Cargo.toml");
    if !manifest_path.is_file() {
        failures.push(format!("workspace member missing Cargo.toml: {raw_member}"));
        return Ok(());
    }
    let manifest = parse_toml(&manifest_path)?;
    let Some(package) = manifest.get("package").and_then(Value::as_table) else {
        failures.push(format!("workspace member has no [package]: {raw_member}"));
        return Ok(());
    };
    let Some(name) = package.get("name").and_then(Value::as_str) else {
        failures.push(format!(
            "workspace member has no package.name: {raw_member}"
        ));
        return Ok(());
    };
    if member_paths.contains_key(name) {
        failures.push(format!("duplicate workspace package name: {name}"));
        return Ok(());
    }
    if package
        .get("edition")
        .and_then(Value::as_table)
        .and_then(|table| table.get("workspace"))
        .and_then(Value::as_bool)
        != Some(true)
    {
        failures.push(format!(
            "{raw_member}: package.edition must inherit from workspace"
        ));
    }
    if package
        .get("rust-version")
        .and_then(Value::as_table)
        .and_then(|table| table.get("workspace"))
        .and_then(Value::as_bool)
        != Some(true)
    {
        failures.push(format!(
            "{raw_member}: package.rust-version must inherit from workspace"
        ));
    }
    member_paths.insert(name.to_owned(), member_dir.to_path_buf());
    manifests.insert(name.to_owned(), manifest);
    Ok(())
}

fn expand_workspace_glob(root: &Path, raw: &str) -> Result<Vec<PathBuf>> {
    if !raw.ends_with("/*") {
        bail!("only trailing workspace globs are supported by lex audit: {raw}");
    }
    let base = root.join(raw.trim_end_matches("/*"));
    if !base.exists() {
        return Ok(Vec::new());
    }
    let mut dirs = Vec::new();
    for entry in fs::read_dir(base)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() && entry.path().join("Cargo.toml").is_file() {
            dirs.push(entry.path());
        }
    }
    dirs.sort();
    Ok(dirs)
}

fn detect_cycles(graph: &BTreeMap<String, BTreeSet<String>>, failures: &mut Vec<String>) {
    fn visit(
        node: &str,
        graph: &BTreeMap<String, BTreeSet<String>>,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
        stack: &mut Vec<String>,
        failures: &mut Vec<String>,
    ) {
        if visited.contains(node) {
            return;
        }
        if visiting.contains(node) {
            let idx = stack.iter().position(|item| item == node).unwrap_or(0);
            let mut cycle = stack[idx..].to_vec();
            cycle.push(node.to_owned());
            failures.push(format!(
                "workspace path dependency cycle: {}",
                cycle.join(" -> ")
            ));
            return;
        }
        visiting.insert(node.to_owned());
        stack.push(node.to_owned());
        if let Some(edges) = graph.get(node) {
            for target in edges {
                visit(target, graph, visiting, visited, stack, failures);
            }
        }
        stack.pop();
        visiting.remove(node);
        visited.insert(node.to_owned());
    }
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for node in graph.keys() {
        visit(
            node,
            graph,
            &mut visiting,
            &mut visited,
            &mut Vec::new(),
            failures,
        );
    }
}

fn rust_files(root: &Path) -> Result<Vec<PathBuf>> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        if !dir.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                walk(&path, out)?;
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, &mut out)?;
    out.sort();
    Ok(out)
}

fn first_argument_prefix(source: &str, mut index: usize) -> String {
    let bytes = source.as_bytes();
    while index < source.len() {
        if source[index..].starts_with("//") {
            index = source[index..]
                .find('\n')
                .map(|offset| index + offset + 1)
                .unwrap_or(source.len());
            continue;
        }
        if source[index..].starts_with("/*") {
            index = source[index + 2..]
                .find("*/")
                .map(|offset| index + 2 + offset + 2)
                .unwrap_or(source.len());
            continue;
        }
        if bytes[index].is_ascii_whitespace() {
            index += 1;
            continue;
        }
        return source[index..].chars().take(96).collect();
    }
    String::new()
}

fn balanced_delimiters(source: &str) -> std::result::Result<(), String> {
    let bytes = source.as_bytes();
    let mut stack: Vec<(u8, usize)> = Vec::new();
    let mut i = 0;
    let mut block_depth = 0usize;
    while i < bytes.len() {
        if block_depth > 0 {
            if bytes[i..].starts_with(b"/*") {
                block_depth += 1;
                i += 2;
                continue;
            }
            if bytes[i..].starts_with(b"*/") {
                block_depth -= 1;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        if bytes[i..].starts_with(b"//") {
            i = source[i..]
                .find('\n')
                .map(|offset| i + offset + 1)
                .unwrap_or(bytes.len());
            continue;
        }
        if bytes[i..].starts_with(b"/*") {
            block_depth = 1;
            i += 2;
            continue;
        }
        if let Some(end) = raw_string_end(source, i)? {
            i = end;
            continue;
        }
        if bytes[i] == b'"' {
            i = quoted_end(bytes, i, b'"')?;
            continue;
        }
        if bytes[i] == b'\''
            && let Some(end) = char_literal_end(bytes, i)
        {
            i = end;
            continue;
        }
        match bytes[i] {
            b'(' | b'[' | b'{' => stack.push((bytes[i], i)),
            b')' | b']' | b'}' => {
                let expected = match bytes[i] {
                    b')' => b'(',
                    b']' => b'[',
                    b'}' => b'{',
                    _ => unreachable!(),
                };
                if stack.last().map(|item| item.0) != Some(expected) {
                    return Err(format!(
                        "unmatched closing delimiter {:?} at byte {i}",
                        bytes[i] as char
                    ));
                }
                stack.pop();
            }
            _ => {}
        }
        i += 1;
    }
    if block_depth > 0 {
        return Err("unterminated block comment".to_owned());
    }
    if let Some((ch, pos)) = stack.last() {
        return Err(format!(
            "unclosed delimiter {:?} at byte {pos}",
            *ch as char
        ));
    }
    Ok(())
}

fn raw_string_end(source: &str, index: usize) -> std::result::Result<Option<usize>, String> {
    let bytes = source.as_bytes();
    if bytes.get(index) != Some(&b'r') {
        return Ok(None);
    }
    let mut cursor = index + 1;
    let mut hashes = 0;
    while bytes.get(cursor) == Some(&b'#') {
        hashes += 1;
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'"') {
        return Ok(None);
    }
    let marker = format!("\"{}", "#".repeat(hashes));
    let start = cursor + 1;
    let Some(offset) = source[start..].find(&marker) else {
        return Err("unterminated raw string".to_owned());
    };
    Ok(Some(start + offset + marker.len()))
}

fn quoted_end(bytes: &[u8], mut index: usize, quote: u8) -> std::result::Result<usize, String> {
    index += 1;
    let mut escaped = false;
    while index < bytes.len() {
        let ch = bytes[index];
        if escaped {
            escaped = false;
        } else if ch == b'\\' {
            escaped = true;
        } else if ch == quote {
            return Ok(index + 1);
        }
        index += 1;
    }
    Err("unterminated quoted literal".to_owned())
}

fn char_literal_end(bytes: &[u8], index: usize) -> Option<usize> {
    let mut i = index + 1;
    if i >= bytes.len() {
        return None;
    }
    if bytes[i] == b'\\' {
        i += 2;
    } else {
        i += 1;
    }
    if bytes.get(i) == Some(&b'\'') {
        Some(i + 1)
    } else {
        None
    }
}

fn validate_pg_ident(value: &str, label: &str) -> Result<()> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        bail!("{label} name cannot be empty");
    };
    if !(first.is_ascii_alphabetic() || first == '_')
        || !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        bail!("invalid PostgreSQL {label} name {value:?}");
    }
    Ok(())
}

fn persist_postgres_test_config(root: &Path, config: &PostgresTestConfig) -> Result<()> {
    let dir = root.join(".lex");
    fs::create_dir_all(&dir)?;
    let path = dir.join("test-postgres.toml");
    let content = format!(
        "database_url = {:?}\ndatabase = {:?}\nuser = {:?}\nhost = {:?}\nport = {}\n",
        config.database_url, config.database, config.user, config.host, config.port
    );
    fs::write(&path, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn load_postgres_test_config(root: &Path) -> Result<PostgresTestConfig> {
    let path = root.join(".lex/test-postgres.toml");
    let value =
        parse_toml(&path).with_context(|| "run `lex database test-postgres setup` first")?;
    let table = value
        .as_table()
        .ok_or_else(|| anyhow!("invalid {}", path.display()))?;
    Ok(PostgresTestConfig {
        database_url: get_toml_string(table, "database_url")?,
        database: get_toml_string(table, "database")?,
        user: get_toml_string(table, "user")?,
        host: get_toml_string(table, "host")?,
        port: table
            .get("port")
            .and_then(Value::as_integer)
            .and_then(|value| u16::try_from(value).ok())
            .ok_or_else(|| anyhow!("invalid port in {}", path.display()))?,
    })
}

fn postgres_test_database_url(root: &Path) -> Result<String> {
    if let Ok(url) = env::var("TEST_DATABASE_URL") {
        if env::var("CONFIRM_DISPOSABLE_DATABASE").ok().as_deref() != Some("yes") {
            bail!(
                "refusing PostgreSQL destructive tests; set CONFIRM_DISPOSABLE_DATABASE=yes or use `lex database test-postgres setup`"
            );
        }
        return Ok(url);
    }
    Ok(load_postgres_test_config(root)?.database_url)
}

fn get_toml_string(table: &toml::map::Map<String, Value>, key: &str) -> Result<String> {
    table
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("missing {key}"))
}

fn sudo_psql(root: &Path, args: &[&str]) -> Result<()> {
    let mut full = vec!["-u", "postgres", "psql", "--no-psqlrc"];
    full.extend_from_slice(args);
    run_checked(root, "sudo", &full)
}

fn sudo_psql_capture(root: &Path, args: &[&str]) -> Result<String> {
    let mut full = vec!["-u", "postgres", "psql", "--no-psqlrc"];
    full.extend_from_slice(args);
    capture_checked(root, "sudo", &full)
}

fn require_command(name: &str) -> Result<()> {
    if command_available(name) {
        Ok(())
    } else {
        bail!("required command {name:?} is not installed or not on PATH")
    }
}

fn command_available(name: &str) -> bool {
    env::var_os("PATH")
        .is_some_and(|paths| env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
}

fn run_checked(root: &Path, program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .current_dir(root)
        .args(args)
        .status()
        .with_context(|| format!("failed to launch {program}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("command failed ({status}): {program} {}", args.join(" "))
    }
}

fn run_checked_env(
    root: &Path,
    program: &str,
    args: &[&str],
    envs: &BTreeMap<String, String>,
) -> Result<()> {
    let status = Command::new(program)
        .current_dir(root)
        .args(args)
        .envs(envs)
        .status()
        .with_context(|| format!("failed to launch {program}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("command failed ({status}): {program} {}", args.join(" "))
    }
}

fn capture_checked(root: &Path, program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .current_dir(root)
        .args(args)
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("failed to launch {program}"))?;
    if !output.status.success() {
        bail!(
            "command failed ({}): {program} {}",
            output.status,
            args.join(" ")
        );
    }
    String::from_utf8(output.stdout).context("command produced non-UTF-8 output")
}

fn command_success(root: &Path, program: &str, args: &[&str]) -> Result<bool> {
    Ok(Command::new(program)
        .current_dir(root)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("failed to launch {program}"))?
        .success())
}

fn print_command_version(root: &Path, program: &str, args: &[&str]) -> Result<()> {
    let output = capture_checked(root, program, args)?;
    print!("{output}");
    Ok(())
}

fn canonicalize_lossy(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

pub fn cargo_status(root: &Path, args: &[&str]) -> Result<ExitStatus> {
    Command::new("cargo")
        .current_dir(root)
        .args(args)
        .status()
        .context("failed to launch Cargo")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_source_can_be_audited_without_panicking() {
        assert!(balanced_delimiters("/* 世界 */ fn café() { let text = \"你好\"; }").is_ok());
        assert_eq!(
            first_argument_prefix(&"界".repeat(100), 0).chars().count(),
            96
        );
    }

    #[test]
    fn delimiter_check_ignores_strings_comments_and_lifetimes() {
        let source = r####"
            fn demo<'a>(value: &'a str) {
                // } ) ]
                let _ = "{[(ignored)]}";
                let _ = r###"{ raw }"###;
                let _ = '(';
                if !value.is_empty() { println!("{value}"); }
            }
        "####;
        assert!(balanced_delimiters(source).is_ok());
    }

    #[test]
    fn delimiter_check_rejects_unbalanced_source() {
        assert!(balanced_delimiters("fn broken() { let x = (1 + 2; }").is_err());
    }

    #[test]
    fn postgres_ident_validation_is_strict() {
        assert!(validate_pg_ident("framework_test", "database").is_ok());
        assert!(validate_pg_ident("test-db", "database").is_err());
        assert!(validate_pg_ident("1test", "database").is_err());
    }

    #[test]
    fn workspace_glob_expands_only_crates() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("apps/one")).expect("one");
        fs::create_dir_all(temp.path().join("apps/not_a_crate")).expect("not a crate");
        fs::write(
            temp.path().join("apps/one/Cargo.toml"),
            "[package]\nname='one'\nversion='0.1.0'\n",
        )
        .expect("manifest");
        let expanded = expand_workspace_glob(temp.path(), "apps/*").expect("expand");
        assert_eq!(expanded.len(), 1);
        assert!(expanded[0].ends_with("apps/one"));
    }
}
