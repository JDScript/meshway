use super::{Runtime, testing::Fixture};
use meshway_dataplane::{
    Packet, PendingAction,
    exit::ExitReplyIngress,
    secure::{Carrier, Received, SecureNode},
};
use meshway_underlay::Host;
use std::{collections::BTreeMap, time::Duration};

fn isolated() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let _ = meshway_logging::Options {
        filter: "meshway_daemon=debug,meshway_underlay=debug".into(),
        format: meshway_logging::Format::Text,
    }
    .init();
}

struct Mesh {
    a: SecureNode,
    r: SecureNode,
    e: SecureNode,
    ar: Carrier,
    re: Carrier,
    circuit: [u8; 32],
}
impl Mesh {
    fn new(f: &Fixture, demand: meshway_model::RequestId) -> Self {
        let ids = f.nodes();
        let carrier = |a, b| Carrier {
            dialer: ids[a],
            listener: ids[b],
            listener_key: "udp".into(),
            protocol: meshway_configuration::transport::TransportProtocol::NativeUdp,
        };
        let ar = carrier(0, 1);
        let re = carrier(1, 2);
        let node = |i: usize, carriers| {
            SecureNode::new(
                f.controls[i]
                    .dataplane_identity(f.controls[i].execution_snapshot().unwrap())
                    .unwrap(),
                Default::default(),
                carriers,
            )
            .unwrap()
        };
        let mut a = node(0, BTreeMap::from([(ids[1], ar.clone())]));
        let mut r = node(
            1,
            BTreeMap::from([(ids[0], ar.clone()), (ids[2], re.clone())]),
        );
        let mut e = node(2, BTreeMap::from([(ids[1], re.clone())]));
        let frame = a.start(demand, ids).unwrap();
        let circuit = frame.circuit();
        let Received::Forward(frame) = r.receive(&ar, frame.release(|b| b).unwrap()).unwrap()
        else {
            panic!("relay handshake")
        };
        let Received::Established {
            response: Some(frame),
            ..
        } = e.receive(&re, frame.release(|b| b).unwrap()).unwrap()
        else {
            panic!("exit handshake")
        };
        let Received::Forward(frame) = r.receive(&re, frame.release(|b| b).unwrap()).unwrap()
        else {
            panic!("relay handshake response")
        };
        assert!(matches!(
            a.receive(&ar, frame.release(|b| b).unwrap()).unwrap(),
            Received::Established { .. }
        ));
        Self {
            a,
            r,
            e,
            ar,
            re,
            circuit,
        }
    }
    fn forward(&mut self, packet: Packet) -> PendingAction {
        let frame = self.a.send(self.circuit, packet).unwrap();
        let Received::Forward(frame) = self
            .r
            .receive(&self.ar, frame.release(|b| b).unwrap())
            .unwrap()
        else {
            panic!("opaque relay")
        };
        let Received::Packet(packet) = self
            .e
            .receive(&self.re, frame.release(|b| b).unwrap())
            .unwrap()
        else {
            panic!("exit packet")
        };
        assert_eq!(self.r.retained_flows(), 0);
        packet
    }
    async fn reply(&mut self, runtime: &mut Runtime) {
        let reply = tokio::time::timeout(Duration::from_secs(10), runtime.reader.recv())
            .await
            .unwrap()
            .unwrap();
        let frame = self
            .e
            .exit_reply(ExitReplyIngress::new(Packet::parse(reply.packet).unwrap()))
            .unwrap();
        runtime.drive(&mut self.e);
        frame.projection_ready().await.unwrap();
        let Received::Forward(frame) = self
            .r
            .receive(&self.re, frame.release(|b| b).unwrap())
            .unwrap()
        else {
            panic!("original reverse path")
        };
        let Received::Packet(packet) = self
            .a
            .receive(&self.ar, frame.release(|b| b).unwrap())
            .unwrap()
        else {
            panic!("source reply")
        };
        packet
            .release(|action| match action {
                meshway_dataplane::Action::Deliver { packet, .. } => {
                    assert!(packet.as_bytes().windows(7).any(|b| b == b"x4-exit"))
                }
                _ => panic!("source delivery"),
            })
            .unwrap();
    }
    fn reconcile(&mut self, f: &Fixture) {
        for (i, node) in [&mut self.a, &mut self.r, &mut self.e]
            .into_iter()
            .enumerate()
        {
            node.reconcile(
                f.controls[i]
                    .dataplane_identity(f.controls[i].execution_snapshot().unwrap())
                    .unwrap(),
            )
            .unwrap();
        }
    }
}

