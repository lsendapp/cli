use std::fmt::{Debug, Formatter};
use std::sync::Arc;

use rustls::client::danger::HandshakeSignatureValid;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::server::WebPkiClientVerifier;
use rustls::{DigitallySignedStruct, DistinguishedName, Error, RootCertStore, SignatureScheme};

/// mTLS client certificate verifier that mirrors the official LocalSend core's
/// `CustomClientCertVerifier`:
///
/// - Client authentication is mandatory (`client_auth_mandatory = true`).
/// - Any certificate that is cryptographically valid (parseable, within its
///   validity period, self-signature verifies) is accepted. We don't pin a
///   specific client identity; the trust model is "is a LocalSend peer".
///
/// The `WebPkiClientVerifier` is used as a scaffold to satisfy the rustls
/// `ClientCertVerifier` API and to reuse its TLS 1.2 / 1.3 signature-scheme
/// verifiers. Its built-in chain-to-root validation is bypassed by overriding
/// `verify_client_cert`.
pub struct LocalSendClientCertVerifier {
    inner: Arc<dyn ClientCertVerifier>,
}

impl LocalSendClientCertVerifier {
    pub fn try_new(server_cert_pem: &str) -> anyhow::Result<Self> {
        // The root store is required to be non-empty by `WebPkiClientVerifier`.
        // We add the server's own certificate purely to satisfy that requirement;
        // we do not actually trust any chain rooted at it (see
        // `verify_client_cert` below).
        let mut root_cert_store = RootCertStore::empty();
        root_cert_store.add(PemObject::from_pem_slice(server_cert_pem.as_bytes())?)?;

        Ok(Self {
            inner: WebPkiClientVerifier::builder(Arc::new(root_cert_store)).build()?,
        })
    }
}

impl Debug for LocalSendClientCertVerifier {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.inner.fmt(f)
    }
}

impl ClientCertVerifier for LocalSendClientCertVerifier {
    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        self.inner.root_hint_subjects()
    }

    fn verify_client_cert(
        &self,
        cert: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: UnixTime,
    ) -> Result<ClientCertVerified, Error> {
        // Trust any certificate that is valid.
        localsend::crypto::cert::verify_cert_from_der(&cert[..], None).map_err(|e| {
            tracing::warn!("Client certificate verification failed: {e:#}");
            Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure)
        })?;
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_cert_pem() -> String {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        cert.cert.pem()
    }

    #[test]
    fn try_new_accepts_valid_pem() {
        let pem = sample_cert_pem();
        let verifier = LocalSendClientCertVerifier::try_new(&pem).expect("verifier");
        // Verifier should advertise mandatory client auth and a non-empty
        // subject hint list rooted at the server cert.
        assert!(verifier.offer_client_auth());
        assert!(verifier.client_auth_mandatory());
        assert!(!verifier.root_hint_subjects().is_empty());
        // At minimum one signature scheme should be supported.
        assert!(!verifier.supported_verify_schemes().is_empty());
    }

    #[test]
    fn try_new_rejects_invalid_pem() {
        let result = LocalSendClientCertVerifier::try_new("not a cert");
        assert!(result.is_err());
    }

    #[test]
    fn debug_impl_does_not_panic() {
        let pem = sample_cert_pem();
        let verifier = LocalSendClientCertVerifier::try_new(&pem).unwrap();
        let s = format!("{verifier:?}");
        assert!(!s.is_empty());
    }
}
