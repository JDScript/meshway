//! Public-address set matching Headscale's TheInternet baseline: IPv4 minus
//! RFC1918, CGNAT and link-local; IPv6 global unicast 2000::/3.
use super::*;
use ipnet::IpNet;
use std::{net::IpAddr, sync::OnceLock};

pub(super) fn ranges() -> &'static [ExitMatch] {
    static RANGES: OnceLock<Vec<ExitMatch>> = OnceLock::new();
    RANGES.get_or_init(|| {
        let excluded: Vec<IpNet> = [
            "10.0.0.0/8",
            "172.16.0.0/12",
            "192.168.0.0/16",
            "100.64.0.0/10",
            "169.254.0.0/16",
        ]
        .iter()
        .map(|s| s.parse().expect("constant prefix"))
        .collect();
        fn split(net: IpNet, excluded: &[IpNet], out: &mut Vec<ExitMatch>) {
            if excluded.iter().any(|e| e.contains(&net)) {
                return;
            }
            if excluded.iter().any(|e| net.contains(e)) {
                for child in net.subnets(net.prefix_len() + 1).expect("bounded split") {
                    split(child, excluded, out);
                }
            } else {
                out.push(ExitMatch::Cidr(net.to_string()));
            }
        }
        let mut out = Vec::new();
        split(
            "0.0.0.0/0".parse().expect("constant prefix"),
            &excluded,
            &mut out,
        );
        out.push(ExitMatch::Cidr("2000::/3".into()));
        out
    })
}
pub(super) fn contains(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            !ip.is_private()
                && !ip.is_link_local()
                && !(octets[0] == 100 && (64..=127).contains(&octets[1]))
        }
        IpAddr::V6(ip) => ip.segments()[0] & 0xe000 == 0x2000,
    }
}
pub(super) fn covered(base: &Compiled, src: usize, destination: &ExitMatch) -> bool {
    let grant = &base.external_grants[src];
    // Domain authorization via Internet is conditional on the resolved address
    // being public; the runtime gate checks the actual IP, including private DNS.
    (grant.internet && matches!(destination, ExitMatch::DomainSuffix(_)))
        || base.grant_index.covered(destination, &grant.range_grants)
}
pub(super) fn all_public(base: &Compiled, src: usize) -> bool {
    let grant = &base.external_grants[src];
    grant.internet
        || ranges()
            .iter()
            .all(|d| base.grant_index.covered(d, &grant.range_grants))
}
