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
