//! Installation gates for components with no production system integration.

/// Whether the netmap's DNS settings require the system DNS owner.
#[cfg(any(target_os = "linux", test))]
pub(super) fn dns_required(dns: &meshway_configuration::DnsConfiguration) -> bool {
    dns.magic_dns
        || dns.override_local
        || !dns.servers.is_empty()
        || !dns.default.is_empty()
        || !dns.routes.is_empty()
        || !dns.search_domains.is_empty()
}

#[cfg(test)]
mod tests {
    use super::dns_required;
    use meshway_configuration::DnsConfiguration;

    #[test]
    fn dns_requires_explicit_server_configuration_disabling_all_system_features() {
        let defaults: DnsConfiguration =
            serde_json::from_value(serde_json::json!({"domain":"mesh.test"})).unwrap();
        assert!(dns_required(&defaults));
        let disabled: DnsConfiguration =
            serde_json::from_value(serde_json::json!({"domain":"mesh.test","magicDNS":false}))
                .unwrap();
        assert!(!dns_required(&disabled));
        for (key, value) in [
            ("overrideLocal", serde_json::json!(true)),
            ("searchDomains", serde_json::json!(["mesh.test"])),
            (
                "servers",
                serde_json::json!({"regular":{"type":"udp","server":"192.0.2.53"}}),
            ),
            ("default", serde_json::json!(["regular"])),
            ("routes", serde_json::json!({"mesh.test":["regular"]})),
        ] {
            let mut value_with_feature = serde_json::to_value(&disabled).unwrap();
            value_with_feature[key] = value;
            let configured: DnsConfiguration = serde_json::from_value(value_with_feature).unwrap();
            assert!(dns_required(&configured), "required DNS field: {key}");
        }
    }
}
