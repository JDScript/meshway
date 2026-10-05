//! Installation gates for components with no production system integration.
#[cfg(target_os = "linux")]
pub(super) fn validate(
    cfg: &meshway_configuration::execution::NodeConfiguration,
    dns_ids: &std::collections::BTreeMap<meshway_dns::ResolverSource, meshway_model::RequestId>,
    retired_dns: &std::collections::BTreeMap<
        meshway_model::RequestId,
        super::dns::intents::Retired,
    >,
    now_ms: u64,
) -> Result<
    (),
    (
        meshway_ipc::Problem,
        meshway_protocol::execution::ApplyFailure,
    ),
> {
    use meshway_ipc::Problem;
    use meshway_protocol::execution::ApplyFailure;
    if !matches!(
        cfg.dns.egress,
        meshway_configuration::execution::DnsEgress::NoExit
    ) {
        return Err((Problem::Dns, ApplyFailure::Dns));
    }
    if cfg.exit_enabled
        || !owns_all(
            cfg.local_egress.iter().filter_map(|(id, permit)| {
                // Old DNS intents cannot execute after leaving Scope. Exact,
                // bounded owner-created metadata need not fence unrelated flows
                // while Server expiry removes it from the next publication.
                (!retired_dns
                    .get(id)
                    .is_some_and(|retired| retired.matches(cfg.node_id, permit, now_ms)))
                .then_some(id)
            }),
            dns_ids,
        )
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
fn owns_all<'a>(
    mut local: impl Iterator<Item = &'a meshway_model::RequestId>,
    dns: &std::collections::BTreeMap<meshway_dns::ResolverSource, meshway_model::RequestId>,
) -> bool {
    // Selection is necessary, never sufficient: Runtime::install additionally
    // validates every sealed resolver route against the exact execution lease.
    local.all(|id| dns.values().any(|owned| owned == id))
}

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
    fn unrelated_local_egress_cannot_pass_the_dns_component_gate() {
        let owned: meshway_model::RequestId =
            "00000000-0000-4000-8000-000000000001".parse().unwrap();
        let foreign: meshway_model::RequestId =
            "00000000-0000-4000-8000-000000000002".parse().unwrap();
        let dns = std::collections::BTreeMap::from([(
            meshway_dns::ResolverSource::Configured("primary".into()),
            owned,
        )]);
        assert!(super::owns_all([owned].iter(), &dns));
        assert!(!super::owns_all([owned, foreign].iter(), &dns));
        assert!(!super::owns_all([foreign].iter(), &Default::default()));
    }

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
