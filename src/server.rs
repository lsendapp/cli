use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use axum::body::Body;
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_server::tls_rustls::RustlsConfig;
use futures_util::StreamExt;
use localsend::http::dto::ErrorResponse;
use localsend::http::dto_v2::{
    InfoResponseDtoV2, PrepareUploadRequestDtoV2, PrepareUploadResponseDtoV2, RegisterDtoV2,
    RegisterResponseDtoV2,
};
use localsend::model::transfer::FileDto;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::config::AppConfig;
use crate::identity::Identity;

#[derive(Clone)]
pub struct ServerState {
    pub config: AppConfig,
    pub identity: Identity,
    inner: Arc<Mutex<InnerState>>,
}

struct InnerState {
    session: Option<ReceiveSession>,
}

struct ReceiveSession {
    session_id: String,
    sender_ip: String,
    destination_dir: PathBuf,
    files: HashMap<String, ReceivingFileEntry>,
    status: SessionStatus,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SessionStatus {
    Sending,
    Finished,
}

struct ReceivingFileEntry {
    file: FileDto,
    token: String,
    desired_name: String,
    path: Option<PathBuf>,
}

impl ServerState {
    pub fn new(config: AppConfig, identity: Identity) -> Self {
        Self {
            config,
            identity,
            inner: Arc::new(Mutex::new(InnerState { session: None })),
        }
    }
}

pub async fn run_http(state: ServerState, addr: SocketAddr) -> Result<()> {
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("Listening on http://{addr}");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

pub async fn run_https(state: ServerState, addr: SocketAddr) -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let config = RustlsConfig::from_pem(
        state.identity.cert_pem.clone().into_bytes(),
        state.identity.key_pem.clone().into_bytes(),
    )
    .await?;

    let app = build_router(state);
    tracing::info!("Listening on https://{addr}");
    axum_server::bind_rustls(addr, config)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await?;
    Ok(())
}

fn build_router(state: ServerState) -> Router {
    Router::new()
        .route("/api/localsend/v2/info", get(info_handler))
        .route("/api/localsend/v2/register", post(register_handler))
        .route("/api/localsend/v2/prepare-upload", post(prepare_upload_handler))
        .route("/api/localsend/v2/upload", post(upload_handler))
        .route("/api/localsend/v2/cancel", post(cancel_handler))
        .with_state(state)
}

async fn info_handler(State(state): State<ServerState>) -> Json<InfoResponseDtoV2> {
    Json(info_response(&state))
}

async fn register_handler(
    State(state): State<ServerState>,
    Json(_payload): Json<RegisterDtoV2>,
) -> Json<RegisterResponseDtoV2> {
    Json(register_response(&state))
}

async fn prepare_upload_handler(
    State(state): State<ServerState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    if let Some(response) = check_pin(&query, &headers) {
        return response;
    }

    let mut guard = state.inner.lock().await;
    if guard.session.is_some() {
        return error_response(StatusCode::CONFLICT, "Blocked by another session");
    }

    let dto: PrepareUploadRequestDtoV2 = match serde_json::from_str(&body) {
        Ok(dto) => dto,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Request body malformed"),
    };

    if dto.files.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "Request must contain at least one file");
    }

    let session_id = Uuid::new_v4().to_string();
    let sender_ip = addr.ip().to_string();
    let destination_dir = state.config.receive_dir.clone();

    let mut response_files = HashMap::new();
    let mut session_files = HashMap::new();

    for (id, file) in dto.files {
        let token = Uuid::new_v4().to_string();
        let desired_name = file.file_name.clone();
        response_files.insert(id.clone(), token.clone());
        session_files.insert(
            id,
            ReceivingFileEntry {
                file,
                token,
                desired_name,
                path: None,
            },
        );
    }

    println!(
        "Incoming transfer from {} ({} file(s))",
        dto.info.alias,
        session_files.len()
    );

    guard.session = Some(ReceiveSession {
        session_id: session_id.clone(),
        sender_ip,
        destination_dir,
        files: session_files,
        status: SessionStatus::Sending,
    });

    Json(PrepareUploadResponseDtoV2 {
        session_id,
        files: response_files,
    })
    .into_response()
}

