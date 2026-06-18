use std::path::PathBuf;

use anyhow::{Context, Result};

pub const PROTOCOL_VERSION: &str = "2.1";
pub const DEFAULT_PORT: u16 = 53317;
pub const DEFAULT_MULTICAST_GROUP: &str = "224.0.0.167";
pub const DEFAULT_DISCOVERY_TIMEOUT_MS: u64 = 1500;

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub alias: String,
    pub port: u16,
    pub https: bool,
    pub multicast_group: String,
    pub config_dir: PathBuf,
    pub receive_dir: PathBuf,
}

impl AppConfig {
    pub const DEFAULT_PORT: u16 = DEFAULT_PORT;
    pub const DEFAULT_DISCOVERY_TIMEOUT_MS: u64 = DEFAULT_DISCOVERY_TIMEOUT_MS;

    pub fn new(alias: Option<String>, port: u16, https: bool, receive_dir: Option<PathBuf>) -> Result<Self> {
        let config_dir = dirs::config_dir()
            .context("Could not resolve config directory")?
            .join("lsend");

        let receive_dir = receive_dir.unwrap_or_else(default_download_dir);

        Ok(Self {
            alias: alias.unwrap_or_else(crate::util::generate_random_alias),
            port,
            https,
            multicast_group: DEFAULT_MULTICAST_GROUP.to_string(),
            config_dir,
            receive_dir,
        })
    }
}

fn default_download_dir() -> PathBuf {
    dirs::download_dir().unwrap_or_else(|| PathBuf::from("."))
}
