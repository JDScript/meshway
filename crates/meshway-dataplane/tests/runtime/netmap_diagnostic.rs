//! `meshway ping` on netmap snapshots: decided locally with `icmpEcho`, run
//! on observable paths only, never waiting for a Server round trip.
use super::support::netmap::{NetmapMesh, rule, sel};
use meshway_dataplane::secure::{
    Applied, CircuitGrant, CircuitState, Error, PendingFrame, Received, Target, diagnostic_paths,
};
use meshway_model::{Digest, EventId};
use std::time::{Duration, Instant};

fn wire(frame: PendingFrame) -> Vec<u8> {
    frame.release(|bytes| bytes.into_vec()).unwrap()
}

fn measurement() -> EventId {
    EventId::new(super::support::request().into_uuid()).unwrap()
}

async fn ping(f: &mut NetmapMesh) -> [u8; 32] {
    let b = f.ids()[2];
    let init = f.a.start_diagnostic(b, None).unwrap().unwrap();
    let id = f.finish_handshake(init).await;
    let info = f.a.circuit_info(id).unwrap().unwrap();
    assert_eq!(info.id, id);
    assert_eq!(info.state, CircuitState::Established);
    assert_eq!(
        info.grant,
        CircuitGrant {
            purpose: meshway_dataplane::Purpose::Diagnostic,
            target: Target::Mesh { node: b },
        }
    );
    id
}

fn probe(f: &mut NetmapMesh, id: [u8; 32]) -> Result<(), Error> {
    let mid = measurement();
    let challenge = Digest::from_bytes([7; 32]);
    let request = f.a.probe(
        id,
        mid,
        challenge,
        Instant::now() + Duration::from_secs(5),
        &f.ar.carrier.clone(),
    )?;
    let Received::Forward(request) = f.r.receive(&f.ar.carrier, wire(request))? else {
        panic!("relay")
    };
    let Received::ProbeRequest { response, .. } = f.b.receive(&f.rb.carrier, wire(request))? else {
        panic!("terminal probe")
    };
    let Received::Forward(response) = f.r.receive(&f.rb.carrier, wire(response))? else {
        panic!("relay")
    };
    let Received::ProbeReply {
        circuit,
        measurement_id,
        challenge: returned,
    } = f.a.receive(&f.ar.carrier, wire(response))?
    else {
        panic!("probe reply")
    };
    assert_eq!((circuit, measurement_id, returned), (id, mid, challenge));
    Ok(())
}

#[tokio::test]
async fn netmap_ping_is_local_and_runs_on_the_observable_path() {
    let mut f = NetmapMesh::start().await;
    let calls = f.sync_calls();
    let [_, _, b] = f.ids();
    let desired = f.nodes[0].desired();
    assert_eq!(diagnostic_paths(&desired, b, None).unwrap(), vec![f.path()]);
    let id = ping(&mut f).await;
    probe(&mut f, id).unwrap();
    // Diagnostic circuits never carry business packets.
    let packet = f.mesh_udp(false, 42424);
    assert!(f.a.send(id, packet).is_err());
    // Same-content renewal keeps the diagnostic circuit and its cipher.
    assert_eq!(f.renew().await, [Applied::default(); 3]);
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    probe(&mut f, id).unwrap();
    assert_eq!(
        f.sync_calls(),
        calls + 3,
        "only the explicit renewals called Control"
    );
    f.close().await;
}

#[tokio::test]
async fn netmap_ping_denied_without_icmp_and_revocation_ends_the_circuit() {
    let mut f = NetmapMesh::start().await;
    let id = ping(&mut f).await;
    let updates = f
        .publish(|cfg, [a, _, b]| cfg.access[0] = rule(a, &sel(b), &["udp:42424"]))
        .await;
    for update in [updates[0], updates[2]] {
        assert!(
            matches!(
                update,
                Applied {
                    retired_circuits: 1,
                    ..
                }
            ),
            "{update:?}"
        );
    }
    assert!(f.a.circuit_info(id).unwrap().is_none());
    let b = f.ids()[2];
    assert!(matches!(f.a.start_diagnostic(b, None), Err(Error::Policy)));
    let desired = f.nodes[0].desired();
    assert!(matches!(
        diagnostic_paths(&desired, b, None),
        Err(Error::Policy)
    ));
    f.close().await;
}

