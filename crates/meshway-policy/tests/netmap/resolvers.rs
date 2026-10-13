//! A resolver kept under an exit (`useWithExit`) is reached from the source
//! itself: its address is a direct exception of the exit ranges, so the
//! source's DNS installs under a global exit profile.
use super::*;
use meshway_policy::dns_route::netmap_resolver;

fn server(address: &str, use_with_exit: bool) -> DnsServer {
    DnsServer {
        transport: DnsTransport::Udp,
        server: address.parse().unwrap(),
        server_port: 53.try_into().unwrap(),
        use_with_exit,
    }
}

#[test]
fn a_kept_resolver_is_a_direct_exception_of_an_exit_profile() {
    let mut s = super::exit::setup();
    let global = format!("global:{}", node(4));
    let assignment = &mut s.cfg.routing.egress.assignments[0];
    assignment.allowed.push(global.clone());
    assignment.default = global;
    s.cfg.dns.servers = BTreeMap::from([
        ("office".into(), server("10.20.0.53", true)),
        ("public".into(), server("1.1.1.1", false)),
        ("mesh".into(), server("100.64.0.2", true)),
    ]);
    s.cfg.dns.routes = BTreeMap::from([("corp.example".into(), vec!["office".into()])]);
    let compiled = netmaps(&s);
    let source = &compiled.nodes[&node(1)].netmap;
    assert_eq!(
        source.dns.profile,
        format!("global:{}", node(4)),
        "the global profile is executed"
    );
    // Names resolve through the exit, the kept resolver from the source.
    assert!(matches!(&source.dns.egress, DnsPlan::ViaExit { service } if service.exit == node(4)));
    assert_eq!(
        netmap_resolver(source, "office"),
        Ok(ResolverAction::Direct)
    );
    assert_eq!(netmap_resolver(source, "public"), Ok(ResolverAction::Exit));
    assert_eq!(
        netmap_resolver(source, "mesh"),
        Ok(ResolverAction::Rejected)
    );
    // The capture keeps every other address of the exit range.
    let capture = LocalCompiler::new(source, &Default::default())
        .capture()
        .unwrap()
        .unwrap();
    let managed = |address: &str| {
        let address: std::net::Ipv4Addr = address.parse().unwrap();
        capture
            .managed_ipv4
            .iter()
            .any(|(start, end)| *start <= address && address <= *end)
    };
    assert!(!managed("10.20.0.53"));
    assert!(managed("10.20.0.52") && managed("10.20.0.54") && managed("1.1.1.1"));
    assert!(capture.default_capture);
    let decided = LocalCompiler::new(source, &Default::default())
        .decide(
            node(1),
            "10.20.0.53".parse().unwrap(),
            BusinessProtocol::Udp,
            port(53),
        )
        .unwrap();
    assert_eq!(decided, LocalDecision::Direct);
}
