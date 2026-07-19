use std::net::SocketAddr;

use anyhow::Result;
use tokio::signal;
use tokio::sync::mpsc;

use crate::config::AppConfig;
use crate::discovery;
use crate::events::ReceiveEvent;
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
    // Bind before spawning so the port is known to be available before
    // discovery announcements go out - no 100 ms sleep guesswork.
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|_| crate::error::CliError::PortInUse { port: config.port })?;
    let server_config = config.clone();

    let server_task = tokio::spawn(async move {
        if server_config.https {
            run_https(state, listener).await
        } else {
            run_http(state, listener).await
        }
    });

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

/// Run the receiver with a programmatic event channel. The CLI's `receive`
/// subcommand uses [`run`]; the desktop app uses this entry point so it can
/// observe receive-lifecycle events without parsing stdout.
///
/// Returns the server task handle and an event receiver. The caller owns the
/// task's lifetime: dropping the handle aborts the server. The caller is
/// also responsible for announcing shutdown on the event channel when it
/// decides to stop the receiver.
pub async fn run_with_events(
    config: AppConfig,
    identity: Identity,
    receive_pin: Option<String>,
    output_mode: crate::output::OutputMode,
) -> Result<(
    mpsc::UnboundedSender<()>,
    mpsc::UnboundedReceiver<ReceiveEvent>,
)> {
    let (stop_tx, mut stop_rx) = mpsc::unbounded_channel::<()>();
    let (event_tx, event_rx) = mpsc::unbounded_channel::<ReceiveEvent>();

    let state = ServerState::new_with_events(
        config.clone(),
        identity.clone(),
        receive_pin,
        output_mode,
        false,
        Some(stop_tx.clone()),
        Some(event_tx.clone()),
    );
    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    // Bind before spawning so the port is known to be available before
    // discovery announcements go out - no 100 ms sleep guesswork.
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|_| crate::error::CliError::PortInUse { port: config.port })?;
    let server_config = config.clone();

    let server_task = tokio::spawn(async move {
        let _ = if server_config.https {
            let _ = rustls::crypto::ring::default_provider().install_default();
            run_https(state, listener).await
        } else {
            run_http(state, listener).await
        };
    });

    // Start the multicast discovery responder so peers can find us by UDP scan.
    // Failure here is non-fatal for the desktop app: receive still works for
    // peers that already know our IP, and the tray will surface "degraded".
    let _responder = discovery::run_responder(config.clone(), identity.clone()).await;
    let responder_handle = match _responder {
        Ok(h) => Some(h),
        Err(e) => {
            tracing::warn!("discovery responder failed to start: {e}");
            None
        }
    };

    let _ = event_tx.send(ReceiveEvent::Ready {
        alias: config.alias.clone(),
        port: config.port,
        https: config.https,
        receive_dir: config.receive_dir.clone(),
    });

    // Supervisor: translate the caller's stop signal into a Shutdown event
    // (the server task itself has no lifecycle hook for it) and tear down
    // the server + discovery tasks so the process can idle. `stop_rx` fires
    // both on an explicit stop message and when `stop_tx` is dropped, so the
    // desktop app's "quit" path works either way.
    {
        let event_tx = event_tx.clone();
        tokio::spawn(async move {
            let _ = stop_rx.recv().await;
            let _ = event_tx.send(ReceiveEvent::Shutdown);
            server_task.abort();
            // MulticastGuard aborts its listener tasks on drop.
            drop(responder_handle);
        });
    }

    Ok((stop_tx, event_rx))
}
