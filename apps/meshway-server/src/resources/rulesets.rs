use super::{MAX_DATASET_BYTES, MAX_RULE_ENTRIES};
use meshway_configuration::RuleSetFormat;
use meshway_protocol::resources::{ResourceProblem, RuleSetData};
use serde::Deserialize;
use std::{collections::BTreeSet, net::IpAddr};

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
    domain: Vec<String>,
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
                domain: vec![],
            };
            for rule in input.rules {
                // sing-box combines different match categories with AND. Our
                // union-only set cannot preserve a mixed rule, so reject it.
                // Exact and suffix names are both name categories: a rule
                // listing both matches a name that is either, a union.
                if (!rule.domain_suffix.is_empty() || !rule.domain.is_empty())
                    && !rule.ip_cidr.is_empty()
                {
                    return Err(ResourceProblem::ConversionFailed);
                }
                if rule.domain_suffix.is_empty()
                    && rule.domain.is_empty()
                    && rule.ip_cidr.is_empty()
                {
                    return Err(ResourceProblem::ConversionFailed);
                }
                data.domain.extend(rule.domain);
                data.domain_suffix.extend(rule.domain_suffix);
                data.ip_cidr.extend(rule.ip_cidr);
            }
            data
        }
        RuleSetFormat::MihomoDomain => {
            let input: MihomoSource =
                serde_yaml_ng::from_slice(bytes).map_err(|_| ResourceProblem::ConversionFailed)?;
            let mut suffixes = vec![];
            let mut exact = vec![];
            for entry in input.payload {
                // +. is the name and its children; a bare name is exact. The
                // *. form (one wildcard label, apex excluded) and any other
                // wildcard have no equivalent and fail the conversion.
                if let Some(suffix) = entry.strip_prefix("+.") {
                    suffixes.push(suffix.to_owned());
                } else if entry.contains(['*', '+']) || entry.starts_with('.') {
                    return Err(ResourceProblem::ConversionFailed);
                } else {
                    exact.push(entry);
                }
            }
            RuleSetData {
                ip_cidr: vec![],
                domain_suffix: suffixes,
                domain: exact,
            }
        }
        RuleSetFormat::MihomoIpcidr => {
            let input: MihomoSource =
                serde_yaml_ng::from_slice(bytes).map_err(|_| ResourceProblem::ConversionFailed)?;
            RuleSetData {
                ip_cidr: input.payload,
                domain_suffix: vec![],
                domain: vec![],
            }
        }
    };
    normalize(data)
}
pub fn entries(data: &RuleSetData) -> usize {
    data.ip_cidr.len() + data.domain_suffix.len() + data.domain.len()
}
pub fn normalize(data: RuleSetData) -> Result<RuleSetData, ResourceProblem> {
    if entries(&data) > MAX_RULE_ENTRIES || entries(&data) == 0 {
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
    let names = |names: Vec<String>| -> Result<Vec<String>, ResourceProblem> {
        names
            .into_iter()
            .map(|name| canonical_domain(&name))
            .collect::<Result<BTreeSet<_>, _>>()
            .map(|set| set.into_iter().collect())
    };
    Ok(RuleSetData {
        ip_cidr: prefixes.into_iter().collect(),
        domain_suffix: names(data.domain_suffix)?,
        domain: names(data.domain)?,
    })
}
fn canonical_domain(name: &str) -> Result<String, ResourceProblem> {
    let name = idna::domain_to_ascii(name.trim_end_matches('.'))
        .map_err(|_| ResourceProblem::ConversionFailed)?
        .to_ascii_lowercase();
    if name.is_empty()
        || name.len() > 253
        || name.split('.').any(|label| {
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
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_semantic_widening_and_unknown_fields() {
        assert!(
            convert(
                br#"{"version":1,"rules":[{"domain_keyword":["example"]}]}"#,
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
        assert!(
            convert(
                b"payload:\n  - '.example.com'",
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
        assert!(parsed.domain.is_empty());
    }
    #[test]
    fn exact_names_stay_exact() {
        // MetaCubeX geosite lists mix `+.` suffixes with bare exact names.
        let parsed = convert(
            b"payload:\n  - 'google.com'\n  - '+.google.com'\n  - 'Github.com.'\n  - 'github.com'",
            &RuleSetFormat::MihomoDomain,
        )
        .unwrap();
        assert_eq!(parsed.domain_suffix, vec!["google.com"]);
        assert_eq!(parsed.domain, vec!["github.com", "google.com"]);
        let parsed = convert(
            br#"{"version":2,"rules":[{"domain":["a.example.com"],"domain_suffix":["b.example.com"]},{"ip_cidr":["10.0.0.0/8"]}]}"#,
            &RuleSetFormat::SingBoxSource,
        )
        .unwrap();
        assert_eq!(parsed.domain, vec!["a.example.com"]);
        assert_eq!(parsed.domain_suffix, vec!["b.example.com"]);
        assert_eq!(parsed.ip_cidr, vec!["10.0.0.0/8"]);
        assert!(
            convert(
                br#"{"version":2,"rules":[{"domain":["a.example.com"],"ip_cidr":["10.0.0.0/8"]}]}"#,
                &RuleSetFormat::SingBoxSource,
            )
            .is_err()
        );
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
