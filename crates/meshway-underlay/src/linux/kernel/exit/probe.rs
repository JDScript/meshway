use super::{ExitError, command::Commands};
use meshway_model::exit::{
    ExitCapability, ExitFamily, ExitFamilyCapability, ExitStatus, ExitUnavailableReason as Reason,
};
use meshway_model::{InstanceId, NetmapRef};
use std::{collections::BTreeMap, time::SystemTime};

/// Owned probe operation. Keep this value until `finish` completes, even if the
/// original requester is cancelled. No UID/capability bitmap stands in for I/O.
pub struct ExitProbe {
    pub(in crate::linux::kernel) owned_tables: Vec<(String, String)>,
    pub(in crate::linux::kernel) crossing: super::firewall::Crossing,
    commands: Commands,
    pending_table: Option<String>,
    // Probe TUN events must not race an already running Host monitor.
    _owner: Option<super::super::super::HostOwner>,
    monitor: Option<std::sync::Arc<std::sync::Mutex<super::super::monitor::Expected>>>,
}

impl ExitProbe {
    pub(in crate::linux) fn new(owner: super::super::super::HostOwner) -> Self {
        Self {
            owned_tables: Vec::new(),
            crossing: Default::default(),
            commands: Commands::default(),
            pending_table: None,
            _owner: Some(owner),
            monitor: None,
        }
    }
    pub(in crate::linux::kernel) fn live(
        monitor: std::sync::Arc<std::sync::Mutex<super::super::monitor::Expected>>,
    ) -> Self {
        Self {
            owned_tables: Vec::new(),
            crossing: Default::default(),
            commands: Commands::default(),
            pending_table: None,
            _owner: None,
            monitor: Some(monitor),
        }
    }
    /// Read-only host prerequisites and atomic, net-zero nft feature
    /// transaction, per family of the role. `interfaces` must be the actual
    /// ingress and egress interfaces. Strict RPF is conservatively
    /// unsupported until a bidirectional proof is available. A shared
    /// prerequisite (TUN, NET_ADMIN, nft, ctnetlink, ownership) fails every
    /// family with the same reason. This is a prerequisite observation; it
    /// never reports installed families.
    pub async fn observe(
        &mut self,
        instance_id: InstanceId,
        netmap: NetmapRef,
        role_generation: meshway_model::Digest,
        families: &[ExitFamily],
        interfaces: &[String],
    ) -> ExitCapability {
        let result = self.check(interfaces, families).await;
        let live = self.monitor.is_some();
        ExitCapability {
            instance_id,
            netmap,
            role_generation,
            families: families
                .iter()
                .map(|family| ExitFamilyCapability {
                    family: *family,
                    status: match result.get(family) {
                        Some(Err(reason)) => ExitStatus::Unavailable { reason: *reason },
                        // Route/rule installation and precise conntrack
                        // deletion still require the Kernel owner's
                        // journaled resource transaction.
                        Some(Ok(())) if live => ExitStatus::Ready,
                        _ => ExitStatus::Probing,
                    },
                    installed: false,
                })
                .collect(),
            observed_at: SystemTime::now().into(),
        }
    }

    pub async fn finish(&mut self) -> Result<(), ExitError> {
        let settled = self.commands.settle().await;
        if let Some(name) = &self.pending_table {
            super::nft::verify_absent(&mut self.commands, name).await?;
            self.pending_table = None;
        }
        settled
            .map(|_| ())
            .map_err(|_| Reason::NftUnavailable.into())
    }

    /// Each family's result. A family's own prerequisite is reported before
    /// shared ones (the forwarding a host administrator sets is read first);
    /// a shared failure fails every family not failed already.
    async fn check(
        &mut self,
        interfaces: &[String],
        families: &[ExitFamily],
    ) -> BTreeMap<ExitFamily, Result<(), Reason>> {
        let mut results: BTreeMap<ExitFamily, Result<(), Reason>> =
            families.iter().map(|family| (*family, Ok(()))).collect();
        if let Err(reason) = forwarding(read_sysctl("/proc/sys/net/ipv4/ip_forward")) {
            fail(&mut results, ExitFamily::Ipv4, reason);
        }
        if families.contains(&ExitFamily::Ipv6)
            && let Err(reason) = self.ipv6()
        {
            fail(&mut results, ExitFamily::Ipv6, reason);
        }
        if let Err(error) = self.shared(interfaces, families, &mut results).await {
            for family in families {
                fail(&mut results, *family, error.reason);
            }
        }
        results
    }

