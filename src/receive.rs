use std::net::SocketAddr;

use anyhow::Result;
use tokio::signal;

use crate::config::AppConfig;
use crate::discovery;
use crate::identity::Identity;
use crate::server::{ServerState, run_http, run_https};

pub async fn run(config: AppConfig, identity: Identity) -> Result<()> {
    let state = ServerState::new(config.clone(), identity.clone());
    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));

    let _responder = discovery::run_responder(config.clone(), identity).await?;

    println!(
        "{} is ready to receive on port {} ({})",
        config.alias,
        config.port,
        if config.https { "HTTPS" } else { "HTTP" }
    );
    println!("Saving files to {}", config.receive_dir.display());
    println!("Press Ctrl+C to stop.");

    let server_task = tokio::spawn(async move {
        if config.https {
            run_https(state, addr).await
        } else {
            run_http(state, addr).await
        }
    });

    signal::ctrl_c().await?;
    println!("Shutting down...");
    server_task.abort();

    Ok(())
}
