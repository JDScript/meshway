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
    if (cfg.exit_enabled && cfg.exit_role.is_none())
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
        || !diagnostic_paths_valid(cfg)
    {
        return Err((Problem::Apply, ApplyFailure::Dataplane));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn diagnostic_paths_valid(cfg: &meshway_configuration::execution::NodeConfiguration) -> bool {
    cfg.diagnostics.values().all(|permit| {
        // Every participant receives the complete candidate set. An optional
        // relay need only occur on one candidate; per-frame policy still checks
        // the actual full path and this node's position within it.
        permit.paths.iter().all(|path| path.nodes.len() >= 2)
            && permit
                .paths
                .iter()
                .any(|path| path.nodes.contains(&cfg.node_id))
    })
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

    #[tokio::test]
    async fn optional_relay_installs_the_full_diagnostic_candidate_set() {
        let mut fixture =
            crate::runtime::ping::integration::Fixture::open_with_optional_relay().await;
        for control in &mut fixture.controls {
            control.refresh_configuration().await.unwrap();
            let snapshot = control.execution_snapshot().unwrap();
            let cfg = snapshot.configuration().unwrap();
            let permit = cfg.diagnostics.values().next().unwrap();
            assert_eq!(permit.paths.len(), 2);
            assert!(
                super::diagnostic_paths_valid(cfg),
                "node {} must accept its participating candidate",
                cfg.node_id
            );
            let mut malformed = cfg.clone();
            malformed.diagnostics.values_mut().next().unwrap().paths[0]
                .nodes
                .clear();
            assert!(!super::diagnostic_paths_valid(&malformed));
            let mut unrelated = cfg.clone();
            unrelated
                .diagnostics
                .values_mut()
                .next()
                .unwrap()
                .paths
                .retain(|path| !path.nodes.contains(&cfg.node_id));
            assert!(!super::diagnostic_paths_valid(&unrelated));
        }
        fixture.close().await;
    }

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