    async fn shared(
        &mut self,
        interfaces: &[String],
        families: &[ExitFamily],
        results: &mut BTreeMap<ExitFamily, Result<(), Reason>>,
    ) -> Result<(), ExitError> {
        self.finish().await?;
        if interfaces.is_empty() || interfaces.len() > 128 {
            return Err(Reason::ForwardingProbeFailed.into());
        }
        let all_rpf = read_sysctl("/proc/sys/net/ipv4/conf/all/rp_filter")
            .map_err(|_| Reason::ForwardingProbeFailed)?;
        let all_mark = read_sysctl("/proc/sys/net/ipv4/conf/all/src_valid_mark")
            .map_err(|_| Reason::ForwardingProbeFailed)?;
        for interface in interfaces {
            if interface.is_empty()
                || matches!(interface.as_str(), "." | "..")
                || interface.len() > 15
                || !interface
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
            {
                return Err(Reason::ForwardingProbeFailed.into());
            }
            let rpf = read_sysctl(&format!("/proc/sys/net/ipv4/conf/{interface}/rp_filter"))
                .map_err(|_| Reason::ForwardingProbeFailed)?;
            let mark = read_sysctl(&format!(
                "/proc/sys/net/ipv4/conf/{interface}/src_valid_mark"
            ))
            .map_err(|_| Reason::ForwardingProbeFailed)?;
            if let Err(reason) = reverse_path(all_rpf, rpf, all_mark, mark) {
                fail(results, ExitFamily::Ipv4, reason);
            }
        }
        if self.monitor.is_some() {
            for (family, judged) in super::firewall::check(
                &mut self.commands,
                &self.owned_tables,
                &self.crossing,
                families,
            )
            .await?
            {
                if let Err(reason) = judged {
                    fail(results, family, reason);
                }
            }
        }
        let _device = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/net/tun")
            .map_err(|_| Reason::TunUnavailable)?;
        // Create a nonpersistent, unnumbered TUN. Success is the real NET_ADMIN
        // check; dropping the owned descriptor cannot delete an external device.
        let mut monitor = self
            .monitor
            .as_ref()
            .map(|state| state.lock().map_err(|_| Reason::OwnershipConflict))
            .transpose()?;
        let tun = meshway_tun::Tun::create(
            meshway_tun::Options::new(1280).map_err(|_| Reason::TunUnavailable)?,
        )
        .map_err(|error| match error {
            meshway_tun::Error::Io(ref io) if io.kind() == std::io::ErrorKind::PermissionDenied => {
                Reason::NetAdminMissing
            }
            _ => Reason::TunUnavailable,
        })?;
        if let Some(state) = monitor.as_mut() {
            state.probe_tun = Some(tun.if_index().map_err(|_| Reason::TunUnavailable)?);
        }
        drop(monitor);
        let ingress_rpf = read_sysctl(&format!("/proc/sys/net/ipv4/conf/{}/rp_filter", tun.name()))
            .map_err(|_| Reason::ForwardingProbeFailed)?;
        let ingress_mark = read_sysctl(&format!(
            "/proc/sys/net/ipv4/conf/{}/src_valid_mark",
            tun.name()
        ))
        .map_err(|_| Reason::ForwardingProbeFailed)?;
        if let Err(reason) = reverse_path(all_rpf, ingress_rpf, all_mark, ingress_mark) {
            fail(results, ExitFamily::Ipv4, reason);
        }
        // A new interface inherits `default/forwarding`, not `ip_forward`:
        // the exit TUN forwards only if this fresh TUN does. IPv6
        // forwarding is `all` alone.
        if let Err(reason) = forwarding(read_sysctl(&format!(
            "/proc/sys/net/ipv4/conf/{}/forwarding",
            tun.name()
        ))) {
            fail(results, ExitFamily::Ipv4, reason);
        }
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| Reason::NftUnavailable)?;
        let name = format!(
            "meshway_probe_{}",
            random
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        // One atomic batch adds, exercises and removes a private table. Even a
        // lost ACK leaves no object to adopt or guess-delete after cancellation.
        self.pending_table = Some(name.clone());
        let outcome = super::nft::submit(&mut self.commands, &super::nft::probe_batch(&name)).await;
        // Both known failure and uncertain ACK reconcile before another tool.
        self.finish().await?;
        outcome?;
        if self.monitor.is_some() {
            // The program of every family of the role, NAT66 included.
            let program = super::program::Program {
                legacy: false,
                installation: random.iter().map(|byte| format!("{byte:02x}")).collect(),
                interface: tun.if_index().map_err(|_| Reason::TunUnavailable)?,
                interface_name: tun.name().to_owned(),
                mark: 1,
                pools: vec![
                    "100.96.0.0/11"
                        .parse()
                        .map_err(|_| Reason::NftUnavailable)?,
                ],
                families: families.to_vec(),
                pools6: if families.contains(&ExitFamily::Ipv6) {
                    vec!["fd00::/64".parse().map_err(|_| Reason::NftUnavailable)?]
                } else {
                    Vec::new()
                },
            };
            let mut batch = program.create();
            for (family, name) in program.tables() {
                batch["nftables"]
                    .as_array_mut()
                    .expect("generated batch")
                    .push(
                        serde_json::json!({"delete": {"table": {"family": family, "name": name}}}),
                    );
            }
            super::nft::submit(&mut self.commands, &batch).await?;
        }
        drop(tun);
        super::conntrack::available().await?;
        Ok(())
    }