fn packet(f: &Fixture, port: u16) -> Packet {
    let snapshot = f.controls[0].execution_snapshot().unwrap();
    let source = snapshot.configuration().unwrap().addresses.ipv4.unwrap();
    let std::net::IpAddr::V4(destination) = f.external.ip() else {
        panic!("IPv4 fixture")
    };
    let payload = b"x4-exit";
    let length = 28 + payload.len();
    let mut bytes = vec![0; length];
    bytes[0] = 0x45;
    bytes[2..4].copy_from_slice(&(length as u16).to_be_bytes());
    bytes[8] = 64;
    bytes[9] = 17;
    bytes[12..16].copy_from_slice(&source.octets());
    bytes[16..20].copy_from_slice(&destination.octets());
    let mut sum: u32 = bytes[..20]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_be_bytes([b[0], b[1]]) as u32)
        .sum();
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    bytes[10..12].copy_from_slice(&(!(sum as u16)).to_be_bytes());
    bytes[20..22].copy_from_slice(&port.to_be_bytes());
    bytes[22..24].copy_from_slice(&f.external.port().to_be_bytes());
    bytes[24..26].copy_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
    bytes[28..].copy_from_slice(payload);
    Packet::parse(bytes).unwrap()
}

#[tokio::test]
#[ignore = "requires isolated Linux NET_ADMIN/TUN/nft/conntrack and external UDP echo"]
async fn daemon_exit_driver_real_nat_renew_confirm_failure_and_retirement() {
    isolated();
    let mut f = Fixture::open().await;
    let mut host = Host::open(f.host_directory(), Fixture::options())
        .await
        .unwrap();
    let snapshot = f.controls[2].execution_snapshot().unwrap();
    let capability = host
        .probe_exit(snapshot.instance().instance_id, snapshot.reference())
        .await;
    assert_eq!(capability.status, meshway_model::exit::ExitStatus::Ready);
    f.capability(capability).await;
    let demand = f.demand().await;
    let snapshot = f.controls[2].execution_snapshot().unwrap();
    host.install(snapshot.clone()).await.unwrap();
    let plan = snapshot.configuration().unwrap().exit_role.clone().unwrap();
    let role = host.install_exit_role(plan.clone()).await.unwrap();
    let mut runtime = Runtime::start(role, plan).unwrap();
    let mut mesh = Mesh::new(&f, demand);
    let pending = mesh.forward(packet(&f, 41000));
    runtime.packet(&mut mesh.e, pending);
    mesh.reply(&mut runtime).await;
    // Preserve an already-admitted old packet through a legal lease handoff.
    let old = mesh.forward(packet(&f, 41000));
    let previous = f.controls[2].execution_snapshot().unwrap();
    let lease = f.controls[0].renew_instance().await.unwrap();
    let instance = f.controls[0].execution_snapshot().unwrap().instance();
    f.controls[0]
        .renew_demand(&meshway_protocol::RenewDemandRequest {
            instance,
            request_id: demand,
            expires_at_ms: lease.expires_at_ms,
        })
        .await
        .unwrap();
    f.refresh().await;
    eprintln!(
        "X4_RENEW before_host role_active={} old_current={} retained={} continuation={}",
        runtime.role.is_active(),
        previous.ensure_current().is_ok(),
        previous.retention_deadline().is_ok(),
        f.controls[2]
            .execution_snapshot()
            .unwrap()
            .continuation_from(&previous)
            .is_ok()
    );
    mesh.reconcile(&f);
    host.install(f.controls[2].execution_snapshot().unwrap())
        .await
        .unwrap();
    eprintln!(
        "X4_RENEW after_host role_active={} host_role={} flows={}",
        runtime.role.is_active(),
        host.exit_role().is_some(),
        mesh.e.retained_flows()
    );
    runtime.packet(&mut mesh.e, old);
    let pending = mesh.forward(packet(&f, 41000));
    runtime.packet(&mut mesh.e, pending);
    mesh.reply(&mut runtime).await;

    // Fault only the new tuple before conntrack lookup in this test namespace.
    // The normal flow must survive ConfirmFailed on a shared circuit.
    let nft = tokio::process::Command::new("/usr/sbin/nft")
        .args(["-f", "-"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    install_drop(nft).await;
    let pending = mesh.forward(packet(&f, 41001));
    runtime.packet(&mut mesh.e, pending);
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            runtime.drive(&mut mesh.e);
            if mesh.e.retained_flows() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        tokio::process::Command::new("/usr/sbin/nft")
            .args(["delete", "table", "inet", "meshway_x4_fault"])
            .status()
            .await
            .unwrap()
            .success()
    );
    let pending = mesh.forward(packet(&f, 41000));
    runtime.packet(&mut mesh.e, pending);
    mesh.reply(&mut runtime).await;
    let health = runtime.health.clone();
    f.controls[2].stop().await.unwrap();
    runtime.stop(&mut mesh.e).await.unwrap();
    assert_eq!(mesh.e.retained_flows(), 0);
    assert_eq!(health.borrow().flows, 0);
    assert!(!health.borrow().failed);
    host.shutdown(false).await.unwrap();
    f.close().await;
}

async fn install_drop(mut child: tokio::process::Child) {
    use tokio::io::AsyncWriteExt;
    child.stdin.take().unwrap().write_all(b"add table inet meshway_x4_fault\nadd chain inet meshway_x4_fault pre { type filter hook prerouting priority -310; policy accept; }\nadd rule inet meshway_x4_fault pre udp sport 41001 drop\n").await.unwrap();
    assert!(child.wait().await.unwrap().success());
}

#[tokio::test]
#[ignore = "requires isolated Linux Host kernel installation"]
async fn daemon_exit_role_applied_probe_failure_and_instance_replacement() {
    isolated();
    let mut f = Fixture::open().await;
    let (reports, mut report) = tokio::sync::watch::channel(super::super::Report::stopped());
    let mut handle = super::super::Handle::open(
        f.host_directory(),
        &f.origin,
        reports,
        Fixture::options(),
        meshway_dns::SystemDnsSettings::default(),
    )
    .await
    .unwrap();
    handle.publish(&mut f.controls[2]).await.unwrap();
    tokio::time::timeout(
        Duration::from_secs(20),
        report.wait_for(|r| r.completion.is_some()),
    )
    .await
    .unwrap()
    .unwrap();
    handle.finish(&mut f.controls[2]).await.unwrap();
    if std::env::var("MESHWAY_X4_UNAVAILABLE").is_ok() {
        let current = report.borrow().clone();
        assert!(current.applied().is_none());
        let cap = current.exit_capability.unwrap();
        assert_eq!(
            cap.status,
            meshway_model::exit::ExitStatus::Unavailable {
                reason: meshway_model::exit::ExitUnavailableReason::IpForwardDisabled
            }
        );
        f.capability(cap).await;
        f.cli_status(report.clone(), false).await;
    } else {
        assert!(report.borrow().applied().is_some());
        assert_eq!(
            report
                .borrow()
                .exit_capability
                .as_ref()
                .unwrap()
                .installed_families,
            vec![meshway_model::exit::ExitFamily::Ipv4]
        );
        // Publish the actual prerequisite observation independently of the
        // installation receipt, then offer the resulting DNS service target.
        let mut capability = report.borrow().exit_capability.clone().unwrap();
        capability.installed_families.clear();
        f.capability(capability).await;
        f.refresh().await;
        let reference = f.controls[2].status().cached.unwrap();
        let external_dns = !matches!(
            f.controls[2]
                .execution_snapshot()
                .unwrap()
                .configuration()
                .unwrap()
                .dns
                .egress,
            meshway_configuration::execution::DnsEgress::NoExit
        );
        handle.publish(&mut f.controls[2]).await.unwrap();
        let outcome = tokio::time::timeout(
            Duration::from_secs(20),
            report.wait_for(|r| r.applied().as_ref() == Some(&reference)),
        )
        .await
        .map(|result| result.map(|_| ()));
        assert!(
            matches!(outcome, Ok(Ok(()))),
            "role refresh: plane={:?} problem={:?} capability={:?}",
            report.borrow().plane,
            report.borrow().problem,
            report.borrow().exit_capability
        );
        assert_eq!(report.borrow().exit_dns_unavailable, external_dns);
        handle.finish(&mut f.controls[2]).await.unwrap();
        f.cli_status(report.clone(), true).await;
        let old = f.controls[2].execution_snapshot().unwrap();
        f.controls[2].stop().await.unwrap();
        handle.stop(false).await.unwrap();
        assert!(old.ensure_current().is_err());
        f.controls[2].authenticate().await.unwrap();
        f.controls[2]
            .acquire_instance(super::super::capabilities())
            .await
            .unwrap();
        f.refresh().await;
        handle.publish(&mut f.controls[2]).await.unwrap();
        tokio::time::timeout(
            Duration::from_secs(20),
            report.wait_for(|r| r.applied().is_some()),
        )
        .await
        .unwrap()
        .unwrap();
        assert_ne!(
            report
                .borrow()
                .exit_capability
                .as_ref()
                .unwrap()
                .instance_id,
            old.instance().instance_id
        );
        handle.finish(&mut f.controls[2]).await.unwrap();
        f.configure(false).await;
        f.refresh().await;
        handle.publish(&mut f.controls[2]).await.unwrap();
        tokio::time::timeout(
            Duration::from_secs(20),
            report.wait_for(|r| r.applied().is_some() && r.exit_capability.is_none()),
        )
        .await
        .unwrap()
        .unwrap();
        handle.finish(&mut f.controls[2]).await.unwrap();
    }
    handle.shutdown(false).await.unwrap();
    f.close().await;
}
