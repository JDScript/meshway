//! Unique Internet address-set definition for validation, compilation and admission.
use ipnet::IpNet;
use std::{net::IpAddr, sync::OnceLock};

/// Conservative global-unicast set. Special-use, documentation, benchmarking,
/// transition and reserved ranges are excluded, including mapped IPv6 addresses.
pub fn internet_ranges() -> &'static [IpNet] {
    static RANGES: OnceLock<Vec<IpNet>> = OnceLock::new();
    RANGES.get_or_init(|| {
        let excluded: Vec<IpNet> = [
            "0.0.0.0/8",
            "10.0.0.0/8",
            "100.64.0.0/10",
            "127.0.0.0/8",
            "169.254.0.0/16",
            "172.16.0.0/12",
            "192.0.0.0/24",
            "192.0.2.0/24",
            "192.88.99.0/24",
            "192.168.0.0/16",
            "198.18.0.0/15",
            "198.51.100.0/24",
            "203.0.113.0/24",
            "224.0.0.0/4",
            "240.0.0.0/4",
            "2001::/23",
            "2001:db8::/32",
            "2002::/16",
            "3fff::/20",
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
        for root in ["0.0.0.0/0", "2000::/3"] {
            split(root.parse().expect("constant prefix"), &excluded, &mut out);
        }
        out
    })
}

/// True only for an address in the public Internet set. Mapped IPv6 is not an
/// alternate representation of an executable IPv4 exit destination.
pub fn is_internet_address(address: IpAddr) -> bool {
    internet_ranges().iter().any(|net| net.contains(&address))
}

/// Absolute external forwarding exclusions, including explicitly granted CIDRs.
/// RFC1918 and CGNAT remain possible only through explicit CIDR Access.
/// Mesh pools are excluded by their owners, which know them. IPv4-mapped
/// IPv6 is not an alias of an IPv4 exit destination, and a NAT64 prefix is
/// never translated to IPv4 through an exit.
pub fn is_exit_destination(address: IpAddr) -> bool {
    !exit_exclusions().iter().any(|net| net.contains(&address))
}

/// The prefixes [`is_exit_destination`] excludes; the exit's nft program
/// drops the same ones.
pub fn exit_exclusions() -> &'static [IpNet] {
    static EXCLUDED: OnceLock<Vec<IpNet>> = OnceLock::new();
    EXCLUDED.get_or_init(|| {
        [
            "0.0.0.0/8",
            "127.0.0.0/8",
            "169.254.0.0/16",
            "224.0.0.0/4",
            "240.0.0.0/4",
            "::/128",
            "::1/128",
            "::ffff:0:0/96",
            "64:ff9b::/96",
            "64:ff9b:1::/48",
            "fe80::/10",
            "ff00::/8",
        ]
        .iter()
        .map(|net| net.parse().expect("constant prefix"))
        .collect()
    })
}