    /// The IPv6 family's own prerequisites: forwarding (`all` alone decides
    /// IPv6 forwarding), a usable global address and a route toward the
    /// Internet on some egress interface, and router advertisements that
    /// still reach the host: with forwarding on, an interface whose
    /// Internet route an advertisement installed must have `accept_ra = 2`,
    /// or that route lapses. Meshway never sets these.
    fn ipv6(&self) -> Result<(), Reason> {
        match read_sysctl("/proc/sys/net/ipv6/conf/all/forwarding") {
            Ok(1) => {}
            Ok(0) => return Err(Reason::Ipv6ForwardDisabled),
            _ => return Err(Reason::ForwardingProbeFailed),
        }
        if self.crossing.egress6.is_empty() || self.crossing.local6.is_empty() {
            return Err(Reason::Ipv6NoEgress);
        }
        for interface in &self.crossing.advertised6 {
            if interface.is_empty() || interface.contains('/') || interface.starts_with('.') {
                return Err(Reason::ForwardingProbeFailed);
            }
            match read_sysctl(&format!("/proc/sys/net/ipv6/conf/{interface}/accept_ra")) {
                Ok(2) => {}
                Ok(_) => return Err(Reason::Ipv6RouterAdvertisementsIgnored),
                Err(()) => return Err(Reason::ForwardingProbeFailed),
            }
        }
        Ok(())
    }
}

/// Fail `family` with `reason` unless it failed already.
fn fail(
    results: &mut BTreeMap<ExitFamily, Result<(), Reason>>,
    family: ExitFamily,
    reason: Reason,
) {
    if let Some(result @ Ok(())) = results.get_mut(&family) {
        *result = Err(reason);
    }
}

fn read_sysctl(path: &str) -> Result<u32, ()> {
    super::host::sysctl(path).map_err(|_| ())
}

fn forwarding(value: Result<u32, ()>) -> Result<(), Reason> {
    match value {
        Ok(1) => Ok(()),
        Ok(0) => Err(Reason::IpForwardDisabled),
        _ => Err(Reason::ForwardingProbeFailed),
    }
}

