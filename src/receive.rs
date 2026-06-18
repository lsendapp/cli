use std::net::SocketAddr;

use anyhow::Result;
use tokio::signal;
use tokio::sync::mpsc;

use crate::config::AppConfig;
use crate::discovery;
use crate::identity::Identity;
use crate::output::{OutputOptions, ReceiveEventJson, print_json};
use crate::server::{ServerState, run_http, run_https};

pub async fn run(
    config: AppConfig,
    identity: Identity,
    receive_pin: Option<String>,
    output: OutputOptions,
    once: bool,
) -> Result<()> {
    crate::port::ensure_available(config.port).await?;

    let (stop_tx, mut stop_rx) = mpsc::unbounded_channel::<()>();

    let state = ServerState::new(
        config.clone(),
        identity.clone(),
        receive_pin,
        output.mode,
        once,
        Some(stop_tx),
    );
    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));

    let _responder = discovery::run_responder(config.clone(), identity).await?;

    if output.show_human_progress() {
        println!(
            "{} is ready to receive on port {} ({})",
            config.alias,
            config.port,
            if config.https { "HTTPS" } else { "HTTP" }
        );
        println!("Saving files to {}", config.receive_dir.display());
        if once {
            println!("Waiting for one transfer, then exiting.");
        } else {
            println!("Press Ctrl+C to stop.");
        }
    } else if output.is_json() {
        print_json(&ReceiveEventJson::Ready {
            alias: config.alias.clone(),
            port: config.port,
            https: config.https,
            receive_dir: config.receive_dir.display().to_string(),
        });
    }

    let server_task = tokio::spawn(async move {
        if config.https {
            run_https(state, addr).await
        } else {
            run_http(state, addr).await
        }
    });

    tokio::select! {
        _ = signal::ctrl_c() => {},
        Some(()) = stop_rx.recv() => {},
    }

    if output.show_human_progress() {
        println!("Shutting down...");
    } else if output.is_json() {
        print_json(&ReceiveEventJson::Shutdown);
    }
    server_task.abort();

    Ok(())
}
