//! An exit resolver's served upstreams: resolved's proxy stub, the netmap's
//! decision on every address it may reach, and `dns.overrideLocal`.
use super::*;

#[tokio::test]
async fn resolved_proxy_serves_only_names_the_os_resolves_and_only_for_sources() {
    let server = support::Server::start().await;
    let (mut control, script) = enrolled(&server).await;
    let mut network = Network::new(&control, 'e');
    network.include_mesh_resolver = false;
    let netmap = network.compile(None);
    assert!(matches!(
        netmap.netmap.dns.egress,
        DnsPlan::ServeExitResolver { .. }
    ));
    let snapshot = sync(&mut control, &script, reply(&netmap, true)).await;
    let proxy = |servers: &[&str]| SystemResolverSnapshot {
        id: "resolved-proxy-fixture".into(),
        upstreams: Arc::new(vec!["127.0.0.54:53".parse().unwrap()]),
        primary: DnsTransport::Udp,
        provenance: crate::system::Provenance::ResolvedProxy {
            routes: Arc::new(vec![
                "i00.meshway-installation.invalid".into(),
                "test.internal".into(),
            ]),
            servers: Arc::new(servers.iter().map(|s| s.parse().unwrap()).collect()),
        },
        invalid: CancellationToken::new(),
        verified_until: Arc::new(StdMutex::new(Instant::now() + Duration::from_secs(30))),
    };
    let mut owner = Owner::new(&snapshot, limits(), &CancellationToken::new()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let system = proxy(&["192.0.2.53"]);
    // resolved would route these to this Node's own link: its own records
    // and routes never answer for a source.
    for name in [
        "b.test.internal.",
        "TEST.Internal.",
        "x.i00.meshway-installation.invalid.",
    ] {
        assert!(matches!(
            owner.begin_served(named(name), Some(&system), deadline),
            Begin::Unavailable(Unavailable::MissingResolverPlan)
        ));
    }
    let Begin::Upstream(work) =
        owner.begin_served(named("nottest.internal."), Some(&system), deadline)
    else {
        panic!("a label-boundary name goes to the OS resolver")
    };
    let ResolverRoute::Local(route) = work.route() else {
        panic!("served upstreams are local routes")
    };
    assert_eq!(route.endpoint(), "127.0.0.54:53".parse().unwrap());
    assert!(work.fallback.is_some());
    // Every OS server resolved may pick needs this Node's local decision:
    // a Mesh address is never a system upstream.
    assert!(matches!(
        owner.begin_served(
            named("ordinary.example."),
            Some(&proxy(&["100.64.0.2"])),
            deadline
        ),
        Begin::Unavailable(Unavailable::MissingResolverPlan)
    ));
    // The proxy is for served queries only, never this Node's own names.
    assert!(matches!(
        owner.begin_system(named("ordinary.example."), &system, deadline),
        Begin::Unavailable(Unavailable::MissingResolverPlan)
    ));
    let _ = control.shutdown().await;
    server.close().await;
}

fn served_snapshot(upstream: &str, servers: Option<&[&str]>) -> SystemResolverSnapshot {
    SystemResolverSnapshot {
        id: format!("served-{upstream}"),
        upstreams: Arc::new(vec![upstream.parse().unwrap()]),
        primary: DnsTransport::Udp,
        provenance: match servers {
            None => crate::system::Provenance::Original,
            Some(servers) => crate::system::Provenance::ResolvedProxy {
                routes: Arc::new(vec!["test.internal".into()]),
                servers: Arc::new(servers.iter().map(|s| s.parse().unwrap()).collect()),
            },
        },
        invalid: CancellationToken::new(),
        verified_until: Arc::new(StdMutex::new(Instant::now() + Duration::from_secs(30))),
    }
}

#[tokio::test]
async fn unusable_served_upstreams_withdraw_only_served_queries() {
    use meshway_model::exit::ExitDnsUnavailableReason as Reason;
    let server = support::Server::start().await;
    let (mut control, script) = enrolled(&server).await;
    let mut network = Network::new(&control, 'e');
    network.include_mesh_resolver = false;
    let compiled = network.compile(None);
    assert!(matches!(
        compiled.netmap.dns.egress,
        DnsPlan::ServeExitResolver { .. }
    ));
    let snapshot = sync(&mut control, &script, reply(&compiled, true)).await;
    let netmap = snapshot.clone();
    let decide = |desired: &Desired, observed| {
        crate::exit_resolvers(desired, observed).map(|served| served.map(|s| s.id().to_owned()))
    };
    // Usable: the original nameserver, or resolved's stub with OS servers
    // this Node leaves to a local socket.
    assert_eq!(
        decide(&netmap, Ok(served_snapshot("192.0.2.53:53", None))),
        Ok(Some("served-192.0.2.53:53".into()))
    );
    assert!(
        decide(
            &netmap,
            Ok(served_snapshot("127.0.0.54:53", Some(&["192.0.2.53"])))
        )
        .is_ok()
    );
    // A Mesh address as the OS upstream, directly or behind resolved.
    assert_eq!(
        decide(&netmap, Ok(served_snapshot("100.64.0.2:53", None))),
        Err(Reason::MeshServer)
    );
    assert_eq!(
        decide(
            &netmap,
            Ok(served_snapshot(
                "127.0.0.54:53",
                Some(&["192.0.2.53", "100.64.0.2"])
            ))
        ),
        Err(Reason::MeshServer)
    );
    // An OS upstream the Node's egress partition sends through an exit: A's
    // partition (192.0.2/24 direct, the rest through E).
    let source = Network::new(&control, 'a');
    let routed = Desired::new(&Arc::new(source.compile(None)), &Default::default()).unwrap();
    assert_eq!(
        decide(&routed, Ok(served_snapshot("203.0.113.53:53", None))),
        Err(Reason::ExitRoutedServer)
    );
    assert_eq!(
        decide(
            &routed,
            Ok(served_snapshot("127.0.0.54:53", Some(&["203.0.113.53"])))
        ),
        Err(Reason::ExitRoutedServer)
    );
    assert_eq!(
        decide(&routed, Ok(served_snapshot("192.0.2.53:53", None))),
        Ok(Some("served-192.0.2.53:53".into()))
    );
    // The backend's own reason is kept.
    assert_eq!(
        decide(&netmap, Err(Reason::ProxyStubMissing)),
        Err(Reason::ProxyStubMissing)
    );
    // This Node's own DNS installs without a served snapshot; its sources'
    // queries then fail and never use this Node's own resolvers.
    let scope = Scope::netmap(snapshot.clone(), None).serving(None);
    scope.validate_resolvers().unwrap();
    let mut owner = Owner::new(&snapshot, limits(), &CancellationToken::new()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    assert!(matches!(
        owner.begin_served(named("ordinary.example."), None, deadline),
        Begin::Unavailable(Unavailable::MissingResolverPlan)
    ));
    assert!(matches!(
        owner.begin_published_until(named("x.direct.example."), deadline),
        Begin::Upstream(_)
    ));
    // A served snapshot belongs to an exit resolver's scope only.
    let mut other = server.control_named("source");
    server.enroll_peer(&control, &mut other).await;
    let other_script = Arc::new(Script::default());
    other.set_sync_transport(other_script.clone());
    let source = Network::new(&other, 'a').compile(None);
    let source = sync(&mut other, &other_script, reply(&source, true)).await;
    assert_eq!(
        Scope::netmap(source, None)
            .serving(Some(served_snapshot("192.0.2.53:53", None)))
            .validate_resolvers(),
        Err(DispatchError::Scope)
    );
    let _ = other.shutdown().await;
    let _ = control.shutdown().await;
    server.close().await;
}

#[tokio::test]
async fn override_local_serves_sources_from_the_configured_default_resolver() {
    use meshway_model::exit::ExitDnsUnavailableReason as Reason;
    let server = support::Server::start().await;
    let (mut control, script) = enrolled(&server).await;
    let mut network = Network::new(&control, 'e');
    network.include_mesh_resolver = false;
    network.override_local = true;
    let compiled = network.compile(None);
    assert!(matches!(
        compiled.netmap.dns.egress,
        DnsPlan::ServeExitResolver { .. }
    ));
    let snapshot = sync(&mut control, &script, reply(&compiled, true)).await;
    // No OS snapshot is used, whatever the backend observed.
    for observed in [
        Err(Reason::ProxyStubMissing),
        Err(Reason::MeshServer),
        Ok(served_snapshot("100.64.0.2:53", None)),
        Ok(served_snapshot("192.0.2.53:53", None)),
    ] {
        assert!(matches!(
            crate::exit_resolvers(&snapshot, observed),
            Ok(None)
        ));
    }
    Scope::netmap(snapshot.clone(), None)
        .serving(None)
        .validate_resolvers()
        .unwrap();
    let mut owner = Owner::new(&snapshot, limits(), &CancellationToken::new()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    // Every served name, including one resolved would route to this Node's
    // own link, goes to the default resolver on its own netmap route.
    for name in ["ordinary.example.", "x.direct.example.", "b.test.internal."] {
        let Begin::Upstream(work) = owner.begin_served(named(name), None, deadline) else {
            panic!("{name}: default resolver")
        };
        let ResolverRoute::Local(route) = work.route() else {
            panic!("a configured resolver is local")
        };
        assert_eq!(route.endpoint(), "192.0.2.53:53".parse().unwrap());
        assert!(work.fallback.is_none() && work.remaining.is_empty());
        assert!(owner.cancel(&work.ticket));
    }
    let _ = control.shutdown().await;
    server.close().await;
}
