use std::net::SocketAddr;

use anyhow::Result;
use tokio::net::TcpListener;

use crate::error::CliError;

/// Fail fast when the TCP port is already bound (e.g. official LocalSend app running).
pub async fn ensure_available(port: u16) -> Result<()> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    match TcpListener::bind(addr).await {
        Ok(_) => Ok(()),
        Err(_) => Err(CliError::PortInUse { port }.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn detects_bound_port() {
        let port = pick_free_port().await;
        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        let _guard = TcpListener::bind(addr).await.unwrap();
        let err = ensure_available(port).await.unwrap_err();
        assert!(err.to_string().contains("already in use"));
    }

    async fn pick_free_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap().port()
    }
}
