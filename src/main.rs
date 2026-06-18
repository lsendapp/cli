mod cli;
mod config;
mod discovery;
mod identity;
mod legacy;
mod network;
mod receive;
mod scan_server;
mod send;
mod server;
mod util;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Commands};
use crate::config::AppConfig;
use crate::identity::Identity;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let default_level = if cli.verbose {
        "lsend=info"
    } else {
        "lsend=warn"
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
    tracing_subscriber::fmt().with_env_filter(filter).init();
    let https = !cli.http;
    let config = AppConfig::new(cli.alias, cli.port, https, None)?;
    let identity = Identity::load_or_create(&config.config_dir, https)?;

    match cli.command {
        Commands::Scan { timeout } => cmd_scan(&config, &identity, timeout).await?,
        Commands::Send { target, paths, pin } => {
            send::send_files(&config, &identity, &target, &paths, pin.as_deref()).await?
        }
        Commands::Receive { dir } => {
            let mut config = config;
            if let Some(dir) = dir {
                config.receive_dir = dir.into();
            }
            receive::run(config, identity).await?
        }
    }

    Ok(())
}

async fn cmd_scan(config: &AppConfig, identity: &Identity, timeout_ms: u64) -> Result<()> {
    println!("Scanning for Lsend devices ({timeout_ms} ms)...");
    let devices = discovery::scan(config, identity, timeout_ms).await?;

    if devices.is_empty() {
        println!("No devices found.");
        return Ok(());
    }

    for device in devices {
        println!("  {}", device.display_label());
    }

    Ok(())
}
