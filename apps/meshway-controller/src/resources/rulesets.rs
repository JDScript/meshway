use super::{MAX_DATASET_BYTES, MAX_RULE_ENTRIES};
use meshway_configuration::RuleSetFormat;
use meshway_protocol::resources::{ResourceProblem, RuleSetData};
use serde::Deserialize;
use std::{collections::BTreeSet, net::IpAddr, time::Duration};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SingBoxSource {
    version: u8,
    rules: Vec<SingBoxRule>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SingBoxRule {
    #[serde(default)]
    domain_suffix: Vec<String>,
    #[serde(default)]
    ip_cidr: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MihomoSource {
    payload: Vec<String>,
}

pub fn convert(bytes: &[u8], format: &RuleSetFormat) -> Result<RuleSetData, ResourceProblem> {
    if bytes.len() > MAX_DATASET_BYTES {
        return Err(ResourceProblem::TooLarge);
    }
    let data = match format {
        RuleSetFormat::SingBoxSource => {
            let input: SingBoxSource =
                serde_json::from_slice(bytes).map_err(|_| ResourceProblem::ConversionFailed)?;
            if !(1..=3).contains(&input.version) {
                return Err(ResourceProblem::ConversionFailed);
            }
            let mut data = RuleSetData {
                ip_cidr: vec![],
                domain_suffix: vec![],
            };
            for rule in input.rules {
                // sing-box combines different match categories with AND. Our
                // union-only set cannot preserve a mixed rule, so reject it.
                if !rule.domain_suffix.is_empty() && !rule.ip_cidr.is_empty() {
                    return Err(ResourceProblem::ConversionFailed);
                }
                if rule.domain_suffix.is_empty() && rule.ip_cidr.is_empty() {
                    return Err(ResourceProblem::ConversionFailed);
                }
                data.domain_suffix.extend(rule.domain_suffix);
                data.ip_cidr.extend(rule.ip_cidr);
            }
            data
        }
        RuleSetFormat::MihomoDomain => {
            let input: MihomoSource =
                serde_yaml_ng::from_slice(bytes).map_err(|_| ResourceProblem::ConversionFailed)?;
            let mut suffixes = vec![];
            for entry in input.payload {
                // Only +. includes both apex and children. Bare exact domains
                // and *. exclusions cannot be widened into suffix matches.
                let suffix = entry
                    .strip_prefix("+.")
                    .ok_or(ResourceProblem::ConversionFailed)?;
                suffixes.push(suffix.to_owned());
            }
            RuleSetData {
                ip_cidr: vec![],
                domain_suffix: suffixes,
            }
        }
        RuleSetFormat::MihomoIpcidr => {
            let input: MihomoSource =
                serde_yaml_ng::from_slice(bytes).map_err(|_| ResourceProblem::ConversionFailed)?;
            RuleSetData {
                ip_cidr: input.payload,
                domain_suffix: vec![],
            }
        }
    };
    normalize(data)
}
pub fn normalize(data: RuleSetData) -> Result<RuleSetData, ResourceProblem> {
    if data.ip_cidr.len() + data.domain_suffix.len() > MAX_RULE_ENTRIES
        || data.ip_cidr.is_empty() && data.domain_suffix.is_empty()
    {
        return Err(ResourceProblem::ConversionFailed);
    }
    let mut prefixes = BTreeSet::new();
    for prefix in data.ip_cidr {
        let (address, bits) = prefix
            .split_once('/')
            .ok_or(ResourceProblem::ConversionFailed)?;
        let address: IpAddr = address
            .parse()
            .map_err(|_| ResourceProblem::ConversionFailed)?;
        let bits: u8 = bits
            .parse()
            .map_err(|_| ResourceProblem::ConversionFailed)?;
        let canonical = match address {
            IpAddr::V4(ip) if bits <= 32 => {
                let mask = u32::MAX.checked_shl(u32::from(32 - bits)).unwrap_or(0);
                format!(
                    "{}/{}",
                    std::net::Ipv4Addr::from(u32::from(ip) & mask),
                    bits
                )
            }
            IpAddr::V6(ip) if bits <= 128 => {
                let mask = u128::MAX.checked_shl(u32::from(128 - bits)).unwrap_or(0);
                format!(
                    "{}/{}",
                    std::net::Ipv6Addr::from(u128::from(ip) & mask),
                    bits
                )
            }
            _ => return Err(ResourceProblem::ConversionFailed),
        };
        prefixes.insert(canonical);
    }
    let mut suffixes = BTreeSet::new();
    for suffix in data.domain_suffix {
        let suffix = idna::domain_to_ascii(suffix.trim_end_matches('.'))
            .map_err(|_| ResourceProblem::ConversionFailed)?
            .to_ascii_lowercase();
        if suffix.is_empty()
            || suffix.len() > 253
            || suffix.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
        {
            return Err(ResourceProblem::ConversionFailed);
        }
        suffixes.insert(suffix);
    }
    Ok(RuleSetData {
        ip_cidr: prefixes.into_iter().collect(),
        domain_suffix: suffixes.into_iter().collect(),
    })
}

/// Per-download client has no ambient proxy, cookies, redirects or credentials.
/// Only resolved public addresses are pinned to avoid DNS rebinding into LAN.
pub async fn download(source: &str) -> Result<Vec<u8>, ResourceProblem> {
    let url = reqwest::Url::parse(source).map_err(|_| ResourceProblem::InvalidSource)?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || source.len() > 2048
    {
        return Err(ResourceProblem::InvalidSource);
    }
    let host = url.host_str().ok_or(ResourceProblem::InvalidSource)?;
    let port = url
        .port_or_known_default()
        .ok_or(ResourceProblem::InvalidSource)?;
    tokio::time::timeout(Duration::from_secs(30), async {
        let addresses: Vec<_> = tokio::net::lookup_host((host, port))
            .await
            .map_err(|_| ResourceProblem::DownloadFailed)?
            .collect();
        if addresses.is_empty()
            || addresses.len() > 16
            || addresses.iter().any(|a| !super::geoip::is_public(a.ip()))
        {
            return Err(ResourceProblem::InvalidSource);
        }
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(25))
            .resolve_to_addrs(host, &addresses)
            .build()
            .map_err(|_| ResourceProblem::DownloadFailed)?;
        let mut response = client
            .get(url.clone())
            .send()
            .await
            .map_err(|_| ResourceProblem::DownloadFailed)?;
        if !response.status().is_success() {
            return Err(ResourceProblem::DownloadFailed);
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_DATASET_BYTES as u64)
        {
            return Err(ResourceProblem::TooLarge);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| ResourceProblem::DownloadFailed)?
        {
            if bytes.len() + chunk.len() > MAX_DATASET_BYTES {
                return Err(ResourceProblem::TooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| ResourceProblem::DownloadFailed)?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_semantic_widening_and_unknown_fields() {
        assert!(
            convert(
                br#"{"version":1,"rules":[{"domain":["example.com"]}]}"#,
                &RuleSetFormat::SingBoxSource
            )
            .is_err()
        );
        assert!(convert(br#"{"version":1,"rules":[{"domain_suffix":["example.com"],"ip_cidr":["10.0.0.0/8"]}]}"#, &RuleSetFormat::SingBoxSource).is_err());
        assert!(
            convert(
                b"payload:\n  - '*.example.com'",
                &RuleSetFormat::MihomoDomain
            )
            .is_err()
        );
        let parsed = convert(
            b"payload:\n  - '+.Example.com'",
            &RuleSetFormat::MihomoDomain,
        )
        .unwrap();
        assert_eq!(parsed.domain_suffix, vec!["example.com"]);
    }
    #[test]
    fn canonical_prefixes_and_duplicate_entries() {
        let parsed = convert(
            b"payload:\n  - 10.4.3.2/8\n  - 10.0.0.0/8",
            &RuleSetFormat::MihomoIpcidr,
        )
        .unwrap();
        assert_eq!(parsed.ip_cidr, vec!["10.0.0.0/8"]);
    }
}
