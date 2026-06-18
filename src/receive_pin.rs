use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

const RECEIVE_PIN_FILE: &str = "receive_pin";

/// Resolve the receive PIN for this session.
///
/// Priority: `--pin` flag (persisted when set) > `receive_pin` file > `LSEND_RECEIVE_PIN` env.
pub fn resolve(config_dir: &Path, cli_pin: Option<String>) -> Result<Option<String>> {
    if let Some(pin) = cli_pin {
        let pin = pin.trim().to_string();
        if pin.is_empty() {
            anyhow::bail!("Receive PIN must not be empty");
        }
        persist(config_dir, &pin)?;
        return Ok(Some(pin));
    }

    if let Some(pin) = load_persisted(config_dir)? {
        return Ok(Some(pin));
    }

    Ok(std::env::var("LSEND_RECEIVE_PIN")
        .ok()
        .map(|pin| pin.trim().to_string())
        .filter(|pin| !pin.is_empty()))
}

fn load_persisted(config_dir: &Path) -> Result<Option<String>> {
    let path = config_dir.join(RECEIVE_PIN_FILE);
    if !path.exists() {
        return Ok(None);
    }

    let pin = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read receive PIN from {}", path.display()))?
        .trim()
        .to_string();

    if pin.is_empty() {
        Ok(None)
    } else {
        Ok(Some(pin))
    }
}

fn persist(config_dir: &Path, pin: &str) -> Result<()> {
    let path = config_dir.join(RECEIVE_PIN_FILE);
    fs::write(&path, format!("{pin}\n"))
        .with_context(|| format!("Failed to write receive PIN to {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_pin_is_persisted() {
        let dir = std::env::temp_dir().join(format!("localsend-receive-pin-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();

        let pin = resolve(&dir, Some("654321".to_string())).unwrap();
        assert_eq!(pin.as_deref(), Some("654321"));
        assert_eq!(load_persisted(&dir).unwrap().as_deref(), Some("654321"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn loads_persisted_when_no_cli_pin() {
        let dir = std::env::temp_dir().join(format!("localsend-receive-pin-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        persist(&dir, "111222").unwrap();

        let pin = resolve(&dir, None).unwrap();
        assert_eq!(pin.as_deref(), Some("111222"));

        let _ = fs::remove_dir_all(dir);
    }
}
