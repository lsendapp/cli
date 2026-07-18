use std::io::{IsTerminal, stdin};

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncReadExt, stdin as async_stdin};
use uuid::Uuid;

pub const TEXT_FILE_TYPE: &str = "text/plain";

pub fn text_file_name() -> String {
    format!("{}.txt", Uuid::new_v4())
}

pub async fn read_stdin_text() -> Result<Vec<u8>> {
    if stdin().is_terminal() {
        bail!(
            "No stdin data (terminal detected). Pipe text in or use --message \"...\" instead of --text."
        );
    }

    let mut buffer = Vec::new();
    async_stdin()
        .read_to_end(&mut buffer)
        .await
        .context("Failed to read text from stdin")?;
    Ok(buffer)
}

pub fn read_message_text(message: &str) -> Result<Vec<u8>> {
    if message.is_empty() {
        bail!("--message must not be empty");
    }
    Ok(message.as_bytes().to_vec())
}

pub fn read_clipboard_text() -> Result<Vec<u8>> {
    let text = arboard::Clipboard::new()
        .context("Failed to access the system clipboard")?
        .get_text()
        .context("Clipboard does not contain plain text")?;
    if text.is_empty() {
        bail!("Clipboard text is empty");
    }
    Ok(text.into_bytes())
}

pub fn text_preview(data: &[u8]) -> Option<String> {
    if data.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(data).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_file_name_uses_txt_suffix() {
        assert!(text_file_name().ends_with(".txt"));
    }

    #[test]
    fn message_rejects_empty_string() {
        assert!(read_message_text("").is_err());
    }

    #[test]
    fn preview_decodes_utf8() {
        assert_eq!(text_preview(b"hello").as_deref(), Some("hello"));
    }

    #[test]
    fn text_preview_returns_none_for_empty_input() {
        assert_eq!(text_preview(b""), None);
    }

    #[test]
    fn read_message_text_round_trips_bytes() {
        let bytes = read_message_text("hello world").unwrap();
        assert_eq!(bytes, b"hello world".to_vec());
    }

    #[test]
    fn text_file_name_is_uuidv4_with_txt_suffix() {
        let name = text_file_name();
        // Format: "<uuid>.txt" — UUIDv4 is 36 chars, plus ".txt" = 40.
        assert_eq!(name.len(), 40);
        assert!(name.ends_with(".txt"));
        // The UUID part should parse as a valid UUID.
        let uuid_str = name.trim_end_matches(".txt");
        uuid::Uuid::parse_str(uuid_str).expect("valid uuid");
    }
}
