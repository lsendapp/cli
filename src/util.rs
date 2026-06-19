use localsend::model::discovery::DeviceType;

pub fn fingerprint_from_cert_pem(cert_pem: &str) -> anyhow::Result<String> {
    let der = pem_to_der(cert_pem)?;
    let hash = localsend::crypto::hash::sha256(&der);
    Ok(hex::encode(hash))
}

pub fn random_fingerprint() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// Official clients send lowercase values (`desktop`); the core crate expects `DESKTOP`.
pub fn parse_device_type(value: &str) -> Option<DeviceType> {
    match value.to_ascii_lowercase().as_str() {
        "mobile" => Some(DeviceType::Mobile),
        "desktop" => Some(DeviceType::Desktop),
        "web" => Some(DeviceType::Web),
        "headless" => Some(DeviceType::Headless),
        "server" => Some(DeviceType::Server),
        _ => None,
    }
}

/// Map Rust's `std::env::consts::OS` to its conventional display name.
pub fn os_display_name() -> String {
    os_display_name_for(std::env::consts::OS)
}

fn os_display_name_for(os: &str) -> String {
    match os {
        "macos" => "macOS".to_string(),
        "ios" => "iOS".to_string(),
        "tvos" => "tvOS".to_string(),
        "watchos" => "watchOS".to_string(),
        "visionos" => "visionOS".to_string(),
        "linux" => "Linux".to_string(),
        "windows" => "Windows".to_string(),
        "android" => "Android".to_string(),
        "freebsd" => "FreeBSD".to_string(),
        "openbsd" => "OpenBSD".to_string(),
        "netbsd" => "NetBSD".to_string(),
        "dragonfly" => "DragonFly BSD".to_string(),
        "solaris" => "Solaris".to_string(),
        "illumos" => "illumos".to_string(),
        "haiku" => "Haiku".to_string(),
        "fuchsia" => "Fuchsia".to_string(),
        "redox" => "Redox".to_string(),
        "hurd" => "GNU Hurd".to_string(),
        "aix" => "AIX".to_string(),
        "apple" => "Apple".to_string(),
        "espidf" => "ESP-IDF".to_string(),
        "vxworks" => "VxWorks".to_string(),
        "wasm32" => "WebAssembly".to_string(),
        "cloudabi" => "CloudABI".to_string(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        }
    }
}

fn pem_to_der(pem: &str) -> anyhow::Result<Vec<u8>> {
    let content: String = pem
        .replace("\r\n", "\n")
        .lines()
        .filter(|line| !line.starts_with("---"))
        .collect();
    Ok(base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        content,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_display_name_uses_brand_casing() {
        assert_eq!(os_display_name_for("macos"), "macOS");
        assert_eq!(os_display_name_for("linux"), "Linux");
        assert_eq!(os_display_name_for("windows"), "Windows");
        assert_eq!(os_display_name_for("ios"), "iOS");
        assert_eq!(os_display_name_for("tvos"), "tvOS");
        assert_eq!(os_display_name_for("watchos"), "watchOS");
        assert_eq!(os_display_name_for("visionos"), "visionOS");
        assert_eq!(os_display_name_for("android"), "Android");
        assert_eq!(os_display_name_for("freebsd"), "FreeBSD");
        assert_eq!(os_display_name_for("openbsd"), "OpenBSD");
        assert_eq!(os_display_name_for("netbsd"), "NetBSD");
        assert_eq!(os_display_name_for("dragonfly"), "DragonFly BSD");
        assert_eq!(os_display_name_for("solaris"), "Solaris");
        assert_eq!(os_display_name_for("illumos"), "illumos");
        assert_eq!(os_display_name_for("haiku"), "Haiku");
        assert_eq!(os_display_name_for("fuchsia"), "Fuchsia");
        assert_eq!(os_display_name_for("redox"), "Redox");
        assert_eq!(os_display_name_for("hurd"), "GNU Hurd");
        assert_eq!(os_display_name_for("aix"), "AIX");
        assert_eq!(os_display_name_for("apple"), "Apple");
        assert_eq!(os_display_name_for("espidf"), "ESP-IDF");
        assert_eq!(os_display_name_for("vxworks"), "VxWorks");
        assert_eq!(os_display_name_for("wasm32"), "WebAssembly");
        assert_eq!(os_display_name_for("cloudabi"), "CloudABI");
        // Unknown OS values get title-cased as a fallback.
        assert_eq!(os_display_name_for("plan9"), "Plan9");
    }

    #[test]
    fn os_display_name_empty_string_is_empty() {
        assert_eq!(os_display_name_for(""), "");
    }

    #[test]
    fn parse_device_type_accepts_lowercase() {
        use localsend::model::discovery::DeviceType;
        assert_eq!(parse_device_type("mobile"), Some(DeviceType::Mobile));
        assert_eq!(parse_device_type("desktop"), Some(DeviceType::Desktop));
        assert_eq!(parse_device_type("web"), Some(DeviceType::Web));
        assert_eq!(parse_device_type("headless"), Some(DeviceType::Headless));
        assert_eq!(parse_device_type("server"), Some(DeviceType::Server));
    }

    #[test]
    fn parse_device_type_is_case_insensitive() {
        use localsend::model::discovery::DeviceType;
        assert_eq!(parse_device_type("MOBILE"), Some(DeviceType::Mobile));
        assert_eq!(parse_device_type("Desktop"), Some(DeviceType::Desktop));
    }

    #[test]
    fn parse_device_type_rejects_unknown() {
        assert_eq!(parse_device_type(""), None);
        assert_eq!(parse_device_type("smartwatch"), None);
        assert_eq!(parse_device_type("unknown"), None);
    }

    #[test]
    fn random_fingerprint_is_uuidv4_no_hyphens() {
        let f = random_fingerprint();
        assert_eq!(f.len(), 32);
        assert!(f.chars().all(|c| c.is_ascii_hexdigit()));
        // No hyphens (stripped UUID)
        assert!(!f.contains('-'));
    }

    #[test]
    fn random_fingerprint_is_unique_across_calls() {
        let a = random_fingerprint();
        let b = random_fingerprint();
        assert_ne!(a, b);
    }

    #[test]
    fn fingerprint_from_cert_pem_is_stable_hex_sha256() {
        // Build a minimal self-signed cert via rcgen and verify the helper
        // returns 64 hex chars (SHA-256 of the DER bytes).
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let pem = cert.cert.pem();
        let fp = fingerprint_from_cert_pem(&pem).unwrap();
        assert_eq!(fp.len(), 64);
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
