//! Installation gates for components with no production system integration.
#[cfg(target_os = "linux")]
pub(super) fn validate(
    cfg: &meshway_configuration::execution::NodeConfiguration,
) -> Result<
    (),
    (
        meshway_ipc::Problem,
        meshway_protocol::execution::ApplyFailure,
    ),
> {
    use meshway_ipc::Problem;
    use meshway_protocol::execution::ApplyFailure;
    if dns_required(&cfg.dns.settings) {
        return Err((Problem::Dns, ApplyFailure::Dns));
    }
    if cfg.exit_enabled
        || !cfg.local_egress.is_empty()
        || !cfg.diagnostics.is_empty()
        || cfg.permits.values().any(|permit| {
            !matches!(
                permit.target,
                meshway_configuration::execution::BusinessTarget::Mesh { .. }
            )
        })
    {
        return Err((Problem::Apply, ApplyFailure::Dataplane));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn dns_required(dns: &meshway_configuration::DnsConfiguration) -> bool {
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