fn reverse_path(all: u32, interface: u32, all_mark: u32, mark: u32) -> Result<(), Reason> {
    match super::host::reverse_path(all, interface, all_mark, mark) {
        super::host::ReversePath::Usable => Ok(()),
        super::host::ReversePath::Strict => Err(Reason::ReversePathFilterConflict),
        super::host::ReversePath::Invalid => Err(Reason::ForwardingProbeFailed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires isolated Linux namespace, forwarding, loose/off RPF, nft and conntrack"]
    async fn exit_preflight_preserves_target_and_observation_without_claiming_ready() {
        assert_eq!(
            std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
            Ok("1")
        );
        let mut probe = crate::Host::exit_probe().unwrap();
        let instance = "11111111-1111-4111-8111-111111111111".parse().unwrap();
        let netmap = NetmapRef {
            revision: meshway_model::Revision::MIN,
            serial: meshway_model::Version::MIN,
            digest: meshway_model::Digest::from_bytes([3; 32]),
        };
        let role_generation = meshway_model::Digest::from_bytes([4; 32]);
        let before = SystemTime::now();
        let observation = probe
            .observe(
                instance,
                netmap,
                role_generation,
                &[ExitFamily::Ipv4],
                &["lo".into()],
            )
            .await;
        assert_eq!(observation.families[0].status, ExitStatus::Probing);
        assert_eq!(observation.instance_id, instance);
        assert_eq!(observation.netmap, netmap);
        assert_eq!(observation.role_generation, role_generation);
        assert!(!observation.families[0].installed);
        let observed: SystemTime = observation.observed_at.into();
        assert!(observed >= before);
        assert!(observed <= SystemTime::now());
        let frozen = observation.clone();
        probe.finish().await.unwrap();
        assert_eq!(observation, frozen);
    }

    #[tokio::test]
    #[ignore = "requires isolated Linux namespace with ip_forward=0"]
    async fn exit_probe_reports_disabled_forwarding_before_privilege_checks() {
        assert_eq!(
            std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
            Ok("1")
        );
        assert_eq!(read_sysctl("/proc/sys/net/ipv4/ip_forward"), Ok(0));
        let mut probe = crate::Host::exit_probe().unwrap();
        assert_eq!(
            probe.check(&["lo".into()], &[ExitFamily::Ipv4]).await[&ExitFamily::Ipv4],
            Err(Reason::IpForwardDisabled)
        );
        probe.finish().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires isolated Linux namespace with forwarding=1, RPF off and no NET_ADMIN"]
    async fn exit_probe_requires_successful_net_admin_operation() {
        assert_eq!(
            std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
            Ok("1")
        );
        assert_eq!(read_sysctl("/proc/sys/net/ipv4/ip_forward"), Ok(1));
        let mut probe = crate::Host::exit_probe().unwrap();
        assert_eq!(
            probe.check(&["lo".into()], &[ExitFamily::Ipv4]).await[&ExitFamily::Ipv4],
            Err(Reason::NetAdminMissing)
        );
        probe.finish().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires isolated Linux namespace with forwarding=1 and strict lo RPF"]
    async fn exit_probe_rejects_strict_reverse_path_filter() {
        assert_eq!(
            std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
            Ok("1")
        );
        assert_eq!(read_sysctl("/proc/sys/net/ipv4/ip_forward"), Ok(1));
        assert_eq!(read_sysctl("/proc/sys/net/ipv4/conf/lo/rp_filter"), Ok(1));
        let mut probe = crate::Host::exit_probe().unwrap();
        assert_eq!(
            probe.check(&["lo".into()], &[ExitFamily::Ipv4]).await[&ExitFamily::Ipv4],
            Err(Reason::ReversePathFilterConflict)
        );
        probe.finish().await.unwrap();
    }

    #[test]
    fn forwarding_and_reverse_path_failures_are_distinct() {
        for (value, expected) in [
            (Ok(0), Err(Reason::IpForwardDisabled)),
            (Ok(1), Ok(())),
            (Ok(2), Err(Reason::ForwardingProbeFailed)),
            (Err(()), Err(Reason::ForwardingProbeFailed)),
        ] {
            assert_eq!(forwarding(value), expected);
        }
        for all in 0..=2 {
            for interface in 0..=2 {
                for mark in 0..=1 {
                    assert_eq!(
                        reverse_path(all, interface, mark, mark),
                        if all.max(interface) == 1 {
                            Err(Reason::ReversePathFilterConflict)
                        } else {
                            Ok(())
                        }
                    );
                }
            }
        }
        assert_eq!(reverse_path(3, 0, 0, 0), Err(Reason::ForwardingProbeFailed));
    }
}
