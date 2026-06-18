use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use axum::body::Body;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_server::tls_rustls::RustlsConfig;
use futures_util::StreamExt;
use localsend::http::dto::ErrorResponse;
use localsend::http::dto_v2::{
    InfoResponseDtoV2, PrepareUploadResponseDtoV2, RegisterDtoV2, RegisterResponseDtoV2,
};
use localsend::model::transfer::FileDto;
use serde::Deserialize;
use serde::Serialize;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, mpsc};
use uuid::Uuid;

use crate::config::AppConfig;
use crate::identity::Identity;
use crate::output::{OutputMode, ReceiveEventJson, print_json};
use crate::text_send::text_file_name;

/// Delay before returning 204 for embedded text messages so the mobile sender
/// can finish its SendPage transition before reading the response.
const TEXT_MESSAGE_RESPONSE_DELAY: Duration = Duration::from_millis(1200);

#[derive(Clone)]
pub struct ServerState {
    pub config: AppConfig,
    pub identity: Identity,
    pub receive_pin: Option<String>,
    output: OutputMode,
    stop_after_transfer: bool,
    stop_tx: Option<mpsc::UnboundedSender<()>>,
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
    pub fn new(
        config: AppConfig,
        identity: Identity,
        receive_pin: Option<String>,
        output: OutputMode,
        stop_after_transfer: bool,
        stop_tx: Option<mpsc::UnboundedSender<()>>,
    ) -> Self {
        Self {
            config,
            identity,
            receive_pin,
            output,
            stop_after_transfer,
            stop_tx,
            inner: Arc::new(Mutex::new(InnerState { session: None })),
        }
    }

    fn notify_human(&self, message: impl AsRef<str>) {
        if self.output == OutputMode::Human {
            println!("{}", message.as_ref());
        }
    }

