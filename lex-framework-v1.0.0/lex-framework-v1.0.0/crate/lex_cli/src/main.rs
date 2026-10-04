use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use lex_cli::ops::{
    AuditKind, PostgresTestOptions, VerifyKind, audit, postgres_test_status, setup_test_postgres,
    verify,
};
use lex_cli::{
    ScaffoldOptions, SettingsOptions, create_crate, create_settings, find_workspace_root,
    project_check, refresh, run_cargo_command, run_migration_command, start_server,
};
use std::{env, path::PathBuf, process::ExitCode};

#[derive(Parser)]
#[command(
    name = "lex",
    version,
    about = "Lex framework CLI",
    long_about = "Convention-first CLI for scaffolding, project wiring, database lifecycle, audits, tests, production verification, migrations, and server startup. The lex binary is the single developer/operations entry point; framework workflows do not require repository shell or Python scripts."
)]
struct Cli {
    /// Workspace root. By default lex walks upward from the current directory.
    #[arg(long, global = true, value_name = "DIR")]
    root: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create framework resources.
    Create {
        #[command(subcommand)]
        resource: CreateCommand,
    },
    /// Start framework services.
    Start {
        #[command(subcommand)]
        target: StartCommand,
    },
    /// Rebuild generated project wiring (Cargo app dependencies and central URLs).
    Refresh,
    /// Validate framework project structure without changing it.
    Check,
    /// Run structural validation followed by Cargo workspace checks.
    Doctor {
        #[arg(long)]
        no_cargo: bool,
    },
    /// Run framework-native source and workspace policy audits.
    Audit {
        #[arg(value_enum, default_value_t = AuditTarget::All)]
        target: AuditTarget,
    },
    /// Run framework verification gates without calling repository scripts.
    Verify {
        #[arg(value_enum, default_value_t = VerifyTarget::All)]
        target: VerifyTarget,
        /// Include destructive tests against the configured disposable PostgreSQL database.
        #[arg(long)]
        postgres: bool,
    },
    /// Manage framework database tooling and disposable integration-test databases.
    Database {
        #[command(subcommand)]
        command: DatabaseCommand,
    },
    /// Run database migrations through the framework migration CLI.
    Migrate(PassthroughArgs),
    /// Show discovered framework application mounts.
    Routes,
    /// Install or refresh the lex executable from this framework workspace.
    #[command(name = "self")]
    SelfManage {
        #[command(subcommand)]
        command: SelfCommand,
    },
    /// Run Cargo for advanced/custom workflows while keeping workspace-root resolution consistent.
    Cargo(PassthroughArgs),
}

#[derive(Subcommand)]
enum CreateCommand {
    /// Generate a standalone consumer workspace using a checkout, pinned Git source, or registry release.
    Project {
        name: String,
        #[arg(long, conflicts_with = "git")]
        framework_path: Option<PathBuf>,
        #[arg(long)]
        git: Option<String>,
        #[arg(long, requires = "git", conflicts_with = "tag")]
        rev: Option<String>,
        #[arg(long, requires = "git")]
        tag: Option<String>,
    },
    /// Create a framework application crate with models, views and URL registration.
    Crate {
        name: String,
        /// URL prefix, e.g. /billing or /api/v1/billing.
        #[arg(long)]
        url: Option<String>,
        /// Update framework-owned manifest wiring if the directory already exists.
        /// Developer-owned Rust source files are never overwritten.
        #[arg(long)]
        force: bool,
    },
    /// Create lex.toml, Config.toml, project server bootstrap and app directories.
    Settings {
        #[arg(long, default_value = "lex_project")]
        project_name: String,
        #[arg(long, default_value = "project")]
        server_dir: PathBuf,
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum StartCommand {
    /// Start the configured Lex HTTP server.
    Server {
        #[arg(long)]
        release: bool,
        /// Arguments after `--` are forwarded to the generated server binary.
        #[arg(last = true)]
        args: Vec<String>,
    },
}

#[derive(Subcommand)]
enum DatabaseCommand {
    /// Manage the disposable PostgreSQL database used by integration tests.
    TestPostgres {
        #[command(subcommand)]
        command: TestPostgresCommand,
    },
}

#[derive(Subcommand)]
enum TestPostgresCommand {
    /// Create/recreate a clearly disposable PostgreSQL database and securely persist its local test configuration.
    Setup {
        #[arg(long, default_value = "rust_database_test")]
        database: String,
        #[arg(long, default_value = "rust_test_user")]
        user: String,
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 5432)]
        port: u16,
        /// Optional password. If omitted, lex generates a cryptographically random password with OpenSSL.
        #[arg(long, env = "TEST_DB_PASSWORD", hide_env_values = true)]
        password: Option<String>,
    },
    /// Show the locally persisted disposable test database configuration without printing its password/URL.
    Status,
}

