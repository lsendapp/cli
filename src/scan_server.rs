use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::{ConnectInfo, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_server::tls_rustls::RustlsConfig;
use localsend::http::dto_v2::InfoResponseDtoV2;
use localsend::model::discovery::DeviceType;
use serde::Deserialize;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{debug, info};

use crate::config::AppConfig;
use crate::discovery::DiscoveredDevice;
use crate::identity::Identity;

#[derive(Clone)]
pub struct DiscoveryServerState {
    config: AppConfig,
    identity: Identity,
    devices: Arc<Mutex<HashMap<String, DiscoveredDevice>>>,
}

pub struct DiscoveryServerHandle {
    task: JoinHandle<()>,
}

impl DiscoveryServerHandle {
    pub fn abort(self) {
        self.task.abort();
    }
}

pub async fn start(config: AppConfig, identity: Identity, devices: Arc<Mutex<HashMap<String, DiscoveredDevice>>>) -> Result<DiscoveryServerHandle> {
    let state = DiscoveryServerState {
        config: config.clone(),
        identity: identity.clone(),
        devices,
    };

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    let app = Router::new()
        .route("/api/localsend/v1/info", get(info_handler))
        .route("/api/localsend/v2/info", get(info_handler))
        .route("/api/localsend/v1/register", post(register_handler))
        .route("/api/localsend/v2/register", post(register_handler))
        .with_state(state)
        .into_make_service_with_connect_info::<SocketAddr>();

    let task = if config.https {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let tls = RustlsConfig::from_pem(
            identity.cert_pem.clone().into_bytes(),
            identity.key_pem.clone().into_bytes(),
        )
        .await
        .context("Failed to load TLS config for discovery server")?;

        tokio::spawn(async move {
            if let Err(e) = axum_server::bind_rustls(addr, tls).serve(app).await {
                debug!("Discovery HTTPS server stopped: {e}");
            }
        })
    } else {
        tokio::spawn(async move {
            match tokio::net::TcpListener::bind(addr).await {
                Ok(listener) => {
                    if let Err(e) = axum::serve(listener, app).await {
                        debug!("Discovery HTTP server stopped: {e}");
                    }
                }
                Err(e) => debug!("Discovery HTTP server bind failed: {e}"),
            }
        })
    };

    // Give the server a moment to bind before multicast announcements go out.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    info!(
        "Discovery {} server listening on port {}",
        if config.https { "HTTPS" } else { "HTTP" },
        config.port
    );

    Ok(DiscoveryServerHandle { task })
}

async fn info_handler(State(state): State<DiscoveryServerState>) -> Json<InfoResponseDtoV2> {
    Json(info_response(&state))
}

async fn register_handler(
    State(state): State<DiscoveryServerState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(payload): Json<RegisterCompat>,
) -> Result<Json<InfoResponseDtoV2>, (axum::http::StatusCode, Json<localsend::http::dto::ErrorResponse>)> {
    let fingerprint = payload.fingerprint();
    if fingerprint.is_empty() {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            Json(localsend::http::dto::ErrorResponse {
                message: "Missing fingerprint".to_string(),
            }),
        ));
    }

    if fingerprint == state.identity.fingerprint {
        return Err((
            axum::http::StatusCode::PRECONDITION_FAILED,
            Json(localsend::http::dto::ErrorResponse {
                message: "Self-discovered".to_string(),
            }),
        ));
    }

    let https = payload
        .protocol
        .as_deref()
        .map(|p| p.eq_ignore_ascii_case("https"))
        .unwrap_or(state.config.https);

    let device = DiscoveredDevice {
        alias: payload.alias.clone(),
        ip: addr.ip().to_string(),
        port: payload.port.unwrap_or(state.config.port),
        fingerprint: fingerprint.to_string(),
        https,
        version: payload
            .version
            .clone()
            .unwrap_or_else(|| crate::config::PROTOCOL_VERSION.to_string()),
        device_type: payload
            .device_type
            .as_deref()
            .and_then(crate::util::parse_device_type),
        device_model: payload.device_model.clone(),
    };

    info!(
        "Discovered via TCP /register: {} ({})",
        device.alias, device.ip
    );

    state
        .devices
        .lock()
        .await
        .insert(fingerprint.to_string(), device);

    Ok(Json(info_response(&state)))
}

fn info_response(state: &DiscoveryServerState) -> InfoResponseDtoV2 {
    InfoResponseDtoV2 {
        alias: state.config.alias.clone(),
        version: crate::config::PROTOCOL_VERSION.to_string(),
        device_model: Some(crate::util::os_display_name()),
        device_type: Some(DeviceType::Headless),
        fingerprint: state.identity.fingerprint.clone(),
        download: false,
    }
}

/// Accepts v2 `fingerprint` and legacy v1 `token` field names.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegisterCompat {
    alias: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    device_model: Option<String>,
    #[serde(default)]
    device_type: Option<String>,
    #[serde(default)]
    fingerprint: String,
    #[serde(default)]
    token: String,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    protocol: Option<String>,
}

impl RegisterCompat {
    fn fingerprint(&self) -> &str {
        if !self.fingerprint.is_empty() {
            &self.fingerprint
        } else {
            &self.token
        }
    }
}
