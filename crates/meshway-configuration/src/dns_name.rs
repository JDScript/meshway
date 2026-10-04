//! Bounded DNS host normalization shared by configuration and discovery.
use std::net::IpAddr;

/// Bound raw UTF-8 before allocating inside IDNA; normalized names are <=253
/// ASCII bytes. Allows Unicode input and an optional trailing root dot.
pub const MAX_DNS_NAME_INPUT_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid or overlong DNS name")]
pub struct DnsNameError;

pub fn normalize_dns_name(value: &str) -> Result<String, DnsNameError> {
    if value.len() > MAX_DNS_NAME_INPUT_BYTES {
        return Err(DnsNameError);
    }
    let normalized = meshway_transport_spec::normalize_host(value).map_err(|_| DnsNameError)?;
    if normalized.parse::<IpAddr>().is_ok() {
        return Err(DnsNameError);
    }
    Ok(normalized)
}

/// Assemble a canonical single-label Node hostname and network domain. The
/// combined name, not just each component, must fit the DNS host limit.
pub fn node_dns_name(hostname: &str, domain: &str) -> Result<String, DnsNameError> {
    if !crate::valid_node_hostname(hostname) {
        return Err(DnsNameError);
    }
    let domain = normalize_dns_name(domain)?;
    if hostname.len() + 1 + domain.len() > 253 {
        return Err(DnsNameError);
    }
    normalize_dns_name(&format!("{hostname}.{domain}"))
}
