use meshway_configuration::{AddressPools, ConfigurationError, NodeAddressesInput, new_network};

#[test]
fn ipam_pools_reject_intersections_with_excluded_ranges_and_invalid_families() {
    for cidr in [
        "0.0.0.0/0",
        "128.0.0.0/1",
        "126.0.0.0/7",
        "169.252.0.0/14",
        "0.0.0.0/32",
        "0.255.255.255/32",
        "127.0.0.0/32",
        "127.255.255.255/32",
        "169.254.0.0/32",
        "169.254.255.255/32",
        "224.0.0.0/32",
        "239.255.255.255/32",
        "240.0.0.0/32",
        "255.255.255.254/32",
        "255.255.255.255/32",
        "256.0.0.0/8",
        "100.64.0.0/33",
        "fd00::/8",
    ] {
        let pools = AddressPools {
            ipv4: vec![cidr.into()],
            ipv6: vec![],
        };
        assert!(
            matches!(
                new_network(pools, "mesh.example".into()),
                Err(ConfigurationError::Invalid(_))
            ),
            "accepted IPv4 pool {cidr}"
        );
    }
    for cidr in [
        "::/0",
        "::/127",
        "fe00::/8",
        "fe00::/7",
        "fe80::/9",
        "::/128",
        "::1/128",
        "fe80::/128",
        "febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff/128",
        "ff00::/128",
        "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff/128",
        "fd00::/129",
        "100.64.0.0/10",
    ] {
        let pools = AddressPools {
            ipv4: vec![],
            ipv6: vec![cidr.into()],
        };
        assert!(
            matches!(
                new_network(pools, "mesh.example".into()),
                Err(ConfigurationError::Invalid(_))
            ),
            "accepted IPv6 pool {cidr}"
        );
    }
    for json in [
        r#"{"ipv4":"256.0.0.1","ipv6":null}"#,
        r#"{"ipv4":"fd00::1","ipv6":null}"#,
        r#"{"ipv4":null,"ipv6":"100.64.0.1"}"#,
    ] {
        assert!(
            serde_json::from_str::<NodeAddressesInput>(json).is_err(),
            "accepted {json}"
        );
    }
}

#[test]
fn ipam_pools_keep_private_unicast_and_adjacent_allowed_boundaries() {
    // These are IPAM acceptance cases, not internet grant classifications.
    for cidr in [
        "10.0.0.0/8",
        "172.16.0.0/12",
        "192.168.0.0/16",
        "100.64.0.0/10",
        "1.0.0.0/32",
        "126.255.255.255/32",
        "128.0.0.0/32",
        "169.253.255.255/32",
        "169.255.0.0/32",
        "223.255.255.255/32",
        "100.64.0.0/31",
        "100.64.0.255/32",
    ] {
        let pools = AddressPools {
            ipv4: vec![cidr.into()],
            ipv6: vec![],
        };
        assert!(
            new_network(pools, "mesh.example".into()).is_ok(),
            "rejected IPv4 pool {cidr}"
        );
    }
    for cidr in [
        "fc00::/7",
        "fd00::/8",
        "fd42:7a65:8800::/48",
        "fd00::1/128",
        "::2/128",
        "fe00::/9",
        "fe7f:ffff:ffff:ffff:ffff:ffff:ffff:ffff/128",
        "fec0::/128",
        "feff:ffff:ffff:ffff:ffff:ffff:ffff:ffff/128",
    ] {
        let pools = AddressPools {
            ipv4: vec![],
            ipv6: vec![cidr.into()],
        };
        assert!(
            new_network(pools, "mesh.example".into()).is_ok(),
            "rejected IPv6 pool {cidr}"
        );
    }
}
