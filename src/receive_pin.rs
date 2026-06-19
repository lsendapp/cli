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
        let dir = std::env::temp_dir().join(format!("lsend-receive-pin-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();

        let pin = resolve(&dir, Some("654321".to_string())).unwrap();
        assert_eq!(pin.as_deref(), Some("654321"));
        assert_eq!(load_persisted(&dir).unwrap().as_deref(), Some("654321"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn loads_persisted_when_no_cli_pin() {
        let dir = std::env::temp_dir().join(format!("lsend-receive-pin-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        persist(&dir, "111222").unwrap();

        let pin = resolve(&dir, None).unwrap();
        assert_eq!(pin.as_deref(), Some("111222"));

        let _ = fs::remove_dir_all(dir);
    }

    fn fresh_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lsend-rp-{}-{}", tag, uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn empty_cli_pin_is_rejected() {
        let dir = fresh_dir("empty");
        let err = resolve(&dir, Some("".to_string())).expect_err("empty PIN");
        assert!(err.to_string().contains("must not be empty"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn whitespace_cli_pin_is_trimmed() {
        let dir = fresh_dir("trim");
        let pin = resolve(&dir, Some("  123456  ".to_string())).unwrap();
        assert_eq!(pin.as_deref(), Some("123456"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_persisted_file_treated_as_absent() {
        let dir = fresh_dir("emptyfile");
        persist(&dir, "").unwrap();
        // Falls through to env. Ensure no env is set for this test, then we
        // expect None.
        let prior = std::env::var("LSEND_RECEIVE_PIN").ok();
        // SAFETY: serial test
        unsafe { std::env::remove_var("LSEND_RECEIVE_PIN") };
        let pin = resolve(&dir, None).unwrap();
        assert_eq!(pin, None);
        if let Some(v) = prior {
            unsafe { std::env::set_var("LSEND_RECEIVE_PIN", v) };
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_fallback_when_no_file_and_no_cli() {
        let dir = fresh_dir("env");
        let prior = std::env::var("LSEND_RECEIVE_PIN").ok();
        unsafe { std::env::set_var("LSEND_RECEIVE_PIN", "987654") };
        let pin = resolve(&dir, None).unwrap();
        assert_eq!(pin.as_deref(), Some("987654"));
        if let Some(v) = prior {
            unsafe { std::env::set_var("LSEND_RECEIVE_PIN", v) };
        } else {
            unsafe { std::env::remove_var("LSEND_RECEIVE_PIN") };
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_empty_value_ignored() {
        let dir = fresh_dir("envempty");
        let prior = std::env::var("LSEND_RECEIVE_PIN").ok();
        unsafe { std::env::set_var("LSEND_RECEIVE_PIN", "   ") };
        let pin = resolve(&dir, None).unwrap();
        assert_eq!(pin, None);
        if let Some(v) = prior {
            unsafe { std::env::set_var("LSEND_RECEIVE_PIN", v) };
        } else {
            unsafe { std::env::remove_var("LSEND_RECEIVE_PIN") };
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cli_pin_overrides_persisted() {
        let dir = fresh_dir("override");
        persist(&dir, "old-pin").unwrap();
        let pin = resolve(&dir, Some("new-pin".to_string())).unwrap();
        assert_eq!(pin.as_deref(), Some("new-pin"));
        // The CLI value is persisted and replaces the old one.
        assert_eq!(load_persisted(&dir).unwrap().as_deref(), Some("new-pin"));
        let _ = fs::remove_dir_all(&dir);
    }
}
