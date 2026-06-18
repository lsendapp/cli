use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use indicatif::{ProgressBar, ProgressStyle};
use localsend::http::client::LsHttpClient;
use localsend::http::dto::ProtocolType;
use localsend::http::dto_v2::{PrepareUploadRequestDtoV2, ProtocolTypeV2, RegisterDtoV2};
use localsend::model::transfer::FileDto;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, BufReader};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::config::AppConfig;
use crate::discovery::resolve_target;
use crate::error::CliError;
use crate::identity::Identity;
use crate::network::build_http_client;
use crate::output::{DeviceJson, OutputOptions, SendFileResult, SendKind, SendResult, print_json};
use crate::text_send::{
    TEXT_FILE_TYPE, read_clipboard_text, read_message_text, read_stdin_text, text_file_name,
    text_preview,
};

const CHUNK_SIZE: usize = 512 * 1024;

pub async fn send_files(
    config: &AppConfig,
    identity: &Identity,
    target: &str,
    paths: &[String],
    text_stdin: bool,
    message: Option<&str>,
    clipboard: bool,
    pin: Option<&str>,
    no_scan: bool,
    output: OutputOptions,
) -> Result<()> {
    let (device, resolved_via) = resolve_target(target, config, identity, !no_scan).await?;
    let is_message = is_message_send(text_stdin, message.is_some(), clipboard);
    let files = collect_inputs(paths, text_stdin, message, clipboard).await?;
    if files.is_empty() {
        return Err(CliError::NoFiles.into());
    }

    let client = build_http_client(identity, config.https)?;
    let protocol = if device.https {
        ProtocolType::Https
    } else {
        ProtocolType::Http
    };

    let register = RegisterDtoV2 {
        alias: config.alias.clone(),
        version: crate::config::PROTOCOL_VERSION.to_string(),
        device_model: Some(crate::util::os_display_name()),
        device_type: Some(localsend::model::discovery::DeviceType::Headless),
        fingerprint: identity.fingerprint.clone(),
        port: config.port,
        protocol: if config.https {
            ProtocolTypeV2::Https
        } else {
            ProtocolTypeV2::Http
        },
        download: false,
    };

    let include_text_preview = files.len() == 1 && files[0].file_type == TEXT_FILE_TYPE;
    let file_map: HashMap<String, FileDto> = files
        .iter()
        .map(|f| {
            let preview = if include_text_preview {
                f.in_memory_preview()
            } else {
                None
            };
            (
                f.id.clone(),
                FileDto {
                    id: f.id.clone(),
                    file_name: f.file_name.clone(),
                    size: f.size,
                    file_type: f.file_type.clone(),
                    sha256: None,
                    preview,
                    metadata: None,
                },
            )
        })
        .collect();

    let prepare_payload = PrepareUploadRequestDtoV2 {
        info: register,
        files: file_map,
    };

    if output.show_human_progress() {
        println!(
            "Sending {} file(s) to {} ({})",
            files.len(),
            device.alias,
            device.ip
        );
    }

    let prepare_result = match &client {
        LsHttpClient::V2(c) => {
            c.prepare_upload(
                protocol.clone(),
                &device.ip,
                device.port,
                None,
                prepare_payload,
                pin,
            )
            .await?
        }
        LsHttpClient::V3(_) => bail!("v3 client is not supported by this CLI"),
    };

    let mut results = Vec::with_capacity(files.len());

    if prepare_result.status_code == 204 {
        if output.show_human_progress() {
            println!("Message delivered.");
        } else if output.is_json() {
            print_json(&build_send_result(
                &device,
                resolved_via,
                is_message,
                &files,
                vec![],
                "finished",
            ));
        }
        return Ok(());
    }

    let response = prepare_result
        .response
        .context("Missing prepare-upload response body")?;

    for local in &files {
        let Some(token) = response.files.get(&local.id) else {
            if output.show_human_progress() {
                println!("Skipped (not accepted): {}", local.file_name);
            }
            results.push(SendFileResult {
                name: local.file_name.clone(),
                path: local.display_source(),
                size: local.size,
                status: "skipped",
            });
            continue;
        };

        upload_file(
            &client,
            &protocol,
            &device.ip,
            device.port,
            &response.session_id,
            local,
            token,
            output,
        )
        .await
        .map_err(|e| anyhow::anyhow!("Upload failed for {}: {e}", local.file_name))?;

        results.push(SendFileResult {
            name: local.file_name.clone(),
            path: local.display_source(),
            size: local.size,
            status: "finished",
        });
    }

    if output.is_json() {
        let message_status = if results.iter().all(|result| result.status == "finished") {
            "finished"
        } else {
            "skipped"
        };
        print_json(&build_send_result(
            &device,
            resolved_via,
            is_message,
            &files,
            results,
            message_status,
        ));
    } else if output.show_human_progress() {
        println!("Done.");
    }

    Ok(())
}

