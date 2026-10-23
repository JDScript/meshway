use meshway_model::{
    InstanceId, NetmapRef,
    exit::{
        ExitCapability, ExitFamily, ExitFamilyCapability, ExitStatus,
        ExitUnavailableReason as Reason,
    },
};
use rtnetlink::packet_route::link::{LinkAttribute, LinkFlags};

impl super::super::Kernel {
    pub async fn probe_exit(
        &mut self,
        instance_id: InstanceId,
        netmap: NetmapRef,
        role_generation: meshway_model::Digest,
        families: &[ExitFamily],
    ) -> ExitCapability {
        let result = self
            .live_probe(instance_id, netmap, role_generation, families)
            .await;
        let status = match result {
            Ok(observation) => return observation,
            Err(Unprobed::Unread) => ExitStatus::Probing,
            Err(Unprobed::Unavailable(reason)) => ExitStatus::Unavailable { reason },
        };
        ExitCapability {
            instance_id,
            netmap,
            role_generation,
            families: families
                .iter()
                .map(|family| ExitFamilyCapability {
                    family: *family,
                    status: status.clone(),
                    installed: false,
                })
                .collect(),
            observed_at: std::time::SystemTime::now().into(),
        }
    }
    async fn live_probe(
        &mut self,
        instance: InstanceId,
        netmap: NetmapRef,
        role_generation: meshway_model::Digest,
        families: &[ExitFamily],
    ) -> Result<ExitCapability, Unprobed> {
        use Unprobed::Unavailable as Not;
        self.ready()
            .await
            .map_err(judged(Reason::OwnershipConflict))?;
        // A proven change of an exit-owned object keeps the role withdrawn
        // until a re-read finds the evidence gone.
        if self.exit_blocked().await {
            return Err(Not(Reason::OwnershipConflict));
        }
        self.reconcile_stopped_exit()
            .await
            .map_err(judged(Reason::OwnershipConflict))?;
        self.verify().await.map_err(judged(Reason::RouteConflict))?;
        if let Some(probe) = &mut self.probe {
            probe.finish().await.map_err(|error| Not(error.reason))?;
        }
        self.quiesce()
            .await
            .map_err(judged(Reason::OwnershipConflict))?;
        self.expected
            .lock()
            .map_err(|_| Not(Reason::OwnershipConflict))?
            .probe_tun = None;
        let owned = self.owned_interfaces();
        let links = super::super::get_links(&self.handle, self.options.max_kernel_objects)
            .await
            .map_err(judged(Reason::RouteConflict))?;
        let routes = super::super::get_routes(&self.handle, self.options.max_kernel_objects)
            .await
            .map_err(judged(Reason::RouteConflict))?;
        let addresses = super::super::get_addresses(&self.handle, self.options.max_kernel_objects)
            .await
            .map_err(judged(Reason::RouteConflict))?;
        let mut crossing = super::firewall::Crossing::observe(&links, &routes, &addresses);
        // An installed exit TUN is judged by its name, not as a fresh one.
        crossing.installed = self.exit.as_ref().map(|owner| owner.interface_name.clone());
        crossing.mesh = self.journal.pools.clone();
        let interfaces: Vec<_> = links
            .iter()
            .filter(|link| {
                link.header.flags.contains(LinkFlags::Up)
                    && !link.header.flags.contains(LinkFlags::Loopback)
            })
            .filter_map(|link| {
                link.attributes
                    .iter()
                    .find_map(|attribute| match attribute {
                        LinkAttribute::IfName(name) if !owned.contains(name) => Some(name.clone()),
                        _ => None,
                    })
            })
            .collect();
        let mut owned_tables = Vec::new();
        if let Some(owner) = &self.exit {
            match owner.role.audit().await {
                Ok(tables) => owned_tables.extend(tables),
                Err(_) if !owner.role.is_active() => self
                    .reconcile_stopped_exit()
                    .await
                    .map_err(judged(Reason::OwnershipConflict))?,
                Err(error) => return Err(judged(Reason::OwnershipConflict)(error)),
            }
        }
        if let Some(owner) = &self.egress {
            owned_tables.extend(
                owner
                    .capture
                    .owned_tables()
                    .await
                    .map_err(judged(Reason::OwnershipConflict))?,
            );
        }
        let probe = self
            .probe
            .get_or_insert_with(|| super::probe::ExitProbe::live(self.expected.clone()));
        probe.owned_tables = owned_tables;
        probe.crossing = crossing;
        let observation = probe
            .observe(instance, netmap, role_generation, families, &interfaces)
            .await;
        probe.finish().await.map_err(|error| Not(error.reason))?;
        self.quiesce()
            .await
            .map_err(judged(Reason::OwnershipConflict))?;
        self.expected
            .lock()
            .map_err(|_| Not(Reason::OwnershipConflict))?
            .probe_tun = None;
        Ok(observation)
    }
}

/// Why the live probe produced no observation.
#[derive(Debug, PartialEq, Eq)]
enum Unprobed {
    /// A read or change failed, or a stopped role's cleanup is still being
    /// retried by its owner: nothing was proved. The families report
    /// `probing`; the periodic probe reads again.
    Unread,
    /// Read back and found unusable for the role.
    Unavailable(Reason),
}

