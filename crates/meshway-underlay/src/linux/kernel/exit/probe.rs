use super::{
    ExitError,
    command::{Commands, Tool},
};
use meshway_model::exit::{
    ExitCapability, ExitFamily, ExitStatus, ExitUnavailableReason as Reason,
};
use meshway_model::{ConfigurationRef, InstanceId};
use std::{io::Read, time::SystemTime};

/// Owned probe operation. Keep this value until `finish` completes, even if the
/// original requester is cancelled. No UID/capability bitmap stands in for I/O.
pub struct ExitProbe {
    commands: Commands,
    pending_table: Option<String>,
    // Probe TUN events must not race an already running Host monitor.
    _owner: super::super::super::HostOwner,
}

impl ExitProbe {
    pub(in crate::linux) fn new(owner: super::super::super::HostOwner) -> Self {
        Self {
            commands: Commands::default(),
            pending_table: None,
            _owner: owner,
        }
    }
    /// Read-only host prerequisites and atomic, net-zero nft feature transaction.
    /// `interfaces` must be the actual ingress and egress interfaces. Strict RPF
    /// is conservatively unsupported until a bidirectional proof is available.
    /// This is a prerequisite observation; it never reports installed families.
    pub async fn observe(
        &mut self,
        instance_id: InstanceId,
        target: ConfigurationRef,
        interfaces: &[String],
    ) -> ExitCapability {
        let result = self.check(interfaces).await;
        ExitCapability {
            instance_id,
            target,
            supported_families: vec![ExitFamily::Ipv4],
            installed_families: Vec::new(),
            status: match result {
                // Route/rule installation and precise conntrack deletion still
                // require the Kernel owner's journaled resource transaction.
                Ok(()) => ExitStatus::Probing,
                Err(error) => ExitStatus::Unavailable {
                    reason: error.reason,
                },
            },
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

    async fn check(&mut self, interfaces: &[String]) -> Result<(), ExitError> {
        self.finish().await?;
        forwarding(read_sysctl("/proc/sys/net/ipv4/ip_forward"))?;
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
            reverse_path(all_rpf, rpf, all_mark, mark)?;
        }
        let _device = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/net/tun")
            .map_err(|_| Reason::TunUnavailable)?;
        // Create a nonpersistent, unnumbered TUN. Success is the real NET_ADMIN
        // check; dropping the owned descriptor cannot delete an external device.
        let tun = meshway_tun::Tun::create(
            meshway_tun::Options::new(1280).map_err(|_| Reason::TunUnavailable)?,
        )
        .map_err(|error| match error {
            meshway_tun::Error::Io(ref io) if io.kind() == std::io::ErrorKind::PermissionDenied => {
                Reason::NetAdminMissing
            }
            _ => Reason::TunUnavailable,
        })?;
        let ingress_rpf = read_sysctl(&format!("/proc/sys/net/ipv4/conf/{}/rp_filter", tun.name()))
            .map_err(|_| Reason::ForwardingProbeFailed)?;
        let ingress_mark = read_sysctl(&format!(
            "/proc/sys/net/ipv4/conf/{}/src_valid_mark",
            tun.name()
        ))
        .map_err(|_| Reason::ForwardingProbeFailed)?;
        reverse_path(all_rpf, ingress_rpf, all_mark, ingress_mark)?;
        drop(tun);
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
        let output = self
            .commands
            .run(
                Tool::Conntrack,
                vec![
                    "-L".into(),
                    "-f".into(),
                    "ipv4".into(),
                    "--mark".into(),
                    "0xffffffff/0xffffffff".into(),
                    "-o".into(),
                    "xml".into(),
                ],
                Vec::new(),
            )
            .await
            .map_err(|_| Reason::ConntrackUnavailable)?;
        // This proves only that the filtered dump operation is available. No
        // output entry is treated as identity, authority, or deletion evidence.
        if !output.success {
            return Err(Reason::ConntrackUnavailable.into());
        }
        Ok(())
    }
}

fn read_sysctl(path: &str) -> Result<u32, ()> {
    let file = std::fs::File::open(path).map_err(|_| ())?;
    let mut text = String::new();
    file.take(32).read_to_string(&mut text).map_err(|_| ())?;
    if text.len() >= 32 {
        return Err(());
    }
    text.trim().parse().map_err(|_| ())
}

fn forwarding(value: Result<u32, ()>) -> Result<(), Reason> {
    match value {
        Ok(1) => Ok(()),
        Ok(0) => Err(Reason::IpForwardDisabled),
        _ => Err(Reason::ForwardingProbeFailed),
    }
}

fn reverse_path(all: u32, interface: u32, all_mark: u32, mark: u32) -> Result<(), Reason> {
    if all > 2 || interface > 2 || all_mark > 1 || mark > 1 {
        return Err(Reason::ForwardingProbeFailed);
    }
    if all.max(interface) == 1 {
        return Err(Reason::ReversePathFilterConflict);
    }
    Ok(())
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
        let target = ConfigurationRef {
            revision: meshway_model::Revision::MIN,
            execution_publication: "22222222-2222-4222-8222-222222222222".parse().unwrap(),
            digest: meshway_model::Digest::from_bytes([3; 32]),
        };
        let before = SystemTime::now();
        let observation = probe
            .observe(instance, target.clone(), &["lo".into()])
            .await;
        assert_eq!(observation.status, ExitStatus::Probing);
        assert_eq!(observation.instance_id, instance);
        assert_eq!(observation.target, target);
        assert!(observation.installed_families.is_empty());
        assert!(observation.observed_at >= before.into());
        assert!(observation.observed_at <= SystemTime::now().into());
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
            probe.check(&["lo".into()]).await.unwrap_err().reason,
            Reason::IpForwardDisabled
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
            probe.check(&["lo".into()]).await.unwrap_err().reason,
            Reason::NetAdminMissing
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
            probe.check(&["lo".into()]).await.unwrap_err().reason,
            Reason::ReversePathFilterConflict
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
