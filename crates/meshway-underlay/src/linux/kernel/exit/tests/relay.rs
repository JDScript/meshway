use super::*;
use meshway_control::Control;
use meshway_model::state::{
    Capability, ConfigurationState, NodeState, StreamKind, SystemState, TransportRole,
};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Seed {
    origin: String,
    keys: Vec<String>,
}

#[tokio::test]
#[ignore = "requires isolated real Server and generated-SDK configuration driver"]
async fn real_control_external_permit_installs_transit_without_exit_writer() {
    run(None).await;
}

pub(super) async fn run(case: Option<super::handoff::Case>) {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let seed: Seed =
        serde_json::from_slice(&std::fs::read("/root/x3c-seed.json").unwrap()).unwrap();
    assert_eq!(seed.keys.len(), 3);
    let temp = tempfile::tempdir().unwrap();
    let directory = Arc::new(Directory::open(&temp.path().join("underlay"), "test.lock").unwrap());
    let mut host = crate::Host::open(directory, Default::default())
        .await
        .unwrap();
    host.attach_control(&seed.origin).await.unwrap();
    let mut controls = Vec::new();
    let mut nodes = Vec::new();
    for (index, key) in seed.keys.into_iter().enumerate() {
        let mut control = Control::open(
            &temp.path().join(format!("control-{index}")),
            &seed.origin,
            host.client_builder().unwrap(),
        )
        .unwrap();
        let key = serde_json::from_value(serde_json::json!(key)).unwrap();
        let binding = control
            .enroll_with_key(["a", "r", "e"][index], key)
            .await
            .unwrap();
        nodes.push(binding.node_id);
        controls.push(control);
    }
    std::fs::write("/root/x3c-nodes.json", serde_json::to_vec(&nodes).unwrap()).unwrap();
    tokio::time::timeout(Duration::from_secs(45), async {
        while !std::path::Path::new("/root/x3c-configured").exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let capabilities = BTreeMap::from([
        (
            "transport:native_tcp".into(),
            Capability::Transport {
                roles: vec![TransportRole::Dial, TransportRole::Listen],
                streams: vec![StreamKind::Tcp],
            },
        ),
        (
            meshway_model::exit::EXIT_IPV4_FEATURE.into(),
            Capability::Feature {
                version: meshway_model::exit::EXIT_IPV4_VERSION,
            },
        ),
    ]);
    for control in &mut controls {
        control.authenticate().await.unwrap();
        control
            .acquire_instance(capabilities.clone())
            .await
            .unwrap();
        control.sync_configuration().await.unwrap();
        control.acknowledge_saved().await.unwrap();
    }
    let e_snapshot = controls[2].execution_snapshot().unwrap();
    let capability = host
        .probe_exit(e_snapshot.instance().instance_id, e_snapshot.reference())
        .await;
    assert_eq!(capability.status, meshway_model::exit::ExitStatus::Ready);
    let observed = capability.observed_at;
    controls[2]
        .report_runtime_state(
            NodeState {
                system: SystemState {
                    hostname: "e".into(),
                    os: "linux".into(),
                    arch: "x86_64".into(),
                    agent_version: "x3c-fixture".into(),
                },
                capabilities,
                configuration: ConfigurationState::default(),
                exit_capability: Some(capability),
                listeners: BTreeMap::new(),
                sockets: BTreeMap::new(),
                links: BTreeMap::new(),
                paths: BTreeMap::new(),
                selection: Default::default(),
                probes: Vec::new(),
                problems: Vec::new(),
            },
            observed,
        )
        .await
        .unwrap();
    let instance = controls[0].execution_snapshot().unwrap().instance().clone();
    let demand = serde_json::from_value(serde_json::json!({
        "instance": instance, "requestId": "44444444-4444-4444-8444-444444444444",
        "target": {"type": "ip", "address": "198.18.71.11"}, "protocol": "udp", "port": 42424, "profile": "mesh", "durationSeconds": if case.is_some() { 30 } else { 60 }
    })).unwrap();
    let feedback = controls[0].create_demand(&demand).await.unwrap();
    assert!(
        matches!(
            feedback.decision,
            meshway_configuration::execution::DemandDecision::Permitted { .. }
        ),
        "{feedback:?}"
    );
    controls[1].refresh_configuration().await.unwrap();
    controls[1].acknowledge_saved().await.unwrap();
    let snapshot = controls[1].execution_snapshot().unwrap();
    let cfg = snapshot.configuration().unwrap();
    assert!(cfg.exit_role.is_none());
    assert!(
        cfg.permits
            .values()
            .any(|permit| permit.exit.is_some()
                && permit.paths.iter().any(|path| path.nodes == nodes))
    );
    host.install(snapshot.clone()).await.unwrap();
    assert!(host.exit_role().is_none());
    let exit_plan = e_snapshot
        .configuration()
        .unwrap()
        .exit_role
        .clone()
        .unwrap();
    assert!(host.install_exit_role(exit_plan).await.is_err());
    assert!(host.kernel.tun.is_some());
    host.kernel.verify().await.unwrap();
    host.down().await.unwrap();
    controls[2].refresh_configuration().await.unwrap();
    controls[2].acknowledge_saved().await.unwrap();
    let current = controls[2].execution_snapshot().unwrap();
    host.install(current.clone()).await.unwrap();
    let role = host
        .install_exit_role(current.configuration().unwrap().exit_role.clone().unwrap())
        .await
        .unwrap();
    let probe = host
        .probe_exit(current.instance().instance_id, current.reference())
        .await;
    assert_eq!(
        probe.status,
        meshway_model::exit::ExitStatus::Ready,
        "owned capture/exit rules must not become foreign firewall conflicts"
    );
    if let Some(case) = case {
        super::handoff::exercise(case, &mut host, &mut controls, &role, &current).await;
    } else {
        guarded_writer(&role, &current).await;
    }
    let unknown = matches!(case, Some(super::handoff::Case::UnknownCleanup));
    assert_eq!(role.retire().await.is_err(), unknown);
    for control in controls {
        control.shutdown().await.unwrap();
    }
    assert_eq!(host.shutdown(false).await.is_err(), unknown);
    println!(
        "X3C_RELAY=real-server-real-control,external-permit,full-A-R-E-path,transit-install,no-exit-role,no-exit-writer"
    );
}

struct PacketGuard {
    revoked: Arc<std::sync::atomic::AtomicBool>,
    checked: Arc<std::sync::atomic::AtomicUsize>,
}
impl ExitSendGuard for PacketGuard {
    fn check(&self) -> Result<(), Invalidated> {
        use std::sync::atomic::Ordering;
        self.checked.fetch_add(1, Ordering::SeqCst);
        if self.revoked.load(Ordering::SeqCst) {
            Err(Invalidated)
        } else {
            Ok(())
        }
    }
}
async fn guarded_writer(role: &ExitRole, snapshot: &meshway_authority::ExecutionSnapshot) {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let permit = snapshot
        .configuration()
        .unwrap()
        .permits
        .values()
        .find(|permit| permit.exit.is_some())
        .unwrap();
    let tuple = OriginalTuple {
        source: permit.source_addresses.ipv4.unwrap(),
        destination: "198.18.71.11".parse().unwrap(),
        transport: ExitTransport::Udp {
            source_port: 17424,
            destination_port: 42424,
        },
    };
    let clock = Arc::new(Clock::new().unwrap());
    let scope = ExitScope {
        binding: Digest::from_bytes([71; 32]),
        revoked: CancellationToken::new(),
    };
    let incarnation = id(220);
    role.install(incarnation, tuple, deadline(&clock, 10_000), scope.clone())
        .await
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    role.send(
        incarnation,
        udp(tuple, b"guarded-first"),
        PacketGuard {
            revoked: Arc::new(AtomicBool::new(false)),
            checked: calls.clone(),
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        role.confirm(incarnation).await.unwrap(),
        CtConfirmation::Confirmed { .. }
    ));
    let mut reader = role.reply_reader().unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(2), reader.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply.incarnation, incarnation);
    assert_eq!(&reply.packet[28..], b"from=198.18.71.20|guarded-first");
    let revoked = Arc::new(AtomicBool::new(false));
    let denied_calls = Arc::new(AtomicUsize::new(0));
    let mut queued = Box::pin(role.send(
        incarnation,
        udp(tuple, b"must-not-leave"),
        PacketGuard {
            revoked: revoked.clone(),
            checked: denied_calls.clone(),
        },
    ));
    assert!(futures_util::poll!(queued.as_mut()).is_pending());
    revoked.store(true, Ordering::SeqCst);
    role.renew(incarnation, deadline(&clock, 20_000), scope)
        .await
        .unwrap();
    assert!(queued.await.is_err());
    assert!(denied_calls.load(Ordering::SeqCst) > 0);
    assert!(calls.load(Ordering::SeqCst) > 0);
    role.retire_flow(incarnation).await.unwrap();
    println!(
        "X3C_PUBLIC_GUARD=real-control-role-and-capture,guarded-roundtrip,queued-old-guard-rejected-across-renewal"
    );
}
