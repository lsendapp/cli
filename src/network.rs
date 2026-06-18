use std::net::Ipv4Addr;

use anyhow::{Context, Result};
use if_addrs::{get_if_addrs, IfAddr};
use localsend::http::client::{LsHttpClient, LsHttpClientV2, LsHttpClientVersion};

use crate::identity::Identity;

pub fn list_ipv4_interfaces() -> Vec<Ipv4Addr> {
    let mut addrs = Vec::new();
    let Ok(interfaces) = get_if_addrs() else {
        return addrs;
    };

    for iface in interfaces {
        if !iface.is_oper_up() || iface.is_loopback() {
            continue;
        }
        if let IfAddr::V4(v4) = iface.addr {
            let ip = v4.ip;
            if ip.is_unspecified() || ip.is_link_local() || ip.is_multicast() {
                continue;
            }
            if !is_usable_interface(&iface.name, ip) {
                continue;
            }
            addrs.push(ip);
        }
    }

    addrs.sort();
    addrs.dedup();
    addrs
}

fn is_usable_interface(name: &str, ip: Ipv4Addr) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("utun")
        || lower.starts_with("gif")
        || lower.starts_with("stf")
        || lower.starts_with("bridge")
        || lower.starts_with("lo")
        || lower.starts_with("awdl")
        || lower.starts_with("llw")
    {
        return false;
    }

    let octets = ip.octets();
    if octets[0] == 198 && (octets[1] == 18 || octets[1] == 19) {
        return false;
    }
    if octets[0] == 100 && (64..=127).contains(&octets[1]) {
        return false;
    }

    true
}

pub fn build_http_client(identity: &Identity, https: bool) -> Result<LsHttpClient> {
    if https {
        Ok(LsHttpClient::new(
            &identity.key_pem,
            &identity.cert_pem,
            LsHttpClientVersion::V2,
        )?)
    } else {
        Ok(LsHttpClient::V2(
            LsHttpClientV2::try_new_without_cert().context("Failed to create HTTP client")?,
        ))
    }
}

pub fn build_reqwest_client(identity: &Identity, https: bool) -> Result<localsend::reqwest::Client> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut builder = localsend::reqwest::Client::builder()
        .use_rustls_tls()
        .danger_accept_invalid_certs(true);

    if https {
        let pem = [
            identity.cert_pem.as_bytes(),
            b"\n",
            identity.key_pem.as_bytes(),
        ]
        .concat();
        let id = localsend::reqwest::Identity::from_pem(&pem)?;
        builder = builder.identity(id);
    }

    Ok(builder.build()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_clash_fake_ip_interface() {
        assert!(!is_usable_interface("utun8", Ipv4Addr::new(198, 18, 0, 1)));
        assert!(is_usable_interface("en0", Ipv4Addr::new(192, 168, 30, 52)));
    }
}
