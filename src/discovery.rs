use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use localsend::http::dto_v2::ProtocolTypeV2;
use localsend::model::discovery::DeviceType;
use serde::Deserialize;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};
use tracing::{debug, info, warn};

use crate::config::{AppConfig, DEFAULT_MULTICAST_GROUP};
use crate::identity::Identity;
use crate::legacy::legacy_http_scan;
use crate::network::list_ipv4_interfaces;
use crate::scan_server::start as start_discovery_server;

#[derive(Clone, Debug)]
pub struct DiscoveredDevice {
    pub alias: String,
    pub ip: String,
    pub port: u16,
    pub fingerprint: String,
    pub https: bool,
    pub version: String,
    pub device_type: Option<DeviceType>,
    pub device_model: Option<String>,
}

impl DiscoveredDevice {
    pub fn display_label(&self) -> String {
        format!(
            "{} ({}{}:{})",
            self.alias,
            if self.https { "https://" } else { "http://" },
            self.ip,
            self.port
        )
    }
}

const MULTICAST_MIN_LISTEN_MS: u64 = 3500;

pub async fn scan(config: &AppConfig, identity: &Identity, timeout_ms: u64) -> Result<Vec<DiscoveredDevice>> {
    let devices = Arc::new(Mutex::new(HashMap::<String, DiscoveredDevice>::new()));
    let multicast_group: Ipv4Addr = DEFAULT_MULTICAST_GROUP.parse()?;
    let listen_ms = timeout_ms.max(MULTICAST_MIN_LISTEN_MS);

    let discovery_server = match start_discovery_server(config.clone(), identity.clone(), devices.clone()).await {
        Ok(handle) => Some(handle),
        Err(e) => {
                warn!(
                    "Could not start discovery HTTP server on port {}: {e}. \
                     Close the official app if it is running, or use another --port. \
                     Peers may only respond via UDP.",
                    config.port
                );
            None
        }
    };

    let listen_sockets = open_multicast_listen_sockets(config.port, multicast_group)
        .context("Failed to open multicast listen sockets")?;
    let announce_sockets = open_multicast_announce_sockets(multicast_group);

    debug!(
        "Scan: {} listen socket(s), {} announce socket(s), port {}, https={}",
        listen_sockets.len(),
        announce_sockets.len(),
        config.port,
        config.https
    );

    let listener = if listen_sockets.is_empty() {
        None
    } else {
        Some(start_multicast_listener(
            config.clone(),
            identity.clone(),
            Some(devices.clone()),
            listen_sockets,
        )
        .await?)
    };

    if !announce_sockets.is_empty() {
        send_announcement(announce_sockets, config, identity, multicast_group).await?;
    }

    sleep(Duration::from_millis(listen_ms)).await;

    if let Some(listener) = listener {
        drop(listener);
    }
    if let Some(server) = discovery_server {
        server.abort();
    }

    let mut found: Vec<_> = devices.lock().await.values().cloned().collect();
    if found.is_empty() {
        found = legacy_http_scan(config, identity).await?;
    }

    if found.is_empty() {
        info!("Scan finished with no devices discovered");
    } else {
        info!("Scan discovered {} device(s)", found.len());
    }

    Ok(found)
}

pub async fn run_responder(config: AppConfig, identity: Identity) -> Result<MulticastGuard> {
    let multicast_group: Ipv4Addr = DEFAULT_MULTICAST_GROUP.parse()?;
    let listen_sockets =
        open_multicast_listen_sockets(config.port, multicast_group).context("Failed to open multicast listen sockets")?;
    let announce_sockets = open_multicast_announce_sockets(multicast_group);
    let guard = start_multicast_listener(config.clone(), identity.clone(), None, listen_sockets).await?;
    if !announce_sockets.is_empty() {
        send_announcement(announce_sockets, &config, &identity, multicast_group).await?;
    }
    Ok(guard)
}

