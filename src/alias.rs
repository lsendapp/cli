use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};
use rand::seq::IndexedRandom;
use serde::Deserialize;

const ALIAS_FILE: &str = "alias.txt";
const MAX_ALIAS_LEN: usize = 255;
const LOCALE_DATA: &str = include_str!("../data/alias_locales.json");

#[derive(Debug, Clone, Deserialize)]
struct AliasLocaleData {
    adjectives: Vec<String>,
    fruits: Vec<String>,
    combination: String,
}

static LOCALE_TABLE: OnceLock<HashMap<String, AliasLocaleData>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasChangeResult {
    pub previous: Option<String>,
    pub alias: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasShowResult {
    pub alias: String,
    pub path: PathBuf,
    pub created: bool,
}

fn locale_table() -> &'static HashMap<String, AliasLocaleData> {
    LOCALE_TABLE.get_or_init(|| {
        serde_json::from_str(LOCALE_DATA).expect("alias_locales.json must be valid")
    })
}

pub fn alias_path(config_dir: &Path) -> PathBuf {
    config_dir.join(ALIAS_FILE)
}

pub fn read_persisted(config_dir: &Path) -> Result<Option<String>> {
    let alias_path = alias_path(config_dir);
    if !alias_path.exists() {
        return Ok(None);
    }

    let alias = fs::read_to_string(&alias_path)
        .with_context(|| format!("Failed to read alias from {}", alias_path.display()))?
        .trim()
        .to_string();

    if alias.is_empty() {
        Ok(None)
    } else {
        Ok(Some(alias))
    }
}

pub fn validate_alias(alias: &str) -> Result<String> {
    let alias = alias.trim();
    if alias.is_empty() {
        bail!("Alias must not be empty");
    }
    if alias.len() > MAX_ALIAS_LEN {
        bail!("Alias must be at most {MAX_ALIAS_LEN} characters");
    }
    Ok(alias.to_string())
}

pub fn save(config_dir: &Path, alias: &str) -> Result<()> {
    fs::create_dir_all(config_dir).with_context(|| {
        format!("Failed to create config directory {}", config_dir.display())
    })?;

    let alias = validate_alias(alias)?;
    let path = alias_path(config_dir);
    fs::write(&path, format!("{alias}\n"))
        .with_context(|| format!("Failed to write alias to {}", path.display()))?;
    Ok(())
}

/// Load persisted alias or generate one using the system locale (official app behavior).
pub fn load_or_create(config_dir: &Path) -> Result<String> {
    if let Some(alias) = read_persisted(config_dir)? {
        return Ok(alias);
    }

    let alias = generate_random_alias();
    save(config_dir, &alias)?;
    Ok(alias)
}

pub fn show_or_create(config_dir: &Path) -> Result<AliasShowResult> {
    let path = alias_path(config_dir);
    let created = read_persisted(config_dir)?.is_none();
    let alias = load_or_create(config_dir)?;
    Ok(AliasShowResult {
        alias,
        path,
        created,
    })
}

pub fn regenerate(config_dir: &Path, locale: Option<&str>) -> Result<AliasChangeResult> {
    let previous = read_persisted(config_dir)?;
    let locale_id = locale
        .map(resolve_locale_tag)
        .unwrap_or_else(resolve_system_locale_id);
    let alias = generate_random_alias_for_locale(&locale_id);
    save(config_dir, &alias)?;
    Ok(AliasChangeResult {
        previous,
        alias,
        path: alias_path(config_dir),
    })
}

pub fn set_persisted(config_dir: &Path, alias: &str) -> Result<AliasChangeResult> {
    let previous = read_persisted(config_dir)?;
    save(config_dir, alias)?;
    Ok(AliasChangeResult {
        previous,
        alias: validate_alias(alias)?,
        path: alias_path(config_dir),
    })
}

/// Generate a random alias using word lists and word order from the active system locale.
pub fn generate_random_alias() -> String {
    let locale_id = resolve_system_locale_id();
    generate_random_alias_for_locale(&locale_id)
}

pub fn generate_random_alias_for_locale(locale_id: &str) -> String {
    let table = locale_table();
    let data = table
        .get(locale_id)
        .or_else(|| table.get("en"))
        .expect("en locale must exist in alias_locales.json");

    let mut rng = rand::rng();
    let adjective = data
        .adjectives
        .choose(&mut rng)
        .map(String::as_str)
        .unwrap_or("Adorable");
    let fruit = data
        .fruits
        .choose(&mut rng)
        .map(String::as_str)
        .unwrap_or("Orange");

    combine(&data.combination, adjective, fruit)
}

fn combine(template: &str, adjective: &str, fruit: &str) -> String {
    template
        .replace("{adjective}", adjective)
        .replace("{fruit}", fruit)
}

/// Resolve the system locale to an official LocalSend i18n file id (e.g. `zh-CN`, `pt-BR`).
pub fn resolve_system_locale_id() -> String {
    let tag = sys_locale::get_locale()
        .or_else(read_lang_env)
        .unwrap_or_else(|| "en".to_string());
    resolve_locale_tag(&tag)
}

fn read_lang_env() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|key| {
            std::env::var(key).ok().and_then(|value| {
                let tag = value.split('.').next()?.trim();
                if tag.is_empty() || tag == "C" || tag == "POSIX" {
                    None
                } else {
                    Some(tag.to_string())
                }
            })
        })
}

pub fn resolve_locale_tag(tag: &str) -> String {
    let parsed = parse_locale_tag(tag);
    for candidate in locale_candidates(&parsed) {
        if locale_table().contains_key(&candidate) {
            return candidate;
        }
    }
    "en".to_string()
}