    fn emit_json_event(&self, event: ReceiveEventJson) {
        if self.output == OutputMode::Json {
            print_json(&event);
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
        .route("/api/localsend/v1/info", get(info_v1_handler))
        .route("/api/localsend/v2/info", get(info_handler))
        .route("/api/localsend/v1/register", post(register_v1_handler))
        .route("/api/localsend/v2/register", post(register_handler))
        .route("/api/localsend/v1/send-request", post(prepare_upload_v1_handler))
        .route("/api/localsend/v2/prepare-upload", post(prepare_upload_v2_handler))
        .route("/api/localsend/v1/send", post(upload_v1_handler))
        .route("/api/localsend/v2/upload", post(upload_v2_handler))
        .route("/api/localsend/v1/cancel", post(cancel_handler))
        .route("/api/localsend/v2/cancel", post(cancel_handler))
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .with_state(state)
}

/// JSON shape expected by legacy v1 clients (manual IP entry uses `peerProtocolVersion = 1.0`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct InfoResponseJson {
    alias: String,
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_type: Option<String>,
    fingerprint: String,
    download: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct RegisterPayloadCompat {
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

impl RegisterPayloadCompat {
    fn fingerprint(&self) -> &str {
        if !self.fingerprint.is_empty() {
            &self.fingerprint
        } else {
            &self.token
        }
    }
}

async fn info_v1_handler(
    State(state): State<ServerState>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<InfoResponseJson>, Response> {
    if is_self_fingerprint(&state, query.get("fingerprint")) {
        return Err(error_response(StatusCode::PRECONDITION_FAILED, "Self-discovered"));
    }
    Ok(Json(info_response_json(&state)))
}

async fn register_v1_handler(
    State(state): State<ServerState>,
    Json(payload): Json<RegisterPayloadCompat>,
) -> Result<Json<InfoResponseJson>, Response> {
    let fingerprint = payload.fingerprint();
    if fingerprint.is_empty() {
        return Err(error_response(StatusCode::BAD_REQUEST, "Missing fingerprint"));
    }
    if fingerprint == state.identity.fingerprint {
        return Err(error_response(StatusCode::PRECONDITION_FAILED, "Self-discovered"));
    }
    Ok(Json(info_response_json(&state)))
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

async fn prepare_upload_v1_handler(
    State(state): State<ServerState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    prepare_upload_handler(state, addr, query, headers, body, false).await
}

async fn prepare_upload_v2_handler(
    State(state): State<ServerState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    prepare_upload_handler(state, addr, query, headers, body, true).await
}

async fn prepare_upload_handler(
    state: ServerState,
    addr: SocketAddr,
    query: HashMap<String, String>,
    headers: HeaderMap,
    body: String,
    v2_api: bool,
) -> Response {
    if let Some(response) = check_pin(&state, &query, &headers) {
        return response;
    }

    let guard = state.inner.lock().await;
    if guard.session.is_some() {
        return error_response(StatusCode::CONFLICT, "Blocked by another session");
    }
    drop(guard);

    let request = match parse_prepare_upload_request(&body) {
        Ok(request) => request,
        Err(error) => {
            tracing::warn!("Prepare upload parse error: {error}");
            return error_response(StatusCode::BAD_REQUEST, "Request body malformed");
        }
    };

    if request.files.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "Request must contain at least one file");
    }

    if let Some(text) = extract_embedded_text_message(&request.files) {
        return handle_embedded_text_message(state, &request.sender_alias, text).await;
    }

    let session_id = Uuid::new_v4().to_string();
    let sender_ip = addr.ip().to_string();
    let destination_dir = state.config.receive_dir.clone();

    let mut response_files = HashMap::new();
    let mut session_files = HashMap::new();

    for (id, file) in request.files {
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

    state.notify_human(format!(
        "Incoming transfer from {} ({} file(s))",
        request.sender_alias,
        session_files.len()
    ));
    if state.output == OutputMode::Json {
        state.emit_json_event(ReceiveEventJson::TransferStarted {
            sender_alias: request.sender_alias,
            file_count: session_files.len(),
        });
    }

    let mut guard = state.inner.lock().await;
    guard.session = Some(ReceiveSession {
        session_id: session_id.clone(),
        sender_ip,
        destination_dir,
        files: session_files,
        status: SessionStatus::Sending,
    });

    if v2_api {
        Json(PrepareUploadResponseDtoV2 {
            session_id,
            files: response_files,
        })
        .into_response()
    } else {
        Json(response_files).into_response()
    }
}

struct ParsedPrepareUpload {
    sender_alias: String,
    files: HashMap<String, FileDto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrepareUploadRequestCompat {
    info: PrepareUploadInfoCompat,
    files: HashMap<String, PrepareUploadFileCompat>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrepareUploadInfoCompat {
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
    #[serde(default)]
    download: Option<bool>,
    #[serde(default, rename = "hasWebInterface")]
    has_web_interface: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrepareUploadFileCompat {
    #[serde(default)]
    id: String,
    file_name: String,
    size: u64,
    file_type: String,
    #[serde(default)]
    hash: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    preview: Option<String>,
    #[serde(default)]
    metadata: Option<localsend::model::transfer::FileMetadata>,
}

impl PrepareUploadFileCompat {
    fn into_file_dto(self, map_key: &str) -> FileDto {
        FileDto {
            id: if self.id.is_empty() {
                map_key.to_string()
            } else {
                self.id
            },
            file_name: self.file_name,
            size: self.size,
            file_type: self.file_type,
            sha256: self.sha256.or(self.hash),
            preview: self.preview,
            metadata: self.metadata,
        }
    }
}

#[derive(Debug)]
enum PrepareUploadParseError {
    Json(serde_json::Error),
    MissingField(&'static str),
    InvalidField(&'static str),
}

impl std::fmt::Display for PrepareUploadParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(error) => write!(f, "json error: {error}"),
            Self::MissingField(field) => write!(f, "missing field: {field}"),
            Self::InvalidField(field) => write!(f, "invalid field: {field}"),
        }
    }
}

fn parse_prepare_upload_request(body: &str) -> Result<ParsedPrepareUpload, PrepareUploadParseError> {
    if let Ok(dto) = serde_json::from_str::<PrepareUploadRequestCompat>(body) {
        let mut files = HashMap::with_capacity(dto.files.len());
        for (id, file) in dto.files {
            files.insert(id.clone(), file.into_file_dto(&id));
        }
        return Ok(ParsedPrepareUpload {
            sender_alias: dto.info.alias,
            files,
        });
    }

    let root: serde_json::Value =
        serde_json::from_str(body).map_err(PrepareUploadParseError::Json)?;
    let info = root
        .get("info")
        .ok_or(PrepareUploadParseError::MissingField("info"))?;
    let alias = read_string_field(info, &["alias"])
        .ok_or(PrepareUploadParseError::MissingField("info.alias"))?;

    let files_value = root
        .get("files")
        .and_then(|value| value.as_object())
        .ok_or(PrepareUploadParseError::MissingField("files"))?;

    let mut files = HashMap::with_capacity(files_value.len());
    for (map_key, file_value) in files_value {
        files.insert(map_key.clone(), parse_file_dto(map_key, file_value)?);
    }

    Ok(ParsedPrepareUpload { sender_alias: alias, files })
}

fn read_string_field(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(found) = value.get(*key).and_then(|v| v.as_str()) {
            return Some(found.to_string());
        }
    }
    None
}

fn read_u64_field(value: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    for key in keys {
        let Some(raw) = value.get(*key) else {
            continue;
        };
        if let Some(number) = raw.as_u64() {
            return Some(number);
        }
        if let Some(text) = raw.as_str() {
            if let Ok(number) = text.parse::<u64>() {
                return Some(number);
            }
        }
    }
    None
}

fn parse_file_dto(
    map_key: &str,
    value: &serde_json::Value,
) -> Result<FileDto, PrepareUploadParseError> {
    let object = value
        .as_object()
        .ok_or(PrepareUploadParseError::InvalidField("files.*"))?;
    let id = read_string_field(value, &["id"]).unwrap_or_else(|| map_key.to_string());
    let file_name = read_string_field(value, &["fileName", "file_name"])
        .ok_or(PrepareUploadParseError::MissingField("files.*.fileName"))?;
    let size = read_u64_field(value, &["size"])
        .ok_or(PrepareUploadParseError::MissingField("files.*.size"))?;
    let file_type = read_string_field(value, &["fileType", "file_type"])
        .ok_or(PrepareUploadParseError::MissingField("files.*.fileType"))?;
    let sha256 = read_string_field(value, &["sha256", "hash"]);
    let preview = read_string_field(value, &["preview"]);
    let metadata = object.get("metadata").and_then(parse_file_metadata);

    Ok(FileDto {
        id,
        file_name,
        size,
        file_type,
        sha256,
        preview,
        metadata,
    })
}

fn parse_file_metadata(value: &serde_json::Value) -> Option<localsend::model::transfer::FileMetadata> {
    let _object = value.as_object()?;
    Some(localsend::model::transfer::FileMetadata {
        modified: read_string_field(value, &["modified", "lastModified"]),
        accessed: read_string_field(value, &["accessed", "lastAccessed"]),
    })
}

fn is_text_file_type(file_type: &str) -> bool {
    let lower = file_type.to_ascii_lowercase();
    lower == "text" || lower.starts_with("text/")
}

fn extract_embedded_text_message(files: &HashMap<String, FileDto>) -> Option<&str> {
    if files.len() != 1 {
        return None;
    }
    let file = files.values().next()?;
    if !is_text_file_type(&file.file_type) {
        return None;
    }
    file.preview.as_deref()
}

async fn handle_embedded_text_message(
    state: ServerState,
    sender_alias: &str,
    text: &str,
) -> Response {
    let destination_dir = state.config.receive_dir.clone();
    let desired_name = text_file_name();
    let target_path = unique_path(&destination_dir, &desired_name);

    if let Err(error) = tokio::fs::create_dir_all(&destination_dir).await {
        tracing::debug!("Failed to create receive directory: {error}");
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to create receive directory",
        );
    }

    match tokio::fs::write(&target_path, text.as_bytes()).await {
        Ok(()) => {
            let bytes = text.len() as u64;
            state.notify_human(format!(
                "Incoming text from {} (saved to {})",
                sender_alias,
                target_path.display()
            ));
            if state.output == OutputMode::Json {
                state.emit_json_event(ReceiveEventJson::TransferStarted {
                    sender_alias: sender_alias.to_string(),
                    file_count: 1,
                });
                state.emit_json_event(ReceiveEventJson::FileSaved {
                    path: target_path.display().to_string(),
                    file_name: desired_name,
                    size: bytes,
                });
                state.emit_json_event(ReceiveEventJson::TransferComplete);
            }
            tokio::time::sleep(TEXT_MESSAGE_RESPONSE_DELAY).await;
            if state.stop_after_transfer {
                if let Some(tx) = &state.stop_tx {
                    let _ = tx.send(());
                }
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Err(error) => {
            tracing::debug!("Failed to save embedded text message: {error}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Failed to save text message")
        }
    }
}

async fn upload_v1_handler(
    State(state): State<ServerState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(params): Query<HashMap<String, String>>,
    body: Body,
) -> Response {
    upload_handler(state, addr, params, body, false).await
}

async fn upload_v2_handler(
    State(state): State<ServerState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(params): Query<HashMap<String, String>>,
    body: Body,
) -> Response {
    upload_handler(state, addr, params, body, true).await
}

async fn upload_handler(
    state: ServerState,
    addr: SocketAddr,
    params: HashMap<String, String>,
    body: Body,
    require_session_id: bool,
) -> Response {
    let session_id = params.get("sessionId").cloned();
    if require_session_id && session_id.is_none() {
        return error_response(StatusCode::BAD_REQUEST, "Missing parameters");
    }
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

    if let Some(expected_session_id) = &session_id {
        if session.session_id != *expected_session_id {
            return error_response(StatusCode::CONFLICT, "Wrong session id");
        }
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
            state.notify_human(format!(
                "Saved {} ({} bytes)",
                target_path.display(),
                bytes
            ));
            if state.output == OutputMode::Json {
                state.emit_json_event(ReceiveEventJson::FileSaved {
                    path: target_path.display().to_string(),
                    file_name: desired_name,
                    size: bytes,
                });
            }
        }
        Err(e) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
        }
    }

    if session.files.values().all(|f| f.path.is_some()) {
        session.status = SessionStatus::Finished;
        state.notify_human("Transfer complete.");
        if state.output == OutputMode::Json {
            state.emit_json_event(ReceiveEventJson::TransferComplete);
        }
        guard.session = None;
        if state.stop_after_transfer {
            if let Some(tx) = &state.stop_tx {
                let _ = tx.send(());
            }
        }
    }

    StatusCode::OK.into_response()
}

async fn cancel_handler(State(state): State<ServerState>, Query(params): Query<HashMap<String, String>>) -> StatusCode {
    let requested = params.get("sessionId").cloned();
    let mut guard = state.inner.lock().await;
    if let Some(session) = &guard.session {
        if requested.as_ref() == Some(&session.session_id) || requested.is_none() {
            state.notify_human("Transfer cancelled by sender.");
            if state.output == OutputMode::Json {
                state.emit_json_event(ReceiveEventJson::TransferCancelled);
            }
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

fn info_response_json(state: &ServerState) -> InfoResponseJson {
    InfoResponseJson {
        alias: state.config.alias.clone(),
        version: crate::config::PROTOCOL_VERSION.to_string(),
        device_model: Some(std::env::consts::OS.to_string()),
        device_type: Some("headless".to_string()),
        fingerprint: state.identity.fingerprint.clone(),
        download: false,
    }
}

fn is_self_fingerprint(state: &ServerState, sender_fingerprint: Option<&String>) -> bool {
    sender_fingerprint.is_some_and(|fp| fp == &state.identity.fingerprint)
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

fn check_pin(state: &ServerState, query: &HashMap<String, String>, headers: &HeaderMap) -> Option<Response> {
    let Some(expected) = state.receive_pin.as_ref().filter(|pin| !pin.is_empty()) else {
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
        Some(value) if value == *expected => None,
        Some(_) => Some(error_response(StatusCode::UNAUTHORIZED, "Invalid PIN")),
        None => Some(error_response(StatusCode::UNAUTHORIZED, "PIN required")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_official_style_prepare_upload_with_token_field() {
        let json = r#"{
            "info": {
                "alias": "Cute Apple",
                "version": "2.1",
                "deviceModel": "iPhone",
                "deviceType": "mobile",
                "token": "sender-fingerprint",
                "port": 53317,
                "protocol": "https",
                "hasWebInterface": false
            },
            "files": {
                "abc": {
                    "id": "abc",
                    "fileName": "hello.txt",
                    "size": 5,
                    "fileType": "text/plain",
                    "preview": "hello"
                }
            }
        }"#;
        let parsed = parse_prepare_upload_request(json).expect("parse");
        assert_eq!(parsed.sender_alias, "Cute Apple");
        assert_eq!(parsed.files.len(), 1);
        assert_eq!(
            extract_embedded_text_message(&parsed.files),
            Some("hello")
        );
    }

    #[test]
    fn parses_prepare_upload_with_fingerprint_and_hash_fields() {
        let json = r#"{
            "info": {
                "alias": "Sender",
                "version": "2.1",
                "fingerprint": "abc123",
                "port": 53317,
                "protocol": "https",
                "download": false
            },
            "files": {
                "f1": {
                    "id": "f1",
                    "fileName": "photo.png",
                    "size": 1024,
                    "fileType": "image/png",
                    "hash": "deadbeef"
                }
            }
        }"#;
        let parsed = parse_prepare_upload_request(json).expect("parse");
        assert_eq!(
            parsed.files.get("f1").and_then(|f| f.sha256.as_deref()),
            Some("deadbeef")
        );
        assert!(extract_embedded_text_message(&parsed.files).is_none());
    }

    #[test]
    fn parses_prepare_upload_without_file_id_using_map_key() {
        let json = r#"{
            "info": {
                "alias": "Sender",
                "deviceType": "mobile"
            },
            "files": {
                "f1": {
                    "fileName": "photo.png",
                    "size": 1024,
                    "fileType": "image"
                }
            }
        }"#;
        let parsed = parse_prepare_upload_request(json).expect("parse");
        assert_eq!(parsed.files.get("f1").map(|f| f.id.as_str()), Some("f1"));
    }

    #[test]
    fn parses_prepare_upload_with_string_size() {
        let json = r#"{
            "info": { "alias": "Sender" },
            "files": {
                "f1": {
                    "id": "f1",
                    "fileName": "photo.png",
                    "size": "1024",
                    "fileType": "image/jpeg"
                }
            }
        }"#;
        let parsed = parse_prepare_upload_request(json).expect("parse");
        assert_eq!(parsed.files.get("f1").map(|f| f.size), Some(1024));
    }
}