async fn upload_file(
    client: &LsHttpClient,
    protocol: &ProtocolType,
    ip: &str,
    port: u16,
    session_id: &str,
    local: &LocalFile,
    token: &str,
    output: OutputOptions,
) -> Result<()> {
    let progress = output.show_human_progress().then(|| {
        let pb = ProgressBar::new(local.size);
        pb.set_style(
            ProgressStyle::with_template("{msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes}")
                .unwrap()
                .progress_chars("=>-"),
        );
        pb.set_message(local.file_name.clone());
        pb
    });
    let progress_for_reader = progress.clone();

    let (tx, rx) = mpsc::channel(4);
    let source = local.source.clone();
    let reader_task = tokio::spawn(async move {
        match source {
            FileSource::Path(path) => {
                let file = File::open(&path).await?;
                let mut reader = BufReader::new(file);
                let mut buffer = vec![0u8; CHUNK_SIZE];
                loop {
                    let n = reader.read(&mut buffer).await?;
                    if n == 0 {
                        break;
                    }
                    if tx.send(buffer[..n].to_vec()).await.is_err() {
                        break;
                    }
                    if let Some(pb) = &progress_for_reader {
                        pb.inc(n as u64);
                    }
                }
            }
            FileSource::Memory { data, .. } => {
                for chunk in data.chunks(CHUNK_SIZE) {
                    if tx.send(chunk.to_vec()).await.is_err() {
                        break;
                    }
                    if let Some(pb) = &progress_for_reader {
                        pb.inc(chunk.len() as u64);
                    }
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    });

    let upload_result = match client {
        LsHttpClient::V2(c) => {
            c.upload(
                protocol.clone(),
                ip,
                port,
                None,
                session_id,
                &local.id,
                token,
                rx,
            )
            .await
        }
        LsHttpClient::V3(_) => unreachable!(),
    };

    reader_task.await??;
    if let Some(pb) = progress {
        pb.finish_and_clear();
    }
    upload_result.map_err(|e| anyhow::anyhow!("{e}"))
}

#[derive(Clone)]
struct LocalFile {
    id: String,
    file_name: String,
    source: FileSource,
    size: u64,
    file_type: String,
}

#[derive(Clone)]
enum FileSource {
    Path(PathBuf),
    Memory { data: Vec<u8>, label: String },
}

impl LocalFile {
    fn display_source(&self) -> String {
        match &self.source {
            FileSource::Path(path) => path.display().to_string(),
            FileSource::Memory { label, .. } => label.clone(),
        }
    }

    fn in_memory_preview(&self) -> Option<String> {
        match &self.source {
            FileSource::Memory { data, .. } => text_preview(data),
            FileSource::Path(_) => None,
        }
    }

    fn message_text(&self) -> String {
        self.in_memory_preview().unwrap_or_default()
    }
}

fn is_message_send(text_stdin: bool, has_message: bool, clipboard: bool) -> bool {
    text_stdin || has_message || clipboard
}

fn build_send_result(
    device: &crate::discovery::DiscoveredDevice,
    resolved_via: &'static str,
    is_message: bool,
    locals: &[LocalFile],
    file_results: Vec<SendFileResult>,
    message_status: &'static str,
) -> SendResult {
    let kind = if is_message {
        let local = locals
            .first()
            .expect("message send must include exactly one in-memory payload");
        SendKind::Message {
            text: local.message_text(),
            size: local.size,
            status: message_status,
        }
    } else {
        SendKind::File {
            files: file_results,
        }
    };

    SendResult {
        command: "send",
        ok: true,
        target: DeviceJson::from(device),
        resolved_via,
        kind,
    }
}

async fn collect_inputs(
    paths: &[String],
    text_stdin: bool,
    message: Option<&str>,
    clipboard: bool,
) -> Result<Vec<LocalFile>> {
    let text_mode = text_stdin || message.is_some() || clipboard;
    if text_mode && !paths.is_empty() {
        bail!("Cannot combine file paths with --text, --message, or --clipboard");
    }

    let mut files = Vec::new();

    if text_stdin {
        let data = read_stdin_text().await?;
        files.push(local_file_from_bytes(data, "stdin")?);
    } else if let Some(message) = message {
        let data = read_message_text(message)?;
        files.push(local_file_from_bytes(data, "inline")?);
    } else if clipboard {
        let data = read_clipboard_text()?;
        files.push(local_file_from_bytes(data, "clipboard")?);
    } else {
        files = collect_files(paths).await?;
    }

    Ok(files)
}

fn local_file_from_bytes(data: Vec<u8>, label: &str) -> Result<LocalFile> {
    let size = data.len() as u64;
    Ok(LocalFile {
        id: Uuid::new_v4().to_string(),
        file_name: text_file_name(),
        source: FileSource::Memory {
            data,
            label: label.to_string(),
        },
        size,
        file_type: TEXT_FILE_TYPE.to_string(),
    })
}

async fn collect_files(paths: &[String]) -> Result<Vec<LocalFile>> {
    let mut files = Vec::new();
    for path_str in paths {
        let path = PathBuf::from(path_str);
        if path.is_dir() {
            collect_dir(&path, &path, &mut files).await?;
        } else if path.is_file() {
            files.push(local_file_from_path(&path, &path.file_name().unwrap().to_string_lossy())?);
        } else {
            bail!("Path not found: {}", path.display());
        }
    }
    Ok(files)
}

async fn collect_dir(base: &Path, dir: &Path, out: &mut Vec<LocalFile>) -> Result<()> {
    let mut entries = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_dir() {
            Box::pin(collect_dir(base, &path, out)).await?;
        } else if path.is_file() {
            let relative = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push(local_file_from_path(&path, &relative)?);
        }
    }
    Ok(())
}

fn local_file_from_path(path: &Path, file_name: &str) -> Result<LocalFile> {
    let metadata = std::fs::metadata(path)?;
    let mime = mime_guess::from_path(path)
        .first_or_octet_stream()
        .essence_str()
        .to_string();
    Ok(LocalFile {
        id: Uuid::new_v4().to_string(),
        file_name: file_name.to_string(),
        source: FileSource::Path(path.to_path_buf()),
        size: metadata.len(),
        file_type: mime,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_mixed_paths_and_text_modes() {
        let result = collect_inputs(&["file.txt".to_string()], true, None, false).await;
        match result {
            Err(err) => assert!(err.to_string().contains("Cannot combine")),
            Ok(_) => panic!("expected error when mixing paths with --text"),
        }
    }

    #[test]
    fn text_file_uses_plain_mime_and_txt_name() {
        let file = local_file_from_bytes(b"hello".to_vec(), "stdin").unwrap();
        assert!(file.file_name.ends_with(".txt"));
        assert_eq!(file.file_type, "text/plain");
        assert_eq!(file.in_memory_preview().as_deref(), Some("hello"));
    }
}
