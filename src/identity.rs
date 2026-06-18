use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use rcgen::{generate_simple_self_signed, CertifiedKey};

use crate::util::{fingerprint_from_cert_pem, random_fingerprint};

const CERT_FILE: &str = "cert.pem";
const KEY_FILE: &str = "key.pem";
const FINGERPRINT_FILE: &str = "fingerprint.txt";

#[derive(Clone, Debug)]
pub struct Identity {
    pub cert_pem: String,
    pub key_pem: String,
    pub fingerprint: String,
}

impl Identity {
    pub fn load_or_create(config_dir: &Path, https: bool) -> Result<Self> {
        fs::create_dir_all(config_dir).with_context(|| {
            format!("Failed to create config directory {}", config_dir.display())
        })?;

        let cert_path = config_dir.join(CERT_FILE);
        let key_path = config_dir.join(KEY_FILE);
        let fingerprint_path = config_dir.join(FINGERPRINT_FILE);

        if cert_path.exists() && key_path.exists() && fingerprint_path.exists() {
            let cert_pem = fs::read_to_string(&cert_path).context("Failed to read certificate")?;
            let key_pem = fs::read_to_string(&key_path).context("Failed to read private key")?;
            let fingerprint = fs::read_to_string(&fingerprint_path)
                .context("Failed to read fingerprint")?
                .trim()
                .to_string();

            return Ok(Self {
                cert_pem,
                key_pem,
                fingerprint,
            });
        }

        let identity = if https {
            generate_https_identity()?
        } else {
            generate_http_identity()?
        };

        fs::write(&cert_path, &identity.cert_pem).context("Failed to write certificate")?;
        fs::write(&key_path, &identity.key_pem).context("Failed to write private key")?;
        fs::write(&fingerprint_path, &identity.fingerprint)
            .context("Failed to write fingerprint")?;

        Ok(identity)
    }
}

fn generate_https_identity() -> Result<Identity> {
    let CertifiedKey { cert, key_pair } = generate_simple_self_signed(vec![
        "127.0.0.1".to_string(),
        "localhost".to_string(),
    ])?;
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();
    let fingerprint = fingerprint_from_cert_pem(&cert_pem)?;

    Ok(Identity {
        cert_pem,
        key_pem,
        fingerprint,
    })
}

fn generate_http_identity() -> Result<Identity> {
    Ok(Identity {
        cert_pem: String::new(),
        key_pem: String::new(),
        fingerprint: random_fingerprint(),
    })
}