#[derive(Debug, Clone)]
struct ParsedLocale {
    language: String,
    script: Option<String>,
    region: Option<String>,
}

fn parse_locale_tag(tag: &str) -> ParsedLocale {
    let normalized = tag.trim().replace('_', "-");
    let mut parts = normalized
        .split('-')
        .filter(|part| !part.is_empty())
        .map(|part| part.to_string())
        .collect::<Vec<_>>();

    if parts.is_empty() {
        return ParsedLocale {
            language: "en".to_string(),
            script: None,
            region: None,
        };
    }

    let language = parts[0].to_ascii_lowercase();
    parts.remove(0);

    let mut script = None;
    let mut region = None;

    if let Some(part) = parts.first().cloned() {
        if part.len() == 4 && part.chars().all(|c| c.is_ascii_alphabetic()) {
            let script_part = part.to_string();
            parts.remove(0);
            script = Some(script_part);
        }
    }

    if let Some(part) = parts.first().cloned() {
        if part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()) {
            region = Some(part.to_ascii_uppercase());
            parts.remove(0);
        }
    }

    ParsedLocale {
        language,
        script,
        region,
    }
}

fn locale_candidates(parsed: &ParsedLocale) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut push = |value: &str| {
        if !candidates.iter().any(|existing| existing == value) {
            candidates.push(value.to_string());
        }
    };

    if let Some(region) = &parsed.region {
        push(&format!("{}-{}", parsed.language, region));
    }

    if parsed.language == "sr" {
        if parsed
            .script
            .as_ref()
            .is_some_and(|s| s.eq_ignore_ascii_case("cyrl"))
        {
            push("sr-Cyrl");
        }
        push("sr");
    }

    if parsed.language == "fil" || parsed.language == "tl" {
        push("fil-PH");
    }

    if parsed.language == "es" {
        push("es-ES");
    }

    if parsed.language == "pt" {
        if parsed.region.as_deref() == Some("PT") {
            push("pt-PT");
        }
        push("pt-BR");
        push("pt-PT");
    }

    if parsed.language == "zh" {
        if let Some(region) = &parsed.region {
            push(&format!("zh-{region}"));
        }
        push("zh-CN");
        push("zh-TW");
        push("zh-HK");
    }

    if parsed.language == "en" {
        if parsed.region.as_deref() == Some("IN") {
            push("en-IN");
        }
        push("en");
    }

    push(&parsed.language);
    push("en");
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_combination_matches_official_pattern() {
        let alias = generate_random_alias_for_locale("en");
        assert!(!alias.is_empty());
        let data = locale_table().get("en").unwrap();
        let adj = &data.adjectives[0];
        let fruit = &data.fruits[0];
        assert_eq!(combine(&data.combination, adj, fruit), format!("{adj} {fruit}"));
    }

    #[test]
    fn chinese_combination_uses_particle() {
        let data = locale_table().get("zh-CN").unwrap();
        let alias = combine(&data.combination, "可爱", "苹果");
        assert_eq!(alias, "可爱的苹果");
    }

    #[test]
    fn romanian_combination_puts_fruit_first() {
        let data = locale_table().get("ro").unwrap();
        let alias = combine(&data.combination, "Drăguță", "Banana");
        assert_eq!(alias, "Banana Drăguță");
    }

    #[test]
    fn malay_combination_uses_yang() {
        let data = locale_table().get("ms").unwrap();
        let alias = combine(&data.combination, "Comel", "Epal");
        assert_eq!(alias, "Epal yang Comel");
    }

    #[test]
    fn inherits_english_words_for_german_locale() {
        let data = locale_table().get("de").unwrap();
        let en = locale_table().get("en").unwrap();
        assert_eq!(data.adjectives, en.adjectives);
        assert_eq!(data.fruits, en.fruits);
    }

    #[test]
    fn resolves_zh_cn_locale_tag() {
        assert_eq!(resolve_locale_tag("zh-CN"), "zh-CN");
        assert_eq!(resolve_locale_tag("zh_CN.UTF-8"), "zh-CN");
    }

    #[test]
    fn resolves_pt_br_locale_tag() {
        assert_eq!(resolve_locale_tag("pt-BR"), "pt-BR");
    }

    #[test]
    fn resolves_sr_cyrl_locale_tag() {
        assert_eq!(resolve_locale_tag("sr-Cyrl"), "sr-Cyrl");
    }

    #[test]
    fn load_or_create_persists_alias() {
        let dir = std::env::temp_dir().join(format!("lsend-alias-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let first = load_or_create(&dir).unwrap();
        let second = load_or_create(&dir).unwrap();
        assert_eq!(first, second);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn regenerate_overwrites_persisted_alias() {
        let dir = std::env::temp_dir().join(format!("lsend-alias-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        save(&dir, "Old Alias").unwrap();

        let result = regenerate(&dir, Some("en")).unwrap();
        assert_eq!(result.previous.as_deref(), Some("Old Alias"));
        assert_ne!(result.alias, "Old Alias");
        assert_eq!(load_or_create(&dir).unwrap(), result.alias);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn set_persisted_rejects_empty_alias() {
        let dir = std::env::temp_dir().join(format!("lsend-alias-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        assert!(set_persisted(&dir, "   ").is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn show_or_create_generates_when_missing() {
        let dir = std::env::temp_dir().join(format!("lsend-alias-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let result = show_or_create(&dir).unwrap();
        assert!(result.created);
        assert!(!result.alias.is_empty());
        assert!(result.path.exists());
        let _ = fs::remove_dir_all(dir);
    }
}
