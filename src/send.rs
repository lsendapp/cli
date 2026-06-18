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
use crate::identity::Identity;

const CHUNK_SIZE: usize = 512 * 1024;

pub async fn send_files(
    config: &AppConfig,
    identity: &Identity,
    target: &str,
    paths: &[String],
    pin: Option<&str>,
) -> Result<()> {
    let device = resolve_target(target, config, identity).await?;
    let files = collect_files(paths).await?;
    if files.is_empty() {
        bail!("No files to send");
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
        device_model: Some(std::env::consts::OS.to_string()),
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

    let file_map: HashMap<String, FileDto> = files
        .iter()
        .map(|f| {
            (
                f.id.clone(),
                FileDto {
                    id: f.id.clone(),
                    file_name: f.file_name.clone(),
                    size: f.size,
                    file_type: f.file_type.clone(),
                    sha256: None,
                    preview: None,
                    metadata: None,
                },
            )
        })
        .collect();

    let prepare_payload = PrepareUploadRequestDtoV2 {
        info: register,
        files: file_map,
    };

    println!(
        "Sending {} file(s) to {} ({})",
        files.len(),
        device.alias,
        device.ip
    );

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

    if prepare_result.status_code == 204 {
        println!("Receiver accepted with no file transfer needed.");
        return Ok(());
    }

    let response = prepare_result
        .response
        .context("Missing prepare-upload response body")?;

    for local in &files {
        let Some(token) = response.files.get(&local.id) else {
            println!("Skipped (not accepted): {}", local.file_name);
            continue;
        };

        let pb = ProgressBar::new(local.size);
        pb.set_style(
            ProgressStyle::with_template("{msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes}")
                .unwrap()
                .progress_chars("=>-"),
        );
        pb.set_message(local.file_name.clone());
        let pb_reader = pb.clone();

        let (tx, rx) = mpsc::channel(4);
        let path = local.path.clone();
        let reader_task = tokio::spawn(async move {
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
                pb_reader.inc(n as u64);
            }
            Ok::<(), anyhow::Error>(())
        });

        let upload_result = match &client {
            LsHttpClient::V2(c) => {
                c.upload(
                    protocol.clone(),
                    &device.ip,
                    device.port,
                    None,
                    &response.session_id,
                    &local.id,
                    token,
                    rx,
                )
                .await
            }
            LsHttpClient::V3(_) => unreachable!(),
        };

        reader_task.await??;
        pb.finish_and_clear();
        upload_result.map_err(|e| anyhow::anyhow!("Upload failed for {}: {e}", local.file_name))?;
    }

    println!("Done.");
    Ok(())
}

#[derive(Clone)]
struct LocalFile {
    id: String,
    file_name: String,
    path: PathBuf,
    size: u64,
    file_type: String,
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
        path: path.to_path_buf(),
        size: metadata.len(),
        file_type: mime,
    })
}

use crate::network::build_http_client;
