use clap::{Parser, Subcommand};

use crate::config::AppConfig;

#[derive(Parser, Debug)]
#[command(name = "lsend", about = "Headless Lsend CLI", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Use plain HTTP instead of HTTPS.
    #[arg(long, global = true)]
    pub http: bool,

    /// Network port (default: 53317).
    #[arg(long, global = true, default_value_t = AppConfig::DEFAULT_PORT)]
    pub port: u16,

    /// Device display name.
    #[arg(long, global = true)]
    pub alias: Option<String>,

    /// Print diagnostic logs (also available via RUST_LOG=info).
    #[arg(short, long, global = true)]
    pub verbose: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Discover Lsend devices on the local network.
    Scan {
        /// How long to wait for responses, in milliseconds.
        #[arg(long, default_value_t = AppConfig::DEFAULT_DISCOVERY_TIMEOUT_MS)]
        timeout: u64,
    },

    /// Send files to a device (IP address or alias from scan).
    Send {
        /// Target device IP or alias.
        target: String,

        /// Files or directories to send.
        #[arg(required = true)]
        paths: Vec<String>,

        /// PIN if the receiver requires one.
        #[arg(long)]
        pin: Option<String>,
    },

    /// Start a receive server and accept incoming files automatically.
    Receive {
        /// Directory where received files are saved.
        #[arg(long)]
        dir: Option<String>,
    },
}
