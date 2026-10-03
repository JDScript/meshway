//! The existing Meshway/Tailscale Internet address-set definition: IPv4 minus
//! RFC1918, CGNAT and link-local; IPv6 global unicast 2000::/3. Shared by declared
//! destination validation and execution consumers, not a DNS trust assertion.
use ipnet::IpNet;
use std::sync::OnceLock;

pub fn internet_ranges() -> &'static [IpNet] {
    static RANGES: OnceLock<Vec<IpNet>> = OnceLock::new();
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
        fn split(net: IpNet, excluded: &[IpNet], out: &mut Vec<IpNet>) {
            if excluded.iter().any(|e| e.contains(&net)) {
                return;
            }
            if excluded.iter().any(|e| net.contains(e)) {
                for child in net.subnets(net.prefix_len() + 1).expect("bounded split") {
                    split(child, excluded, out);
                }
            } else {
                out.push(net);
            }
        }
        let mut out = Vec::new();
        split(
            "0.0.0.0/0".parse().expect("constant prefix"),
            &excluded,
            &mut out,
        );
        out.push("2000::/3".parse().expect("constant prefix"));
        out
    })
}

/// Union coverage, including adjacent grants that together cover a target.
pub fn address_set_covers(grants: &[IpNet], target: &IpNet) -> bool {
    IpNet::aggregate(&grants.to_vec())
        .iter()
        .any(|grant| grant.contains(target))
}