pub struct MulticastGuard {
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for MulticastGuard {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

async fn start_multicast_listener(
    config: AppConfig,
    identity: Identity,
    collect: Option<Arc<Mutex<HashMap<String, DiscoveredDevice>>>>,
    sockets: Vec<Arc<UdpSocket>>,
) -> Result<MulticastGuard> {
    let mut tasks = Vec::with_capacity(sockets.len());

    for socket in sockets {
        let config = config.clone();
        let identity = identity.clone();
        let collect = collect.clone();
        let task = tokio::spawn(async move {
            let mut buf = vec![0u8; 65535];
            loop {
                match timeout(Duration::from_secs(1), socket.recv_from(&mut buf)).await {
                    Ok(Ok((len, addr))) => {
                        handle_datagram(&buf[..len], addr.ip(), &config, &identity, collect.as_ref(), &socket)
                            .await;
                    }
                    Ok(Err(e)) => {
                        debug!("Multicast recv error: {e}");
                    }
                    Err(_) => {}
                }
            }
        });
        tasks.push(task);
    }

    Ok(MulticastGuard { tasks })
}

async fn handle_datagram(
    payload: &[u8],
    source_ip: IpAddr,
    config: &AppConfig,
    identity: &Identity,
    collect: Option<&Arc<Mutex<HashMap<String, DiscoveredDevice>>>>,
    socket: &UdpSocket,
) {
    let Ok(message) = serde_json::from_slice::<MulticastMessageCompat>(payload) else {
        debug!(
            "Ignored unparsable multicast payload from {source_ip}: {}",
            String::from_utf8_lossy(payload).chars().take(200).collect::<String>()
        );
        return;
    };

    if message.fingerprint == identity.fingerprint {
        return;
    }

    let device = message_to_device(&message, source_ip, config.port, config.https);
    info!(
        "Discovered via UDP multicast: {} ({}) announce={}",
        device.alias,
        device.ip,
        message.is_announce()
    );

    if let Some(collect) = collect {
        collect
            .lock()
            .await
            .insert(device.fingerprint.clone(), device);
    }

    if message.is_announce() {
        let response = build_multicast_message(config, identity, false);
        if let Ok(json) = serde_json::to_vec(&response) {
            let multicast_group: Ipv4Addr = DEFAULT_MULTICAST_GROUP.parse().unwrap_or(Ipv4Addr::new(224, 0, 0, 167));
            let dest = SocketAddr::new(multicast_group.into(), config.port);
            let _ = socket.send_to(&json, dest).await;
        }
    }
}

async fn send_announcement(
    sockets: Vec<Arc<UdpSocket>>,
    config: &AppConfig,
    identity: &Identity,
    multicast_group: Ipv4Addr,
) -> Result<()> {
    let message = build_multicast_message(config, identity, true);
    let json = serde_json::to_vec(&message).context("Failed to serialize multicast message")?;
    let dest = SocketAddr::new(multicast_group.into(), config.port);

    for delay_ms in [100u64, 500, 2000] {
        sleep(Duration::from_millis(delay_ms)).await;
        for socket in &sockets {
            socket
                .send_to(&json, dest)
                .await
                .context("Failed to send announcement")?;
        }
    }

    Ok(())
}

/// Listen on the standard LocalSend UDP port (53317). Matches the official app listener.
fn open_multicast_listen_sockets(port: u16, multicast_group: Ipv4Addr) -> Result<Vec<Arc<UdpSocket>>> {
    let interfaces = list_ipv4_interfaces();
    let mut sockets = Vec::new();

    if interfaces.is_empty() {
        sockets.push(create_bound_udp_socket(port, multicast_group, Ipv4Addr::UNSPECIFIED)?);
        return Ok(sockets);
    }

    for interface in interfaces {
        match create_bound_udp_socket(port, multicast_group, interface) {
            Ok(socket) => {
                debug!("Multicast listen socket on 0.0.0.0:{port} (iface {interface})");
                sockets.push(socket);
            }
            Err(e) => {
                debug!("Skipping multicast listen on {interface}: {e}");
            }
        }
    }

    Ok(sockets)
}

/// Ephemeral UDP ports for outbound announcements (official app behavior).
fn open_multicast_announce_sockets(multicast_group: Ipv4Addr) -> Vec<Arc<UdpSocket>> {
    let interfaces = list_ipv4_interfaces();
    let mut sockets = Vec::new();

    let targets: Vec<Ipv4Addr> = if interfaces.is_empty() {
        vec![Ipv4Addr::UNSPECIFIED]
    } else {
        interfaces
    };

    for interface in targets {
        match create_bound_udp_socket(0, multicast_group, interface) {
            Ok(socket) => {
                debug!("Multicast announce socket on iface {interface}");
                sockets.push(socket);
            }
            Err(e) => {
                debug!("Skipping multicast announce on {interface}: {e}");
            }
        }
    }

    sockets
}

fn create_bound_udp_socket(
    port: u16,
    multicast_group: Ipv4Addr,
    interface: Ipv4Addr,
) -> Result<Arc<UdpSocket>> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .context("Failed to create UDP socket")?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    {
        let _ = socket.set_reuse_port(true);
    }

    socket
        .bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)).into())
        .with_context(|| format!("Failed to bind UDP 0.0.0.0:{port}"))?;

    socket
        .join_multicast_v4(&multicast_group, &interface)
        .with_context(|| format!("Failed to join multicast on interface {interface}"))?;

    if !interface.is_unspecified() {
        let _ = socket.set_multicast_if_v4(&interface);
    }

    socket.set_nonblocking(true)?;
    Ok(Arc::new(UdpSocket::from_std(socket.into())?))
}

