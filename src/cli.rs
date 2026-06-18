use clap::{Parser, Subcommand, ValueEnum};

use crate::config::AppConfig;

#[derive(Parser, Debug)]
#[command(
    name = "lsend",
    about = "Headless Lsend CLI. Use --json or `lsend agent` for AI agents.",
    version
)]
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

    /// Print diagnostic logs on stderr (RUST_LOG=info also works).
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Emit structured JSON on stdout (implies --quiet).
    #[arg(long, global = true, visible_alias = "output-json")]
    pub json: bool,

    /// Suppress progress and status text on stdout (human mode only).
    #[arg(short, long, global = true)]
    pub quiet: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Progressive agent documentation (offline, no network).
    Agent {
        #[command(subcommand)]
        topic: Option<AgentCommand>,
    },

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

        /// Do not scan when target is an alias; require IP or prior scan.
        #[arg(long)]
        no_scan: bool,
    },

    /// Start a receive server and accept incoming files automatically.
    Receive {
        /// Directory where received files are saved.
        #[arg(long)]
        dir: Option<String>,

        /// Exit after the first completed transfer (recommended for agents).
        #[arg(long)]
        once: bool,
    },
}

#[derive(Subcommand, Debug, Clone, Copy, ValueEnum)]
pub enum AgentCommand {
    /// Device discovery workflow and JSON schema.
    Scan,
    /// File send workflow; prefer IP targets.
    Send,
    /// Receive workflow; use with --once.
    Receive,
    /// Exit codes and error JSON schema.
    Errors,
    /// Smoke-test checklist for agents.
    Eval,
}
