use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use rand::seq::IndexedRandom;
use serde::Deserialize;

const ALIAS_FILE: &str = "alias.txt";
const LOCALE_DATA: &str = include_str!("../data/alias_locales.json");

#[derive(Debug, Clone, Deserialize)]
struct AliasLocaleData {
    adjectives: Vec<String>,
    fruits: Vec<String>,
    combination: String,
}

static LOCALE_TABLE: OnceLock<HashMap<String, AliasLocaleData>> = OnceLock::new();

fn locale_table() -> &'static HashMap<String, AliasLocaleData> {
    LOCALE_TABLE.get_or_init(|| {
        serde_json::from_str(LOCALE_DATA).expect("alias_locales.json must be valid")
    })
}

/// Load persisted alias or generate one using the system locale (official app behavior).
pub fn load_or_create(config_dir: &Path) -> Result<String> {
    let alias_path = config_dir.join(ALIAS_FILE);
    if alias_path.exists() {
        let alias = fs::read_to_string(&alias_path)
            .with_context(|| format!("Failed to read alias from {}", alias_path.display()))?
            .trim()
            .to_string();
        if !alias.is_empty() {
            return Ok(alias);
        }
    }

    let alias = generate_random_alias();
    fs::write(&alias_path, format!("{alias}\n"))
        .with_context(|| format!("Failed to write alias to {}", alias_path.display()))?;
    Ok(alias)
}

/// Generate a random alias using word lists and word order from the active system locale.
pub fn generate_random_alias() -> String {
    let locale_id = resolve_system_locale_id();
    generate_random_alias_for_locale(&locale_id)
}

fn generate_random_alias_for_locale(locale_id: &str) -> String {
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

fn resolve_locale_tag(tag: &str) -> String {
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
        let dir = std::env::temp_dir().join(format!("localsend-cli-alias-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let first = load_or_create(&dir).unwrap();
        let second = load_or_create(&dir).unwrap();
        assert_eq!(first, second);
        let _ = fs::remove_dir_all(dir);
    }
}
