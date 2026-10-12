use super::*;
use std::{future::pending, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Packets,
    Adjacencies,
    Host,
    Dns,
}
#[derive(Default)]
struct Resources {
    calls: Vec<(Part, Option<NetmapRef>)>,
    fail: Option<Part>,
    block: Option<Part>,
}
impl Resources {
    async fn apply(&mut self, part: Part, desired: Option<&Desired>) -> Result<(), Error> {
        self.calls.push((part, desired.map(Desired::reference)));
        if desired.is_some() && self.block == Some(part) {
            pending::<()>().await;
        }
        if desired.is_some() && self.fail == Some(part) {
            Err(Error::Apply)
        } else {
            Ok(())
        }
    }
}
impl Components for Resources {
    async fn apply_packets(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        self.apply(Part::Packets, desired).await
    }
    async fn apply_adjacencies(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        self.apply(Part::Adjacencies, desired).await
    }
    async fn apply_host(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        self.apply(Part::Host, desired).await
    }
    async fn apply_dns(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        self.apply(Part::Dns, desired).await
    }
}
fn input(
    engine: &mut Engine<Resources>,
    content: Arc<CompiledNetmap>,
    milliseconds: u64,
) -> Netmap {
    let now = engine.clock.now();
    Netmap {
        lease: NetmapLease::new(content.reference(), now, now + milliseconds).unwrap(),
        content,
        artifacts: Default::default(),
        associations: None,
    }
}

#[tokio::test(start_paused = true)]
async fn renewal_changes_only_engine_timer_and_expiry_withdraws_every_component() {
    let mut engine = Engine::new(Resources::default());
    let content = Arc::new(meshway_test_server::single_node("a", None));
    let first = input(&mut engine, content.clone(), 60_000);
    engine.reconfig(Some(&first)).await.unwrap();
    let compiler = engine.desired().unwrap().compiler().clone();
    let before = engine.next_expiry().unwrap();
    let calls = engine.components.calls.clone();
    tokio::time::advance(Duration::from_secs(20)).await;
    let renewed = input(&mut engine, content, 60_000);
    engine.reconfig(Some(&renewed)).await.unwrap();
    assert!(engine.next_expiry().unwrap() > before);
    assert!(Arc::ptr_eq(&compiler, engine.desired().unwrap().compiler()));
    assert_eq!(engine.components.calls, calls);
    tokio::time::advance(Duration::from_secs(61)).await;
    engine.expire().await.unwrap();
    assert!(engine.desired().is_none() && engine.next_expiry().is_none());
    assert_eq!(
        &engine.components.calls[calls.len()..],
        &[
            (Part::Packets, None),
            (Part::Adjacencies, None),
            (Part::Host, None),
            (Part::Dns, None),
        ]
    );
    // A fresh signed lease re-installs the same content after actual expiry.
    let restored = input(&mut engine, renewed.content, 60_000);
    engine.reconfig(Some(&restored)).await.unwrap();
    assert!(engine.desired().is_some());
    assert!(engine.components.calls.ends_with(&[
        (Part::Host, Some(restored.content.reference())),
        (Part::Adjacencies, Some(restored.content.reference())),
        (Part::Packets, Some(restored.content.reference())),
        (Part::Dns, Some(restored.content.reference())),
    ]));
}

#[tokio::test(start_paused = true)]
async fn a_newer_offer_that_cannot_be_admitted_ends_the_older_content() {
    let mut engine = Engine::new(Resources::default());
    let content = Arc::new(meshway_test_server::single_node("a", None));
    let first = input(&mut engine, content.clone(), 60_000);
    engine.reconfig(Some(&first)).await.unwrap();
    // The newer serial's lease has already ended on this Engine's clock: the
    // older content must not keep running in its place.
    let newer = Arc::new(meshway_test_server::single_node("b", Some(&content)));
    let now = engine.clock.now();
    let ended = Netmap {
        lease: NetmapLease::new(newer.reference(), now - 120_000, now - 1).unwrap(),
        content: newer,
        artifacts: Default::default(),
        associations: None,
    };
    assert_eq!(engine.reconfig(Some(&ended)).await, Err(Error::Lease));
    assert!(engine.desired().is_none() && engine.next_expiry().is_none());
    assert!(engine.components.calls.ends_with(&[
        (Part::Packets, None),
        (Part::Adjacencies, None),
        (Part::Host, None),
        (Part::Dns, None),
    ]));
    // The original serial cannot come back after its successor was seen.
    let again = input(&mut engine, content, 60_000);
    assert_eq!(engine.reconfig(Some(&again)).await, Err(Error::Reference));
    assert!(engine.desired().is_none());
}

