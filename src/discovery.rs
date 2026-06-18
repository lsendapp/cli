use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use localsend::http::dto_v2::{MulticastMessageV2, ProtocolTypeV2};
use localsend::model::discovery::DeviceType;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};

use crate::config::{AppConfig, DEFAULT_MULTICAST_GROUP};
use crate::identity::Identity;

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

pub async fn scan(config: &AppConfig, identity: &Identity, timeout_ms: u64) -> Result<Vec<DiscoveredDevice>> {
    let devices = Arc::new(Mutex::new(HashMap::<String, DiscoveredDevice>::new()));
    let multicast_group: Ipv4Addr = DEFAULT_MULTICAST_GROUP.parse()?;
    let socket = open_multicast_socket(config.port, multicast_group)?;

    let listener = start_multicast_listener(
        config.clone(),
        identity.clone(),
        Some(devices.clone()),
        Arc::clone(&socket),
    )
    .await?;

    send_announcement(&socket, config, identity, multicast_group).await?;

    sleep(Duration::from_millis(timeout_ms)).await;

    drop(listener);

    let found = devices.lock().await.clone();
    Ok(found.into_values().collect())
}

pub async fn run_responder(config: AppConfig, identity: Identity) -> Result<MulticastGuard> {
    let multicast_group: Ipv4Addr = DEFAULT_MULTICAST_GROUP.parse()?;
    let socket = open_multicast_socket(config.port, multicast_group)?;
    let guard = start_multicast_listener(config.clone(), identity.clone(), None, Arc::clone(&socket)).await?;
    send_announcement(&socket, &config, &identity, multicast_group).await?;
    Ok(guard)
}

pub struct MulticastGuard {
    task: tokio::task::JoinHandle<()>,
}

impl Drop for MulticastGuard {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn start_multicast_listener(
    config: AppConfig,
    identity: Identity,
    collect: Option<Arc<Mutex<HashMap<String, DiscoveredDevice>>>>,
    socket: Arc<UdpSocket>,
) -> Result<MulticastGuard> {
    let multicast_group: Ipv4Addr = DEFAULT_MULTICAST_GROUP.parse()?;

    let task = tokio::spawn(async move {
        let mut buf = vec![0u8; 65535];
        loop {
            match timeout(Duration::from_secs(1), socket.recv_from(&mut buf)).await {
                Ok(Ok((len, addr))) => {
                    let payload = &buf[..len];
                    if let Ok(message) = serde_json::from_slice::<MulticastMessageV2>(payload) {
                        if message.fingerprint == identity.fingerprint {
                            continue;
                        }

                        let device = message_to_device(&message, addr.ip(), config.port);
                        if let Some(collect) = &collect {
                            collect
                                .lock()
                                .await
                                .insert(device.fingerprint.clone(), device);
                        }

                        if message.announce {
                            let response = build_multicast_message(&config, &identity, false);
                            if let Ok(json) = serde_json::to_vec(&response) {
                                let dest = SocketAddr::new(multicast_group.into(), config.port);
                                let _ = socket.send_to(&json, dest).await;
                            }
                        }
                    }
                }
                Ok(Err(e)) => {
                    tracing::debug!("Multicast recv error: {e}");
                }
                Err(_) => {}
            }
        }
    });

    Ok(MulticastGuard { task })
}

async fn send_announcement(
    socket: &UdpSocket,
    config: &AppConfig,
    identity: &Identity,
    multicast_group: Ipv4Addr,
) -> Result<()> {
    let message = build_multicast_message(config, identity, true);
    let json = serde_json::to_vec(&message).context("Failed to serialize multicast message")?;
    let dest = SocketAddr::new(multicast_group.into(), config.port);

    for delay_ms in [0u64, 100, 500] {
        if delay_ms > 0 {
            sleep(Duration::from_millis(delay_ms)).await;
        }
        socket
            .send_to(&json, dest)
            .await
            .context("Failed to send announcement")?;
    }

    Ok(())
}

fn open_multicast_socket(port: u16, multicast_group: Ipv4Addr) -> Result<Arc<UdpSocket>> {
    let std_socket = create_multicast_socket(port, multicast_group)?;
    std_socket.set_nonblocking(true)?;
    let socket = UdpSocket::from_std(std_socket.into())?;
    Ok(Arc::new(socket))
}

fn create_multicast_socket(port: u16, multicast_group: Ipv4Addr) -> Result<Socket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .context("Failed to create UDP socket")?;
    socket.set_reuse_address(true)?;
    socket.bind(&SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), port).into())?;
    socket.join_multicast_v4(&multicast_group, &Ipv4Addr::UNSPECIFIED)?;
    Ok(socket)
}

fn build_multicast_message(
    config: &AppConfig,
    identity: &Identity,
    announce: bool,
) -> MulticastMessageV2 {
    MulticastMessageV2 {
        alias: config.alias.clone(),
        version: crate::config::PROTOCOL_VERSION.to_string(),
        device_model: Some(std::env::consts::OS.to_string()),
        device_type: Some(DeviceType::Headless),
        fingerprint: identity.fingerprint.clone(),
        port: config.port,
        protocol: if config.https {
            ProtocolTypeV2::Https
        } else {
            ProtocolTypeV2::Http
        },
        download: false,
        announce,
    }
}

fn message_to_device(message: &MulticastMessageV2, ip: IpAddr, fallback_port: u16) -> DiscoveredDevice {
    DiscoveredDevice {
        alias: message.alias.clone(),
        ip: ip.to_string(),
        port: if message.port == 0 {
            fallback_port
        } else {
            message.port
        },
        fingerprint: message.fingerprint.clone(),
        https: message.protocol == ProtocolTypeV2::Https,
        version: message.version.clone(),
        device_type: message.device_type.clone(),
        device_model: message.device_model.clone(),
    }
}

pub async fn resolve_target(
    target: &str,
    config: &AppConfig,
    identity: &Identity,
) -> Result<DiscoveredDevice> {
    if looks_like_ip(target) {
        return Ok(DiscoveredDevice {
            alias: target.to_string(),
            ip: target.to_string(),
            port: config.port,
            fingerprint: String::new(),
            https: config.https,
            version: crate::config::PROTOCOL_VERSION.to_string(),
            device_type: None,
            device_model: None,
        });
    }

    let devices = scan(config, identity, AppConfig::DEFAULT_DISCOVERY_TIMEOUT_MS * 4).await?;
    devices
        .into_iter()
        .find(|d| d.alias.eq_ignore_ascii_case(target))
        .with_context(|| format!("No device found with alias \"{target}\""))
}

fn looks_like_ip(value: &str) -> bool {
    value.parse::<IpAddr>().is_ok()
}
