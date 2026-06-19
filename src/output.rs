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
    pub command: &'static str,
    pub ok: bool,
    pub code: &'static str,
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

pub fn error_envelope(command: &'static str, err: &CliError) -> ErrorEnvelope {
    ErrorEnvelope {
        command,
        ok: false,
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
pub struct AliasShowResult {
    pub command: &'static str,
    pub action: &'static str,
    pub ok: bool,
    pub alias: String,
    pub path: String,
    pub locale: String,
    pub created: bool,
}

#[derive(Debug, Serialize)]
pub struct AliasRegenerateResult {
    pub command: &'static str,
    pub action: &'static str,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    pub alias: String,
    pub path: String,
    pub locale: String,
}

#[derive(Debug, Serialize)]
pub struct AliasSetResult {
    pub command: &'static str,
    pub action: &'static str,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    pub alias: String,
    pub path: String,
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
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SendKind {
    File {
        files: Vec<SendFileResult>,
    },
    Message {
        text: String,
        size: u64,
        status: &'static str,
    },
}

#[derive(Debug, Serialize)]
pub struct SendResult {
    pub command: &'static str,
    pub ok: bool,
    pub target: DeviceJson,
    pub resolved_via: &'static str,
    #[serde(flatten)]
    pub kind: SendKind,
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
    MessageReceived {
        sender_alias: String,
        text: String,
        size: u64,
    },
    FileSaved {
        path: String,
        file_name: String,
        size: u64,
    },
    TransferComplete,
    TransferFinishedWithErrors,
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
    fn error_envelope_serializes_command_before_ok() {
        let err = CliError::NoFiles;
        let envelope = error_envelope("send", &err);
        let json = serde_json::to_string(&envelope).expect("serialize");
        let command_pos = json.find("\"command\"").expect("command field");
        let ok_pos = json.find("\"ok\"").expect("ok field");
        assert!(command_pos < ok_pos);
        assert!(json.starts_with("{\"command\":"));
    }

    #[test]
    fn no_tui_env_forces_json_mode() {
        // SAFETY: this test runs single-threaded; env var mutation is isolated to this test.
        unsafe {
            std::env::set_var("LSEND_NO_TUI", "1");
        }
        let output = OutputOptions::from_cli(false, false);
        unsafe {
            std::env::remove_var("LSEND_NO_TUI");
        }
        assert!(output.is_json());
    }

    #[test]
    fn message_received_event_serializes_as_snake_case() {
        let event = ReceiveEventJson::MessageReceived {
            sender_alias: "iPhone".to_string(),
            text: "hello".to_string(),
            size: 5,
        };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(json.contains("\"event\":\"message_received\""));
        assert!(json.contains("\"sender_alias\":\"iPhone\""));
        assert!(json.contains("\"text\":\"hello\""));
    }

    #[test]
    fn send_file_result_serializes_with_kind_file() {
        let result = SendResult {
            command: "send",
            ok: true,
            target: DeviceJson {
                alias: "Peer".to_string(),
                ip: "192.168.1.10".to_string(),
                port: 53317,
                fingerprint: String::new(),
                https: true,
                version: "2.1".to_string(),
                device_type: None,
                device_model: None,
            },
            resolved_via: "ip",
            kind: SendKind::File {
                files: vec![SendFileResult {
                    name: "file.pdf".to_string(),
                    path: "/tmp/file.pdf".to_string(),
                    size: 1024,
                    status: "finished",
                }],
            },
        };
        let json = serde_json::to_string(&result).expect("serialize");
        assert!(json.contains("\"kind\":\"file\""));
        assert!(json.contains("\"status\":\"finished\""));
        assert!(!json.contains("\"kind\":\"message\""));
    }

    #[test]
    fn send_message_result_serializes_with_kind_message() {
        let result = SendResult {
            command: "send",
            ok: true,
            target: DeviceJson {
                alias: "Peer".to_string(),
                ip: "192.168.1.10".to_string(),
                port: 53317,
                fingerprint: String::new(),
                https: true,
                version: "2.1".to_string(),
                device_type: None,
                device_model: None,
            },
            resolved_via: "ip",
            kind: SendKind::Message {
                text: "hello".to_string(),
                size: 5,
                status: "finished",
            },
        };
        let json = serde_json::to_string(&result).expect("serialize");
        assert!(json.contains("\"kind\":\"message\""));
        assert!(json.contains("\"text\":\"hello\""));
        assert!(!json.contains("\"files\""));
    }
}
