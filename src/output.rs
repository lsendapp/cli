use std::io::{IsTerminal, stdout};

use localsend::model::discovery::DeviceType;
use serde::Serialize;

use crate::discovery::DiscoveredDevice;
use crate::error::CliError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputMode {
    Human,
    Json,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputOptions {
    pub mode: OutputMode,
    pub quiet: bool,
}

impl OutputOptions {
    pub fn from_cli(json_flag: bool, quiet_flag: bool) -> Self {
        let json = json_flag || should_force_json();
        Self::new(json, quiet_flag)
    }

    pub fn new(json: bool, quiet: bool) -> Self {
        Self {
            mode: if json { OutputMode::Json } else { OutputMode::Human },
            quiet: json || quiet,
        }
    }

    pub fn is_json(&self) -> bool {
        self.mode == OutputMode::Json
    }

    pub fn show_human_progress(&self) -> bool {
        self.mode == OutputMode::Human && !self.quiet
    }
}

fn should_force_json() -> bool {
    no_tui_env() || !stdout().is_terminal()
}

fn no_tui_env() -> bool {
    matches!(
        std::env::var("LSEND_NO_TUI")
            .ok()
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("1") | Some("true") | Some("yes")
    )
}

#[derive(Debug, Serialize)]
pub struct DeviceJson {
    pub alias: String,
    pub ip: String,
    pub port: u16,
    pub fingerprint: String,
    pub https: bool,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_model: Option<String>,
}

impl From<&DiscoveredDevice> for DeviceJson {
    fn from(device: &DiscoveredDevice) -> Self {
        Self {
            alias: device.alias.clone(),
            ip: device.ip.clone(),
            port: device.port,
            fingerprint: device.fingerprint.clone(),
            https: device.https,
            version: device.version.clone(),
            device_type: device.device_type.as_ref().map(device_type_label),
            device_model: device.device_model.clone(),
        }
    }
}

fn device_type_label(device_type: &DeviceType) -> String {
    match device_type {
        DeviceType::Mobile => "mobile",
        DeviceType::Desktop => "desktop",
        DeviceType::Web => "web",
        DeviceType::Headless => "headless",
        DeviceType::Server => "server",
    }
    .to_string()
}

#[derive(Debug, Serialize)]
pub struct ErrorEnvelope {
    pub ok: bool,
    pub command: &'static str,
    pub code: &'static str,
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

pub fn error_envelope(command: &'static str, err: &CliError) -> ErrorEnvelope {
    ErrorEnvelope {
        ok: false,
        command,
        code: err.code(),
        error: err.to_string(),
        hint: err.hint(),
    }
}

pub fn print_json<T: Serialize>(value: &T) {
    println!(
        "{}",
        serde_json::to_string(value).expect("JSON serialization must not fail")
    );
}

#[derive(Debug, Serialize)]
pub struct ScanResult {
    pub command: &'static str,
    pub ok: bool,
    pub timeout_ms: u64,
    pub devices: Vec<DeviceJson>,
}

#[derive(Debug, Serialize)]
pub struct SendFileResult {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct SendResult {
    pub command: &'static str,
    pub ok: bool,
    pub target: DeviceJson,
    pub resolved_via: &'static str,
    pub files: Vec<SendFileResult>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ReceiveEventJson {
    Ready {
        alias: String,
        port: u16,
        https: bool,
        receive_dir: String,
    },
    TransferStarted {
        sender_alias: String,
        file_count: usize,
    },
    FileSaved {
        path: String,
        file_name: String,
        size: u64,
    },
    TransferComplete,
    TransferCancelled,
    Shutdown,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CliError;

    #[test]
    fn error_envelope_includes_hint() {
        let err = CliError::PortInUse { port: 53317 };
        let envelope = error_envelope("receive", &err);
        assert_eq!(envelope.code, "port_in_use");
        assert!(envelope.hint.is_some());
    }

    #[test]
    fn no_tui_env_forces_json_mode() {
        std::env::set_var("LSEND_NO_TUI", "1");
        let output = OutputOptions::from_cli(false, false);
        std::env::remove_var("LSEND_NO_TUI");
        assert!(output.is_json());
    }
}