#[tokio::test(start_paused = true)]
async fn failed_or_cancelled_apply_never_exposes_old_desired_as_current() {
    let content = Arc::new(meshway_test_server::single_node("a", None));
    for part in [Part::Packets, Part::Adjacencies, Part::Host, Part::Dns] {
        let mut engine = Engine::new(Resources {
            fail: Some(part),
            ..Default::default()
        });
        let first = input(&mut engine, content.clone(), 60_000);
        assert_eq!(engine.reconfig(Some(&first)).await, Err(Error::Apply));
        assert!(engine.desired().is_none());
        assert!(engine.components.calls.ends_with(&[
            (Part::Packets, None),
            (Part::Adjacencies, None),
            (Part::Host, None),
            (Part::Dns, None),
        ]));
    }
    let mut engine = Engine::new(Resources::default());
    let first = input(&mut engine, content.clone(), 60_000);
    engine.reconfig(Some(&first)).await.unwrap();
    let next = input(
        &mut engine,
        Arc::new(meshway_test_server::single_node("b", Some(&content))),
        60_000,
    );
    engine.components.block = Some(Part::Host);
    assert!(
        tokio::time::timeout(Duration::from_secs(1), engine.reconfig(Some(&next)))
            .await
            .is_err()
    );
    assert!(engine.desired().is_none() && engine.next_expiry().is_none());
    engine.components.block = None;
    engine.reconfig(None).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn older_signed_lease_and_regressing_content_do_not_change_live_engine() {
    let mut engine = Engine::new(Resources::default());
    let content = Arc::new(meshway_test_server::single_node("a", None));
    let first = input(&mut engine, content, 60_000);
    engine.reconfig(Some(&first)).await.unwrap();
    let calls = engine.components.calls.clone();
    let expires = engine.next_expiry();
    let mut older = Netmap {
        content: first.content.clone(),
        lease: first.lease,
        artifacts: Default::default(),
        associations: None,
    };
    older.lease.expires_at_ms -= 1;
    assert_eq!(engine.reconfig(Some(&older)).await, Err(Error::Reference));
    assert_eq!(engine.next_expiry(), expires);
    assert_eq!(engine.components.calls, calls);
    // A Server clock stepped back issues an earlier `issuedAt`; a lease that
    // does not end earlier is still a renewal.
    let mut stepped = Netmap {
        content: first.content.clone(),
        lease: first.lease,
        artifacts: Default::default(),
        associations: None,
    };
    stepped.lease.issued_at_ms -= 5_000;
    stepped.lease.expires_at_ms += 1_000;
    engine.reconfig(Some(&stepped)).await.unwrap();
    assert!(engine.next_expiry() > expires);
    assert_eq!(engine.components.calls, calls);
}

#[derive(Default)]
struct Degrading {
    calls: Vec<(Part, Option<NetmapRef>)>,
    degraded: bool,
}
impl Degrading {
    fn apply(&mut self, part: Part, desired: Option<&Desired>) -> Result<(), Error> {
        self.calls.push((part, desired.map(Desired::reference)));
        if desired.is_some() && part == Part::Adjacencies && self.degraded {
            Err(Error::Degraded)
        } else {
            Ok(())
        }
    }
}
impl Components for Degrading {
    async fn apply_packets(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        self.apply(Part::Packets, desired)
    }
    async fn apply_adjacencies(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        self.apply(Part::Adjacencies, desired)
    }
    async fn apply_host(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        self.apply(Part::Host, desired)
    }
    async fn apply_dns(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        self.apply(Part::Dns, desired)
    }
}

#[tokio::test(start_paused = true)]
async fn a_degraded_apply_keeps_content_and_the_same_content_retries_it() {
    let mut engine = Engine::new(Degrading {
        degraded: true,
        ..Default::default()
    });
    let content = Arc::new(meshway_test_server::single_node("a", None));
    let now = engine.clock.now();
    let first = Netmap {
        lease: NetmapLease::new(content.reference(), now, now + 60_000).unwrap(),
        content,
        artifacts: Default::default(),
        associations: None,
    };
    assert_eq!(engine.reconfig(Some(&first)).await, Err(Error::Degraded));
    let reference = Some(first.content.reference());
    assert!(engine.degraded() && engine.next_expiry().is_some());
    assert_eq!(engine.desired().map(Desired::reference), reference);
    // Every step ran on the content; nothing was withdrawn.
    assert_eq!(
        engine.components.calls,
        [Part::Host, Part::Adjacencies, Part::Packets, Part::Dns].map(|part| (part, reference))
    );
    engine.components.calls.clear();
    engine.components.degraded = false;
    engine.reconfig(Some(&first)).await.unwrap();
    assert!(!engine.degraded());
    assert_eq!(
        engine.components.calls,
        [Part::Host, Part::Adjacencies, Part::Packets, Part::Dns].map(|part| (part, reference))
    );
    engine.components.calls.clear();
    engine.reconfig(Some(&first)).await.unwrap();
    assert!(
        engine.components.calls.is_empty(),
        "healthy renewal is a no-op"
    );
}

#[tokio::test(start_paused = true)]
async fn a_raised_clock_floor_never_lets_an_ended_lease_run() {
    let mut engine = Engine::new(Resources::default());
    let content = Arc::new(meshway_test_server::single_node("a", None));
    let netmap = input(&mut engine, content, 60_000);
    // A time observed before a restart lies past the lease end although the
    // wall clock was set back: the lease is over.
    engine.raise_clock_floor(netmap.lease.expires_at_ms + 1);
    assert_eq!(engine.reconfig(Some(&netmap)).await, Err(Error::Lease));
    assert!(engine.desired().is_none());
}

/// A one-Node netmap whose public egress rule names a rule set, and the set.
fn with_rule_set() -> (Arc<CompiledNetmap>, Arc<meshway_rules::RuleSet>) {
    use meshway_test_server::configuration as fixture;
    let set = Arc::new(
        meshway_rules::RuleSet::from_inline(&meshway_rules::inline::Inline {
            ip_cidr: &["9.9.9.0/24".to_owned()],
            ..Default::default()
        })
        .unwrap(),
    );
    let node = fixture::node(1);
    let mut cfg = fixture::network("fixture.internal");
    cfg.nodes
        .insert(node, fixture::tag_node("a", 1, BTreeMap::new(), false));
    cfg.routing.egress.rules = vec![meshway_configuration::EgressRule {
        remark: String::new(),
        matcher: meshway_configuration::RouteMatch {
            rule_sets: vec!["corp".into()],
            ..Default::default()
        },
        action: meshway_configuration::RuleAction::Direct,
    }];
    let sets: meshway_policy::compile::RuleSets = [("corp".to_owned(), set.clone())].into();
    let compiled = meshway_policy::compile::netmap::compile_netmap(
        meshway_policy::compile::netmap::NetmapInput {
            network: "00000000-0000-4003-8000-000000000001".parse().unwrap(),
            revision: meshway_model::Revision::MIN,
            configuration: &cfg,
            subjects: &Default::default(),
            inventory: &BTreeMap::from([(node, fixture::trusted(1, false))]),
            rule_sets: &sets,
            selected_profiles: &BTreeMap::new(),
            selection_versions: &BTreeMap::new(),
            previous: &BTreeMap::new(),
        },
    )
    .unwrap();
    (Arc::new(compiled.nodes[&node].clone()), set)
}

#[tokio::test(start_paused = true)]
async fn content_applies_before_its_artifacts_and_again_once_they_load() {
    let mut engine = Engine::new(Resources::default());
    let (content, set) = with_rule_set();
    let first = input(&mut engine, content.clone(), 60_000);
    engine.reconfig(Some(&first)).await.unwrap();
    let desired = engine.desired().unwrap();
    assert_eq!(desired.missing(), [set.digest()]);
    // Fail closed until the artifact is held: full capture, no direct route.
    let capture = &desired.egress().unwrap().capture;
    assert_eq!(
        capture.unavailable,
        Some(meshway_configuration::execution::UnavailableReason::MissingDependency)
    );
    assert!(capture.direct_ipv4.is_empty());
    assert_eq!(
        desired
            .content()
            .netmap
            .egress
            .as_ref()
            .unwrap()
            .capture
            .unavailable,
        None
    );
    let applied = engine.components.calls.len();
    // The same content and lease with the artifact loaded is applied again.
    let loaded = Netmap {
        artifacts: Arc::new([(set.digest(), set.clone())].into()),
        associations: None,
        ..first.clone()
    };
    engine.reconfig(Some(&loaded)).await.unwrap();
    assert!(engine.components.calls.len() > applied);
    let desired = engine.desired().unwrap();
    assert!(desired.missing().is_empty());
    assert!(Arc::ptr_eq(desired.netmap(), &content.netmap));
    // Offering it once more changes nothing.
    let calls = engine.components.calls.len();
    engine.reconfig(Some(&loaded)).await.unwrap();
    assert_eq!(engine.components.calls.len(), calls);
}

/// One-Node netmap whose public rule sends `chat.example` direct (the rest
/// unmanaged), and an overlay associating one public address with it.
fn with_association(
    expires_at_ms: u64,
) -> (
    Arc<CompiledNetmap>,
    Arc<meshway_configuration::netmap::DnsAssociationOverlay>,
) {
    use meshway_configuration::netmap::{
        AssociationRecordType, DnsAssociation, DnsAssociationOverlay,
    };
    use meshway_test_server::configuration as fixture;
    let node = fixture::node(1);
    let mut cfg = fixture::network("fixture.internal");
    cfg.nodes
        .insert(node, fixture::tag_node("a", 1, BTreeMap::new(), false));
    cfg.routing.egress.rules = vec![meshway_configuration::EgressRule {
        remark: String::new(),
        matcher: meshway_configuration::RouteMatch {
            domain: vec!["chat.example".into()],
            ..Default::default()
        },
        action: meshway_configuration::RuleAction::Direct,
    }];
    let compiled = meshway_policy::compile::netmap::compile_netmap(
        meshway_policy::compile::netmap::NetmapInput {
            network: "00000000-0000-4003-8000-000000000001".parse().unwrap(),
            revision: meshway_model::Revision::MIN,
            configuration: &cfg,
            subjects: &Default::default(),
            inventory: &BTreeMap::from([(node, fixture::trusted(1, false))]),
            rule_sets: &Default::default(),
            selected_profiles: &BTreeMap::new(),
            selection_versions: &BTreeMap::new(),
            previous: &BTreeMap::new(),
        },
    )
    .unwrap();
    let content = Arc::new(compiled.nodes[&node].clone());
    let overlay = DnsAssociationOverlay {
        netmap: content.reference(),
        associations: vec![DnsAssociation {
            source: node,
            generation: content.netmap.egress.as_ref().unwrap().capture.generation,
            name: "chat.example".into(),
            record_type: AssociationRecordType::A,
            cnames: vec![],
            addresses: vec![meshway_configuration::execution::ExecutionAddress(
                "203.0.113.9".parse().unwrap(),
            )],
            expires_at_ms,
        }],
    };
    (content, Arc::new(overlay))
}

#[tokio::test(start_paused = true)]
async fn associations_apply_without_new_content_and_end_on_their_own_deadline() {
    let mut engine = Engine::new(Resources::default());
    let now = engine.clock.now();
    let (content, overlay) = with_association(now + 30_000);
    let plain = input(&mut engine, content.clone(), 3_600_000);
    engine.reconfig(Some(&plain)).await.unwrap();
    let before = engine.desired().unwrap().clone();
    assert!(before.associations().is_empty());
    let applied = engine.components.calls.len();
    let associated = Netmap {
        associations: Some(overlay.clone()),
        ..plain.clone()
    };
    engine.reconfig(Some(&associated)).await.unwrap();
    let desired = engine.desired().unwrap().clone();
    assert!(desired.same_content(&before) && !desired.same(&before));
    assert_eq!(desired.associations().entries().len(), 1);
    assert!(
        engine.components.calls.len() > applied,
        "every component sees it"
    );
    let capture = &desired.egress().unwrap().capture;
    assert!(capture.direct_ipv4.contains(&"203.0.113.9/32".to_owned()));
    // The signed content itself is unchanged.
    assert_eq!(
        content.netmap.egress.as_ref().unwrap().capture.direct_ipv4,
        Vec::<String>::new()
    );
    // An overlay of other content is ignored.
    let mut foreign = (*overlay).clone();
    foreign.netmap.serial = foreign.netmap.serial.checked_next().unwrap();
    let ignored = desired
        .with_associations(Some(&Arc::new(foreign)), now)
        .unwrap();
    assert!(ignored.associations().is_empty());
    // The association deadline is scheduled before the lease's.
    let at = engine.next_expiry().unwrap();
    assert!(at <= tokio::time::Instant::now() + Duration::from_secs(31));
    tokio::time::advance(Duration::from_secs(31)).await;
    let calls = engine.components.calls.len();
    engine.expire().await.unwrap();
    let pruned = engine.desired().unwrap();
    assert!(pruned.associations().is_empty() && pruned.same_content(&desired));
    assert!(engine.components.calls.len() > calls);
    assert!(
        !pruned
            .egress()
            .unwrap()
            .capture
            .direct_ipv4
            .contains(&"203.0.113.9/32".to_owned())
    );
}
