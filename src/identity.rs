use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use rcgen::{CertifiedKey, generate_simple_self_signed};

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
    let CertifiedKey { cert, key_pair } =
        generate_simple_self_signed(vec!["127.0.0.1".to_string(), "localhost".to_string()])?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fresh_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lsend-id-{}-{}", tag, uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn load_or_create_https_persists_cert_and_key() {
        let dir = fresh_dir("https");
        let identity = Identity::load_or_create(&dir, true).expect("create");

        // Cert and key PEMs are written and not empty.
        assert!(identity.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(identity.key_pem.contains("PRIVATE KEY"));
        // Fingerprint is the SHA-256 hex of the cert DER.
        assert_eq!(identity.fingerprint.len(), 64);
        assert!(identity.fingerprint.chars().all(|c| c.is_ascii_hexdigit()));
        // Fingerprint matches the cert we wrote.
        let cert_fp = fingerprint_from_cert_pem(&identity.cert_pem).unwrap();
        assert_eq!(identity.fingerprint, cert_fp);

        // On-disk files exist.
        assert!(dir.join(CERT_FILE).exists());
        assert!(dir.join(KEY_FILE).exists());
        assert!(dir.join(FINGERPRINT_FILE).exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_or_create_https_round_trips_existing_identity() {
        let dir = fresh_dir("https-roundtrip");
        let first = Identity::load_or_create(&dir, true).unwrap();
        let second = Identity::load_or_create(&dir, true).unwrap();
        assert_eq!(first.cert_pem, second.cert_pem);
        assert_eq!(first.key_pem, second.key_pem);
        assert_eq!(first.fingerprint, second.fingerprint);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_or_create_http_uses_random_fingerprint_and_no_cert() {
        let dir = fresh_dir("http");
        let identity = Identity::load_or_create(&dir, false).expect("create");
        assert!(identity.cert_pem.is_empty());
        assert!(identity.key_pem.is_empty());
        // Random fingerprint is 32 hex chars (UUIDv4 stripped).
        assert_eq!(identity.fingerprint.len(), 32);
        assert!(identity.fingerprint.chars().all(|c| c.is_ascii_hexdigit()));
        // On-disk files may exist but contain empty / random content — the
        // important guarantee is that the in-memory cert/key are empty in
        // HTTP mode.
        let on_disk_cert = fs::read_to_string(dir.join(CERT_FILE)).unwrap_or_default();
        assert!(on_disk_cert.is_empty());
        let on_disk_key = fs::read_to_string(dir.join(KEY_FILE)).unwrap_or_default();
        assert!(on_disk_key.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_or_create_http_persists_fingerprint() {
        let dir = fresh_dir("http-persist");
        let first = Identity::load_or_create(&dir, false).unwrap();
        let second = Identity::load_or_create(&dir, false).unwrap();
        assert_eq!(first.fingerprint, second.fingerprint);
        assert!(dir.join(FINGERPRINT_FILE).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_or_create_https_and_http_have_distinct_fingerprints() {
        let dir_https = fresh_dir("https-fp");
        let dir_http = fresh_dir("http-fp");
        let https = Identity::load_or_create(&dir_https, true).unwrap();
        let http = Identity::load_or_create(&dir_http, false).unwrap();
        // HTTPS fingerprint is 64 hex (SHA-256); HTTP fingerprint is 32 hex (UUID).
        assert_eq!(https.fingerprint.len(), 64);
        assert_eq!(http.fingerprint.len(), 32);
        let _ = fs::remove_dir_all(&dir_https);
        let _ = fs::remove_dir_all(&dir_http);
    }

    #[test]
    fn fingerprint_in_persisted_file_matches_in_memory() {
        let dir = fresh_dir("fp-file");
        let identity = Identity::load_or_create(&dir, true).unwrap();
        let on_disk = fs::read_to_string(dir.join(FINGERPRINT_FILE)).unwrap();
        assert_eq!(on_disk.trim(), identity.fingerprint);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn generate_https_identity_produces_valid_self_signed_cert() {
        let identity = generate_https_identity().expect("generate");
        // Cert is parseable via our fingerprint helper, which would fail on
        // malformed PEMs.
        let fp = fingerprint_from_cert_pem(&identity.cert_pem).expect("parse");
        assert_eq!(fp, identity.fingerprint);
    }

    #[test]
    fn generate_http_identity_randomizes_fingerprint_per_call() {
        let a = generate_http_identity().unwrap();
        let b = generate_http_identity().unwrap();
        assert_ne!(a.fingerprint, b.fingerprint);
    }
}