#[tokio::test]
async fn netmap_ping_without_observable_path_is_unsupported_not_denied() {
    let mut f = NetmapMesh::start().await;
    let r = f.ids()[1];
    f.inventory
        .get_mut(&r)
        .unwrap()
        .capabilities
        .remove("feature:path-observation");
    f.publish(|_, _| {}).await;
    let b = f.ids()[2];
    assert!(matches!(
        f.a.start_diagnostic(b, None),
        Err(Error::Unsupported)
    ));
    let desired = f.nodes[0].desired();
    assert!(matches!(
        diagnostic_paths(&desired, b, None),
        Err(Error::Unsupported)
    ));
    // An explicit path outside the decision is denied, not unsupported.
    let [a, _, b] = f.ids();
    assert!(matches!(
        diagnostic_paths(&desired, b, Some(&[a, b])),
        Err(Error::Policy)
    ));
    f.close().await;
}

/// The decision `meshway ping` prints: the action of A's own first-packet
/// decision, the deciding rule with its remark, the profile and the path.
#[tokio::test]
async fn ping_decision_names_the_rule_profile_action_and_path() {
    use meshway_configuration::execution::UnavailableReason;
    use meshway_dataplane::diagnostics::decision::decide;
    use meshway_protocol::ipc::PingAction;
    let mut f = NetmapMesh::with(super::support::netmap::DAY_MS, |cfg, [a, _, _]| {
        cfg.access.push(rule(a, "8.8.8.0/24", &["icmp:*"]));
        let profile = cfg.routing.egress.profiles.get_mut("exit").unwrap();
        profile.rules.push(meshway_configuration::EgressRule {
            remark: "lan direct".into(),
            matcher: meshway_configuration::RouteMatch {
                ip_cidr: vec!["192.168.7.0/24".into()],
                ..Default::default()
            },
            action: meshway_configuration::RuleAction::Direct,
        });
    })
    .await;
    let [a, r, b] = f.ids();
    f.a.set_ready_exits([b].into(), Default::default());
    let now = 0;
    let exit = decide(&mut f.a, "8.8.8.8".parse().unwrap(), None, now);
    assert_eq!(exit.action, Some(PingAction::Exit { node: b }));
    assert_eq!(exit.path, [a, r, b]);
    assert_eq!(exit.profile.as_deref(), Some("exit"));
    assert_eq!(exit.rule, None, "the profile fallback decides");
    assert!(exit.node.is_none() && exit.resolution.is_none());
    let direct = decide(&mut f.a, "192.168.7.9".parse().unwrap(), None, now);
    assert_eq!(direct.action, Some(PingAction::Direct));
    assert!(direct.path.is_empty());
    let rule = direct.rule.unwrap();
    assert_eq!(
        (rule.pointer.as_str(), rule.remark.as_str()),
        ("/routing/egress/profiles/exit/rules/0", "lan direct")
    );
    // The Internet is granted for UDP only: echo is denied, never probed.
    let denied = decide(&mut f.a, "1.1.1.1".parse().unwrap(), None, now);
    assert_eq!(
        denied.action,
        Some(PingAction::Denied {
            reason: UnavailableReason::Unauthorized
        })
    );
    // A Node's Mesh address is a Mesh decision without an egress rule.
    let address = f.address(2);
    let mesh = decide(&mut f.a, address, None, now);
    assert_eq!(mesh.action, Some(PingAction::Mesh));
    assert_eq!(mesh.node.map(|node| node.node_id), Some(b));
    assert_eq!((mesh.profile, mesh.rule), (None, None));
    assert_eq!(mesh.path, [a, r, b]);
    f.close().await;
}