async fn upload_handler(
    State(state): State<ServerState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(params): Query<HashMap<String, String>>,
    body: Body,
) -> Response {
    let session_id = match params.get("sessionId") {
        Some(v) => v.clone(),
        None => return error_response(StatusCode::BAD_REQUEST, "Missing parameters"),
    };
    let file_id = match params.get("fileId") {
        Some(v) => v.clone(),
        None => return error_response(StatusCode::BAD_REQUEST, "Missing parameters"),
    };
    let token = match params.get("token") {
        Some(v) => v.clone(),
        None => return error_response(StatusCode::BAD_REQUEST, "Missing parameters"),
    };

    let mut guard = state.inner.lock().await;
    let Some(session) = guard.session.as_mut() else {
        return error_response(StatusCode::CONFLICT, "No session");
    };

    if session.session_id != session_id {
        return error_response(StatusCode::CONFLICT, "Wrong session id");
    }

    if session.sender_ip != addr.ip().to_string() {
        return error_response(
            StatusCode::FORBIDDEN,
            &format!("Invalid IP address: {}", addr.ip()),
        );
    }

    if session.status != SessionStatus::Sending {
        return error_response(StatusCode::CONFLICT, "Recipient is in wrong state");
    }

    let Some(entry) = session.files.get_mut(&file_id) else {
        return error_response(StatusCode::FORBIDDEN, "Invalid token");
    };

    if entry.token != token {
        return error_response(StatusCode::FORBIDDEN, "Invalid token");
    }

    let desired_name = entry.desired_name.clone();
    let destination_dir = session.destination_dir.clone();
    drop(guard);

    let target_path = unique_path(&destination_dir, &desired_name);
    tokio::fs::create_dir_all(&destination_dir)
        .await
        .ok();

    let save_result = save_stream(body, &target_path).await;

    let mut guard = state.inner.lock().await;
    let Some(session) = guard.session.as_mut() else {
        return error_response(StatusCode::CONFLICT, "No session");
    };
    let Some(entry) = session.files.get_mut(&file_id) else {
        return error_response(StatusCode::FORBIDDEN, "Invalid token");
    };

    match save_result {
        Ok(bytes) => {
            entry.path = Some(target_path.clone());
            println!("Saved {} ({} bytes)", target_path.display(), bytes);
        }
        Err(e) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
        }
    }

    if session.files.values().all(|f| f.path.is_some()) {
        session.status = SessionStatus::Finished;
        println!("Transfer complete.");
        guard.session = None;
    }

    StatusCode::OK.into_response()
}

async fn cancel_handler(State(state): State<ServerState>, Query(params): Query<HashMap<String, String>>) -> StatusCode {
    let requested = params.get("sessionId").cloned();
    let mut guard = state.inner.lock().await;
    if let Some(session) = &guard.session {
        if requested.as_ref() == Some(&session.session_id) || requested.is_none() {
            println!("Transfer cancelled by sender.");
            guard.session = None;
        }
    }
    StatusCode::OK
}

async fn save_stream(body: Body, path: &Path) -> Result<u64, anyhow::Error> {
    let mut file = File::create(path).await?;
    let mut stream = body.into_data_stream();
    let mut total = 0u64;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        total += chunk.len() as u64;
    }

    file.flush().await?;
    Ok(total)
}

fn unique_path(dir: &Path, file_name: &str) -> PathBuf {
    let mut candidate = dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }

    let stem = Path::new(file_name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let ext = Path::new(file_name)
        .extension()
        .map(|s| format!(".{}", s.to_string_lossy()))
        .unwrap_or_default();

    for i in 1..1000 {
        candidate = dir.join(format!("{stem} ({i}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }

    dir.join(format!("{stem}-{}{ext}", Uuid::new_v4()))
}

fn info_response(state: &ServerState) -> InfoResponseDtoV2 {
    InfoResponseDtoV2 {
        alias: state.config.alias.clone(),
        version: crate::config::PROTOCOL_VERSION.to_string(),
        device_model: Some(std::env::consts::OS.to_string()),
        device_type: Some(localsend::model::discovery::DeviceType::Headless),
        fingerprint: state.identity.fingerprint.clone(),
        download: false,
    }
}

fn register_response(state: &ServerState) -> RegisterResponseDtoV2 {
    RegisterResponseDtoV2 {
        alias: state.config.alias.clone(),
        version: crate::config::PROTOCOL_VERSION.to_string(),
        device_model: Some(std::env::consts::OS.to_string()),
        device_type: Some(localsend::model::discovery::DeviceType::Headless),
        fingerprint: state.identity.fingerprint.clone(),
        download: false,
    }
}

fn error_response(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(ErrorResponse {
            message: message.to_string(),
        }),
    )
        .into_response()
}

fn check_pin(query: &HashMap<String, String>, headers: &HeaderMap) -> Option<Response> {
    let pin = std::env::var("LSEND_RECEIVE_PIN").ok();
    let Some(expected) = pin.filter(|p| !p.is_empty()) else {
        return None;
    };

    let provided = query
        .get("pin")
        .cloned()
        .or_else(|| {
            headers
                .get("pin")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        });

    match provided {
        Some(value) if value == expected => None,
        Some(_) => Some(error_response(StatusCode::UNAUTHORIZED, "Invalid PIN")),
        None => Some(error_response(StatusCode::UNAUTHORIZED, "PIN required")),
    }
}