#[derive(Subcommand)]
enum SelfCommand {
    /// Install/reinstall the current workspace lex binary with Cargo.
    Install,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum AuditTarget {
    All,
    Workspace,
    Sql,
    Source,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum VerifyTarget {
    All,
    Framework,
    Sqlite,
    Postgres,
}

#[derive(Args)]
struct PassthroughArgs {
    /// Arguments forwarded to the underlying command.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    if let Command::Create {
        resource:
            CreateCommand::Project {
                name,
                framework_path,
                git,
                rev,
                tag,
            },
    } = &cli.command
    {
        let source = if let Some(path) = framework_path {
            lex_cli::FrameworkSource::Path(path.clone())
        } else if let Some(url) = git {
            lex_cli::FrameworkSource::Git {
                url: url.clone(),
                revision: rev.clone(),
                tag: tag.clone(),
            }
        } else {
            lex_cli::FrameworkSource::Registry("=1.0.0".into())
        };
        let destination = cli.root.clone().unwrap_or(env::current_dir()?).join(name);
        lex_cli::create_project(&destination, name, &source)?;
        println!("created {}", destination.display());
        return Ok(ExitCode::SUCCESS);
    }
    let root = match cli.root {
        Some(root) => find_workspace_root(root)?,
        None => find_workspace_root(env::current_dir()?)?,
    };

    match cli.command {
        Command::Create { resource } => match resource {
            CreateCommand::Project { .. } => {
                unreachable!("project handled before workspace discovery")
            }
            CreateCommand::Crate { name, url, force } => {
                let path = create_crate(
                    &root,
                    &ScaffoldOptions {
                        name,
                        mount: url,
                        force,
                    },
                )?;
                println!("created {}", path.display());
                println!("routes refreshed; run `lex start server`");
                Ok(ExitCode::SUCCESS)
            }
            CreateCommand::Settings {
                project_name,
                server_dir,
                force,
            } => {
                create_settings(
                    &root,
                    &SettingsOptions {
                        project_name,
                        server_dir,
                        force,
                    },
                )?;
                println!("Lex project settings created at {}", root.display());
                println!("development database defaults to sqlite://db.sqlite3");
                Ok(ExitCode::SUCCESS)
            }
        },
        Command::Start { target } => match target {
            StartCommand::Server { release, args } => {
                let status = start_server(&root, release, &args)?;
                Ok(exit_code(status.success()))
            }
        },
        Command::Refresh => {
            let apps = refresh(&root)?;
            println!("refreshed {} application crate(s)", apps.len());
            Ok(ExitCode::SUCCESS)
        }
        Command::Check => {
            let report = project_check(&root)?;
            println!("workspace: {}", report.root.display());
            println!("applications: {}", report.app_count);
            if report.is_ok() {
                println!("status: OK");
                Ok(ExitCode::SUCCESS)
            } else {
                for issue in &report.issues {
                    eprintln!("FAIL: {issue}");
                }
                Ok(ExitCode::FAILURE)
            }
        }
        Command::Doctor { no_cargo } => {
            let report = project_check(&root)?;
            if !report.is_ok() {
                for issue in &report.issues {
                    eprintln!("FAIL: {issue}");
                }
                bail!("project structure validation failed");
            }
            println!("structure: OK ({} apps)", report.app_count);
            audit(&root, AuditKind::All)?;
            if no_cargo {
                return Ok(ExitCode::SUCCESS);
            }
            let status = run_cargo_command(&root, &["check", "--workspace", "--all-targets"])?;
            Ok(exit_code(status.success()))
        }
        Command::Audit { target } => {
            audit(
                &root,
                match target {
                    AuditTarget::All => AuditKind::All,
                    AuditTarget::Workspace => AuditKind::Workspace,
                    AuditTarget::Sql => AuditKind::Sql,
                    AuditTarget::Source => AuditKind::Source,
                },
            )?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Verify { target, postgres } => {
            verify(
                &root,
                match target {
                    VerifyTarget::All => VerifyKind::All,
                    VerifyTarget::Framework => VerifyKind::Framework,
                    VerifyTarget::Sqlite => VerifyKind::Sqlite,
                    VerifyTarget::Postgres => VerifyKind::Postgres,
                },
                postgres,
            )?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Database { command } => match command {
            DatabaseCommand::TestPostgres { command } => match command {
                TestPostgresCommand::Setup {
                    database,
                    user,
                    host,
                    port,
                    password,
                } => {
                    setup_test_postgres(
                        &root,
                        &PostgresTestOptions {
                            database,
                            user,
                            host,
                            port,
                            password,
                        },
                    )?;
                    Ok(ExitCode::SUCCESS)
                }
                TestPostgresCommand::Status => {
                    postgres_test_status(&root)?;
                    Ok(ExitCode::SUCCESS)
                }
            },
        },
        Command::Migrate(args) => {
            let forwarded = if args.args.is_empty() {
                vec!["migrate".to_owned()]
            } else {
                args.args
            };
            let status = run_migration_command(&root, &forwarded)?;
            Ok(exit_code(status.success()))
        }
        Command::Routes => {
            let apps = refresh(&root)?;
            println!("/health\tframework-health");
            for app in apps {
                println!("{}\t{}", app.mount, app.name);
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::SelfManage { command } => match command {
            SelfCommand::Install => {
                let cli_path = root.join("crate/lex_cli");
                let cli_path = cli_path
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("CLI path is not valid UTF-8"))?;
                let status = run_cargo_command(&root, &["install", "--path", cli_path, "--force"])?;
                Ok(exit_code(status.success()))
            }
        },
        Command::Cargo(args) => {
            let refs = args.args.iter().map(String::as_str).collect::<Vec<_>>();
            let status = run_cargo_command(&root, &refs)?;
            Ok(exit_code(status.success()))
        }
    }
}

fn exit_code(success: bool) -> ExitCode {
    if success {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