/// A failed step of the probe judged by its error: an unknown one proves
/// nothing (`Unread`); anything else is `reason`. A stopped role's cleanup
/// its owner still retries keeps reporting `ownershipConflict` (with
/// `exit.cleanupPending`) until it completed.
fn judged(reason: Reason) -> impl Fn(crate::Error) -> Unprobed {
    move |error| match error {
        crate::Error::CleanupPending => Unprobed::Unavailable(Reason::OwnershipConflict),
        error if error.judgement() == crate::Judgement::Unknown => Unprobed::Unread,
        _ => Unprobed::Unavailable(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// U3: a probe step whose read failed proves nothing: `probing`, read
    /// again; what was read back and differs is a reason, and a stopped
    /// role's cleanup its owner still retries stays `ownershipConflict`.
    #[test]
    fn a_failed_read_of_the_probe_is_probing_and_a_read_back_conflict_its_reason() {
        for error in [
            crate::Error::Netlink,
            crate::Error::Drain,
            crate::Error::JournalUnwritten,
        ] {
            assert_eq!(judged(Reason::RouteConflict)(error), Unprobed::Unread);
        }
        assert_eq!(
            judged(Reason::RouteConflict)(crate::Error::CleanupPending),
            Unprobed::Unavailable(Reason::OwnershipConflict)
        );
        assert_eq!(
            judged(Reason::RouteConflict)(crate::Error::Conflict),
            Unprobed::Unavailable(Reason::RouteConflict)
        );
        assert_eq!(
            judged(Reason::RouteConflict)(crate::Error::PoolOverlap),
            Unprobed::Unavailable(Reason::RouteConflict)
        );
    }

    #[tokio::test]
    #[ignore = "requires isolated Linux NET_ADMIN namespace and an external interface"]
    async fn live_probe_open_host_reports_ipv4_ready_without_external_change() {
        assert_eq!(
            std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
            Ok("1")
        );
        let temp = tempfile::tempdir().unwrap();
        let directory = std::sync::Arc::new(
            meshway_private_storage::Directory::open(&temp.path().join("private"), "test.lock")
                .unwrap(),
        );
        let mut host = crate::Host::open(directory, Default::default())
            .await
            .unwrap();
        let mut changes = host.kernel.shared.network.dirty.subscribe();
        changes.borrow_and_update();
        let instance = "11111111-1111-4111-8111-111111111111".parse().unwrap();
        let netmap = NetmapRef {
            revision: meshway_model::Revision::MIN,
            serial: meshway_model::Version::MIN,
            digest: meshway_model::Digest::from_bytes([3; 32]),
        };
        let role_generation = meshway_model::Digest::from_bytes([4; 32]);
        let before = std::time::SystemTime::now();
        let expected = std::env::var("MESHWAY_X3C_PROBE").unwrap_or_else(|_| "ready".into());
        let mut commands = super::super::command::Commands::default();
        if expected == "firewall" {
            super::super::objects::mutate(&mut commands, &serde_json::json!({"nftables": [
                {"create": {"table": {"family": "inet", "name": "x3c_foreign_sentinel"}}},
                {"add": {"chain": {"family": "inet", "table": "x3c_foreign_sentinel", "name": "forward", "type": "filter", "hook": "forward", "prio": 0, "policy": "drop"}}}
            ]})).await.unwrap();
        }
        let families = if std::env::var("MESHWAY_PROBE_IPV6").as_deref() == Ok("1") {
            vec![ExitFamily::Ipv4, ExitFamily::Ipv6]
        } else {
            vec![ExitFamily::Ipv4]
        };
        let observation = host
            .probe_exit(instance, netmap, role_generation, &families)
            .await;
        let status = observation.families[0].status.clone();
        match expected.as_str() {
            "ready" => {
                assert_eq!(status, ExitStatus::Ready);
                assert_eq!(observation.families.len(), families.len());
            }
            "ipv6-ready" => assert_eq!(
                observation
                    .families
                    .iter()
                    .map(|entry| entry.status.clone())
                    .collect::<Vec<_>>(),
                [ExitStatus::Ready, ExitStatus::Ready]
            ),
            "ipv6-forwarding" | "ipv6-noegress" | "ipv6-ra" => {
                // IPv4 stays ready while the IPv6 family alone is unavailable.
                assert_eq!(status, ExitStatus::Ready);
                assert_eq!(
                    observation.families[1].status,
                    ExitStatus::Unavailable {
                        reason: match expected.as_str() {
                            "ipv6-forwarding" => Reason::Ipv6ForwardDisabled,
                            "ipv6-noegress" => Reason::Ipv6NoEgress,
                            _ => Reason::Ipv6RouterAdvertisementsIgnored,
                        }
                    }
                );
            }
            "forwarding" => assert_eq!(
                status,
                ExitStatus::Unavailable {
                    reason: Reason::IpForwardDisabled
                }
            ),
            "rpf" => assert_eq!(
                status,
                ExitStatus::Unavailable {
                    reason: Reason::ReversePathFilterConflict
                }
            ),
            "firewall" => assert_eq!(
                status,
                ExitStatus::Unavailable {
                    reason: Reason::FirewallConflict
                }
            ),
            _ => panic!("unknown fixture case"),
        }
        assert_eq!(observation.instance_id, instance);
        assert_eq!(observation.netmap, netmap);
        assert_eq!(observation.role_generation, role_generation);
        assert!(observation.families.iter().all(|entry| !entry.installed));
        let observed: std::time::SystemTime = observation.observed_at.into();
        assert!(observed >= before);
        assert!(!changes.has_changed().unwrap());
        host.shutdown(false).await.unwrap();
        if expected == "firewall" {
            let remaining = commands
                .run(
                    super::super::command::Tool::Nft,
                    vec![
                        "-j".into(),
                        "list".into(),
                        "table".into(),
                        "inet".into(),
                        "x3c_foreign_sentinel".into(),
                    ],
                    Vec::new(),
                )
                .await
                .unwrap();
            assert!(
                remaining.success,
                "Host cleanup must preserve the foreign firewall"
            );
            super::super::objects::mutate(&mut commands, &serde_json::json!({"nftables": [{"delete": {"table": {"family": "inet", "name": "x3c_foreign_sentinel"}}}]})).await.unwrap();
        }
    }
}
