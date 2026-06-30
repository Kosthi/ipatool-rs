mod commands;
mod output;
mod tui;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use ipatool_core::credential;
use ipatool_core::guid;
use ipatool_core::model::Platform;

use output::OutputFormat;

#[derive(Parser)]
#[command(
    name = "ipatool",
    version,
    about = "Download iOS IPA files from the App Store"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
    #[arg(long, global = true, default_value = "text")]
    format: OutputFormat,
    #[arg(long, global = true)]
    verbose: bool,
    #[arg(long, global = true)]
    non_interactive: bool,
    #[arg(long, global = true)]
    keychain_passphrase: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
    Search {
        term: String,
        #[arg(short, long, default_value = "5")]
        limit: u32,
        #[arg(long, default_value = "iphone")]
        platform: Platform,
        #[arg(long, default_value = "US")]
        country: String,
    },
    Purchase {
        #[arg(short = 'b', long)]
        bundle_identifier: String,
    },
    Download {
        #[arg(short = 'b', long)]
        bundle_identifier: Option<String>,
        #[arg(short = 'i', long)]
        app_id: Option<i64>,
        #[arg(long)]
        version_id: Option<String>,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        purchase: bool,
        #[arg(long, default_value = "iphone")]
        platform: Platform,
        #[arg(long, default_value = "4")]
        connections: usize,
    },
    Version {
        #[command(subcommand)]
        action: VersionAction,
    },
}

#[derive(Subcommand)]
enum AuthAction {
    Login {
        #[arg(long)]
        email: String,
        #[arg(long)]
        password: Option<String>,
        #[arg(long)]
        auth_code: Option<String>,
    },
    Info,
    Revoke,
}

#[derive(Subcommand)]
enum VersionAction {
    List {
        #[arg(short = 'i', long)]
        app_id: Option<i64>,
        #[arg(short = 'b', long)]
        bundle_identifier: Option<String>,
    },
    Meta {
        #[arg(short = 'i', long)]
        app_id: Option<i64>,
        #[arg(short = 'b', long)]
        bundle_identifier: Option<String>,
        #[arg(long)]
        version_id: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let filter = if cli.verbose {
        EnvFilter::new("warn,ipatool=debug,ipatool_core=debug")
    } else {
        EnvFilter::from_default_env()
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    let Some(command) = cli.command else {
        return tui::run().await;
    };

    let guid_str = guid::generate_guid().context("failed to generate GUID")?;

    let data_dir = data_dir();
    prepare_data_dir(&data_dir).ok();
    let cookie_path = data_dir.join("cookies.json");

    let mut client = ipatool_core::client::AppleClient::new(guid_str, Some(&cookie_path))
        .context("failed to create client")?;

    match command {
        Commands::Auth { action } => match action {
            AuthAction::Login {
                email,
                password,
                auth_code,
            } => {
                commands::auth::login(
                    &mut client,
                    &email,
                    password.as_deref(),
                    auth_code.as_deref(),
                    cli.non_interactive,
                    cli.format,
                )
                .await?
            }
            AuthAction::Info => commands::auth::info(cli.format).await?,
            AuthAction::Revoke => commands::auth::revoke().await?,
        },
        Commands::Search {
            term,
            limit,
            platform,
            country,
        } => {
            commands::search::search(&client, &term, limit, platform, &country, cli.format).await?
        }
        Commands::Purchase { bundle_identifier } => {
            let account = load_account()?;
            client.set_account(account.clone());
            commands::purchase::purchase(&client, &bundle_identifier, &account, cli.format).await?
        }
        Commands::Download {
            bundle_identifier,
            app_id,
            version_id,
            output,
            purchase,
            platform,
            connections,
        } => {
            let account = load_account()?;
            client.set_account(account.clone());
            commands::download::download(
                &client,
                bundle_identifier.as_deref(),
                app_id,
                version_id.as_deref(),
                output,
                purchase,
                platform,
                connections,
                &account,
            )
            .await?
        }
        Commands::Version { action } => {
            let account = load_account()?;
            client.set_account(account.clone());
            match action {
                VersionAction::List {
                    app_id,
                    bundle_identifier,
                } => {
                    commands::version::list(
                        &client,
                        app_id,
                        bundle_identifier.as_deref(),
                        &account,
                        cli.format,
                    )
                    .await?
                }
                VersionAction::Meta {
                    app_id,
                    bundle_identifier,
                    version_id,
                } => {
                    commands::version::meta(
                        &client,
                        app_id,
                        bundle_identifier.as_deref(),
                        &version_id,
                        &account,
                        cli.format,
                    )
                    .await?
                }
            }
        }
    }

    client.save_cookies(&cookie_path).ok();

    Ok(())
}

pub(crate) fn data_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".ipatool")
}

pub(crate) fn prepare_data_dir(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }

    Ok(())
}

fn load_account() -> Result<ipatool_core::model::Account> {
    credential::load_account()
        .context("failed to load credentials")?
        .context("not logged in, run `ipatool auth login` first")
}
