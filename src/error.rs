use std::fmt;

#[derive(Debug, Clone)]
pub enum CliError {
    PortInUse { port: u16 },
    TargetNotFound { target: String },
    NoFiles,
    Other(String),
}

impl CliError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::PortInUse { .. } => "port_in_use",
            Self::TargetNotFound { .. } => "target_not_found",
            Self::NoFiles => "no_files",
            Self::Other(_) => "error",
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            Self::PortInUse { .. } => 3,
            Self::TargetNotFound { .. } | Self::NoFiles => 2,
            Self::Other(_) => 1,
        }
    }

    pub fn hint(&self) -> Option<String> {
        match self {
            Self::PortInUse { port } => Some(format!(
                "Close the official LocalSend app or another localsend instance, or pass --port with a free port (e.g. --port {}).",
                port.saturating_add(1)
            )),
            Self::TargetNotFound { .. } => Some(
                "Run `lsend scan --json` first and use the device IP, or pass an IP address directly."
                    .to_string(),
            ),
            Self::NoFiles => Some(
                "Pass at least one existing file or directory path to send.".to_string(),
            ),
            Self::Other(_) => None,
        }
    }

    pub fn from_anyhow(err: anyhow::Error) -> Self {
        if let Some(cli_error) = err.downcast_ref::<CliError>() {
            return cli_error.clone();
        }

        let message = err.to_string();
        if message.contains("Address already in use")
            || message.contains("discovery HTTP server on port")
            || (message.contains("bind") && message.contains("53317"))
            || message.contains("already in use")
        {
            let port = parse_port_from_message(&message).unwrap_or(crate::config::DEFAULT_PORT);
            return Self::PortInUse { port };
        }
        if message.contains("No device found with alias") {
            if let Some(target) = message
                .strip_prefix("No device found with alias \"")
                .and_then(|s| s.strip_suffix('"'))
            {
                return Self::TargetNotFound {
                    target: target.to_string(),
                };
            }
            return Self::TargetNotFound {
                target: message,
            };
        }
        if message == "No files to send" {
            return Self::NoFiles;
        }
        Self::Other(message)
    }
}

fn parse_port_from_message(message: &str) -> Option<u16> {
    message
        .split_whitespace()
        .find_map(|token| token.trim_matches(|c: char| !c.is_ascii_digit()).parse().ok())
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PortInUse { port } => write!(
                f,
                "Port {port} is already in use. Close the official app or pass --port."
            ),
            Self::TargetNotFound { target } => write!(
                f,
                "No device found with alias \"{target}\". Run `lsend scan --json` first or pass an IP address."
            ),
            Self::NoFiles => write!(f, "No files to send"),
            Self::Other(message) => write!(f, "{message}"),
        }
    }
}

impl From<CliError> for anyhow::Error {
    fn from(value: CliError) -> Self {
        Self::msg(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_port_in_use() {
        let err = CliError::from_anyhow(anyhow::anyhow!("Address already in use (os error 48)"));
        assert_eq!(err.code(), "port_in_use");
        assert_eq!(err.exit_code(), 3);
        assert!(err.hint().is_some());
    }

    #[test]
    fn structured_port_in_use_carries_port() {
        let err = CliError::PortInUse { port: 53317 };
        assert!(err.to_string().contains("53317"));
        assert!(err.hint().unwrap().contains("53318"));
    }
}