fn build_multicast_message(
    config: &AppConfig,
    identity: &Identity,
    announce: bool,
) -> MulticastMessageCompat {
    MulticastMessageCompat {
        alias: config.alias.clone(),
        version: Some(crate::config::PROTOCOL_VERSION.to_string()),
        device_model: Some(std::env::consts::OS.to_string()),
        device_type: Some("headless".to_string()),
        fingerprint: identity.fingerprint.clone(),
        port: Some(config.port),
        protocol: Some(if config.https {
            ProtocolTypeV2::Https
        } else {
            ProtocolTypeV2::Http
        }),
        download: false,
        announce,
        announcement: announce,
    }
}

fn message_to_device(
    message: &MulticastMessageCompat,
    ip: IpAddr,
    fallback_port: u16,
    fallback_https: bool,
) -> DiscoveredDevice {
    DiscoveredDevice {
        alias: message.alias.clone(),
        ip: ip.to_string(),
        port: message.port.unwrap_or(fallback_port),
        fingerprint: message.fingerprint.clone(),
        https: message
            .protocol
            .as_ref()
            .map(|p| *p == ProtocolTypeV2::Https)
            .unwrap_or(fallback_https),
        version: message
            .version
            .clone()
            .unwrap_or_else(|| crate::config::PROTOCOL_VERSION.to_string()),
        device_type: message
            .device_type
            .as_deref()
            .and_then(crate::util::parse_device_type),
        device_model: message.device_model.clone(),
    }
}

pub async fn resolve_target(
    target: &str,
    config: &AppConfig,
    identity: &Identity,
    allow_scan: bool,
) -> Result<(DiscoveredDevice, &'static str)> {
    if looks_like_ip(target) {
        return Ok((
            DiscoveredDevice {
                alias: target.to_string(),
                ip: target.to_string(),
                port: config.port,
                fingerprint: String::new(),
                https: config.https,
                version: crate::config::PROTOCOL_VERSION.to_string(),
                device_type: None,
                device_model: None,
            },
            "ip",
        ));
    }

    if !allow_scan {
        return Err(crate::error::CliError::TargetNotFound {
            target: target.to_string(),
        }
        .into());
    }

    let devices = scan(config, identity, AppConfig::DEFAULT_DISCOVERY_TIMEOUT_MS * 4).await?;
    let device = devices
        .into_iter()
        .find(|d| d.alias.eq_ignore_ascii_case(target))
        .ok_or_else(|| crate::error::CliError::TargetNotFound {
            target: target.to_string(),
        })?;

    Ok((device, "scan"))
}

fn looks_like_ip(value: &str) -> bool {
    value.parse::<IpAddr>().is_ok()
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct MulticastMessageCompat {
    alias: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device_type: Option<String>,
    fingerprint: String,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    protocol: Option<ProtocolTypeV2>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    download: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    announce: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    announcement: bool,
}

impl MulticastMessageCompat {
    fn is_announce(&self) -> bool {
        self.announce || self.announcement
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_official_multicast_json() {
        let json = br#"{"alias":"Happy Orange","version":"2.1","deviceModel":"macOS","deviceType":"desktop","fingerprint":"abc123","port":53317,"protocol":"https","download":false,"announce":true,"announcement":true}"#;
        let msg: MulticastMessageCompat = serde_json::from_slice(json).expect("parse");
        assert_eq!(msg.alias, "Happy Orange");
        assert_eq!(msg.device_type.as_deref(), Some("desktop"));
        assert!(msg.is_announce());
    }
}
