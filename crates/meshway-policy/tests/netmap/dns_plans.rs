//! The DNS gaps the Server saves by equal those each Node installs by: the
//! same `resolver_plan_gaps` on the full configuration (every Node and
//! profile) and on the Node's projected netmap.
use super::*;
use meshway_policy::dns_route::netmap_resolver_plan_gaps;

fn server(address: &str, use_with_exit: bool) -> DnsServer {
    DnsServer {
        transport: DnsTransport::Udp,
        server: address.parse().unwrap(),
        server_port: 53.try_into().unwrap(),
        use_with_exit,
    }
}

/// DNS settings drawn after every draw of [`generated`]: resolvers that a
/// Node can query, loopback, Mesh and (under the generated exit rules)
/// exit-routed ones, and search domains under and outside the routes.
fn with_dns(seed: u64, s: &mut Scenario) {
    let mut rng = Rng::new(seed ^ 0xd45);
    let dns = &mut s.cfg.dns;
    dns.magic_dns = rng.chance(70);
    let candidates = [
        ("public", "1.1.1.1"),
        ("office", "10.20.0.53"),
        ("stub", "127.0.0.53"),
        ("mesh", "100.64.0.9"),
        ("peer", "100.64.0.3"),
        ("scoped", "fe80::53"),
        ("v6", "2001:4860:4860::8888"),
    ];
    for (key, address) in candidates {
        if rng.chance(40) {
            dns.servers
                .insert(key.into(), server(address, rng.chance(50)));
        }
    }
    let keys: Vec<String> = dns.servers.keys().cloned().collect();
    if !keys.is_empty() {
        if rng.chance(50) {
            dns.routes
                .insert("corp.example".into(), vec![rng.pick(&keys).clone()]);
        }
        if rng.chance(30) {
            dns.default = vec![rng.pick(&keys).clone()];
            dns.override_local = rng.chance(50);
        }
    }
    for suffix in ["test.internal", "eu.corp.example", "elsewhere.example"] {
        if rng.chance(35) {
            dns.search_domains.push(suffix.into());
        }
    }
}

#[test]
fn server_and_node_judge_the_same_dns_gaps() {
    let (mut compared, mut refused, mut accepted) = (0, 0, 0);
    let mut reasons = std::collections::BTreeSet::new();
    for seed in 0..96 {
        let mut s = generated(seed);
        with_dns(seed, &mut s);
        let compiled = netmaps(&s);
        for (id, netmap) in &compiled.nodes {
            let netmap = &netmap.netmap;
            // Without a partition every resolver is rejected; its egress
            // is reported apart (`egress_partition_limit`).
            if !s.cfg.nodes[id].enabled
                || netmap
                    .egress
                    .as_ref()
                    .is_none_or(|e| e.unavailable.is_some())
            {
                continue;
            }
            let node = netmap_resolver_plan_gaps(netmap);
            let server = profile_resolver_plan_gaps(
                &s.cfg,
                &BTreeMap::new(),
                &netmap.dns.profile,
                Some(*id),
            )
            .unwrap();
            assert_eq!(node, server, "seed={seed} node={id}");
            reasons.extend(node.iter().map(ResolverPlanGap::reason));
            compared += 1;
            if node.is_empty() {
                accepted += 1;
            } else {
                refused += 1;
            }
        }
    }
    assert_eq!(
        reasons.into_iter().collect::<Vec<_>>(),
        [
            "local_resolver",
            "routed_resolver",
            "search_domain_without_resolver"
        ]
    );
    assert!(
        compared > 300 && refused > 50 && accepted > 50,
        "{compared} compared; {refused} refused; {accepted} accepted"
    );
}
