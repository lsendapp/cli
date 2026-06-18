use std::fmt;

#[derive(Debug, Clone)]
pub enum CliError {
    PortInUse,
    TargetNotFound { target: String },
    NoFiles,
    Other(String),
}

impl CliError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::PortInUse => "port_in_use",
            Self::TargetNotFound { .. } => "target_not_found",
            Self::NoFiles => "no_files",
            Self::Other(_) => "error",
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            Self::PortInUse => 3,
            Self::TargetNotFound { .. } | Self::NoFiles => 2,
            Self::Other(_) => 1,
        }
    }

    pub fn from_anyhow(err: anyhow::Error) -> Self {
        let message = err.to_string();
        if message.contains("Address already in use")
            || message.contains("discovery HTTP server on port")
            || message.contains("bind")
                && message.contains("53317")
        {
            return Self::PortInUse;
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

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PortInUse => write!(
                f,
                "Port 53317 is already in use. Close the official app or pass --port."
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
    }
}
