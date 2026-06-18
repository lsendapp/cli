use rand::seq::IndexedRandom;

const ADJECTIVES: &[&str] = &[
    "Adorable", "Brave", "Calm", "Clever", "Cute", "Eager", "Fair", "Fancy", "Gentle", "Happy",
    "Honest", "Jolly", "Kind", "Lively", "Lucky", "Nice", "Proud", "Quick", "Quiet", "Shy",
    "Silly", "Smart", "Super", "Sweet", "Witty",
];

const FRUITS: &[&str] = &[
    "Apple", "Banana", "Blueberry", "Cherry", "Grape", "Kiwi", "Lemon", "Lime", "Mango", "Melon",
    "Orange", "Peach", "Pear", "Plum", "Strawberry",
];

pub fn generate_random_alias() -> String {
    let mut rng = rand::rng();
    let adjective = ADJECTIVES.choose(&mut rng).unwrap_or(&"Happy");
    let fruit = FRUITS.choose(&mut rng).unwrap_or(&"Orange");
    format!("{adjective} {fruit}")
}

pub fn fingerprint_from_cert_pem(cert_pem: &str) -> anyhow::Result<String> {
    let der = pem_to_der(cert_pem)?;
    let hash = localsend::crypto::hash::sha256(&der);
    Ok(hex::encode(hash))
}

pub fn random_fingerprint() -> String {
    uuid::Uuid::new_v4().simple().to_string()
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
