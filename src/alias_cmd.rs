use anyhow::Result;

use crate::alias::{self, resolve_system_locale_id};
use crate::cli::AliasAction;
use crate::config;
use crate::error::CliError;
use crate::output::{
    AliasRegenerateResult, AliasSetResult, AliasShowResult, OutputOptions, error_envelope,
    print_json,
};

pub fn run(action: AliasAction, output: OutputOptions) -> Result<(), i32> {
    let config_dir = config::default_config_dir().map_err(|e| fail("alias", output, e))?;

    match action {
        AliasAction::Show => run_show(&config_dir, output),
        AliasAction::Regenerate { locale } => {
            run_regenerate(&config_dir, locale.as_deref(), output)
        }
        AliasAction::Set { name } => run_set(&config_dir, &name, output),
    }
}

fn run_show(config_dir: &std::path::Path, output: OutputOptions) -> Result<(), i32> {
    let result = alias::show_or_create(config_dir).map_err(|e| fail("alias", output, e))?;
    let locale = resolve_system_locale_id();

    if output.is_json() {
        print_json(&AliasShowResult {
            command: "alias",
            action: "show",
            ok: true,
            alias: result.alias.clone(),
            path: result.path.display().to_string(),
            locale,
            created: result.created,
        });
    } else if output.show_human_progress() {
        println!("Device alias: {}", result.alias);
        println!("Saved to {}", result.path.display());
        if result.created {
            println!("Generated using locale {locale}.");
        }
    } else {
        println!("{}", result.alias);
    }

    Ok(())
}

fn run_regenerate(
    config_dir: &std::path::Path,
    locale: Option<&str>,
    output: OutputOptions,
) -> Result<(), i32> {
    let locale_id = locale
        .map(alias::resolve_locale_tag)
        .unwrap_or_else(resolve_system_locale_id);
    let result = alias::regenerate(config_dir, locale).map_err(|e| fail("alias", output, e))?;

    if output.is_json() {
        print_json(&AliasRegenerateResult {
            command: "alias",
            action: "regenerate",
            ok: true,
            previous: result.previous.clone(),
            alias: result.alias.clone(),
            path: result.path.display().to_string(),
            locale: locale_id,
        });
    } else if output.show_human_progress() {
        if let Some(previous) = &result.previous {
            println!("Previous: {previous}");
        }
        println!("New alias: {}", result.alias);
        println!("Saved to {}", result.path.display());
        println!("Note: Restart receive (if running) so peers see the new device name.");
        println!("The global --alias flag still overrides this alias for a single command.");
    } else {
        println!("{}", result.alias);
    }

    Ok(())
}

fn run_set(config_dir: &std::path::Path, name: &str, output: OutputOptions) -> Result<(), i32> {
    let result = alias::set_persisted(config_dir, name).map_err(|e| {
        if e.to_string().contains("Alias must") {
            fail(
                "alias",
                output,
                CliError::InvalidAlias {
                    reason: e.to_string(),
                },
            )
        } else {
            fail("alias", output, e)
        }
    })?;

    if output.is_json() {
        print_json(&AliasSetResult {
            command: "alias",
            action: "set",
            ok: true,
            previous: result.previous.clone(),
            alias: result.alias.clone(),
            path: result.path.display().to_string(),
        });
    } else if output.show_human_progress() {
        if let Some(previous) = &result.previous {
            println!("Previous: {previous}");
        }
        println!("Device alias: {}", result.alias);
        println!("Saved to {}", result.path.display());
        println!("Note: Restart receive (if running) so peers see the new device name.");
    } else {
        println!("{}", result.alias);
    }

    Ok(())
}

fn fail(command: &'static str, output: OutputOptions, error: impl Into<anyhow::Error>) -> i32 {
    let cli_error = CliError::from_anyhow(error.into());
    let code = cli_error.exit_code();
    match output.is_json() {
        true => print_json(&error_envelope(command, &cli_error)),
        false => {
            eprintln!("Error: {cli_error}");
            if let Some(hint) = cli_error.hint() {
                eprintln!("Hint: {hint}");
            }
        }
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::OutputMode;
    use std::fs;

    fn fresh_dir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lsend-aliascmd-{}-{}", tag, uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn quiet_json() -> OutputOptions {
        OutputOptions::new(true, false)
    }

    #[test]
    fn run_show_creates_alias_and_returns_ok() {
        let dir = fresh_dir("show");
        let result = run_show(&dir, quiet_json());
        assert!(result.is_ok(), "show failed: {:?}", result.err());
        let persisted = fs::read_to_string(dir.join("alias.txt")).unwrap();
        assert!(!persisted.trim().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_show_reuses_existing_alias() {
        let dir = fresh_dir("show-existing");
        fs::write(dir.join("alias.txt"), "Existing Name\n").unwrap();
        let result = run_show(&dir, quiet_json());
        assert!(result.is_ok());
        let persisted = fs::read_to_string(dir.join("alias.txt")).unwrap();
        assert_eq!(persisted.trim(), "Existing Name");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_regenerate_overwrites_alias() {
        let dir = fresh_dir("regen");
        fs::write(dir.join("alias.txt"), "Old Name\n").unwrap();
        let result = run_regenerate(&dir, Some("en"), quiet_json());
        assert!(result.is_ok());
        let persisted = fs::read_to_string(dir.join("alias.txt")).unwrap();
        assert_ne!(persisted.trim(), "Old Name");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_set_persists_alias() {
        let dir = fresh_dir("set");
        let result = run_set(&dir, "My Laptop", quiet_json());
        assert!(result.is_ok());
        let persisted = fs::read_to_string(dir.join("alias.txt")).unwrap();
        assert_eq!(persisted.trim(), "My Laptop");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_set_rejects_empty_with_invalid_alias_code() {
        let dir = fresh_dir("set-empty");
        let result = run_set(&dir, "   ", quiet_json());
        // The function returns Err(invalid_alias exit code) on empty input.
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_set_rejects_too_long() {
        let dir = fresh_dir("set-long");
        let too_long = "a".repeat(256);
        let result = run_set(&dir, &too_long, quiet_json());
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
