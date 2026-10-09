use meshway_model::{
    ConfigurationRef, InstanceId,
    exit::{ExitCapability, ExitStatus, ExitUnavailableReason as Reason},
};
use rtnetlink::packet_route::link::{LinkAttribute, LinkFlags};

impl super::super::Kernel {
    pub async fn probe_exit(
        &mut self,
        instance_id: InstanceId,
        target: ConfigurationRef,
    ) -> ExitCapability {
        let result = self.live_probe(instance_id, target.clone()).await;
        match result {
            Ok(observation) => observation,
            Err(reason) => ExitCapability {
                instance_id,
                target,
                supported_families: Vec::new(),
                installed_families: Vec::new(),
                status: ExitStatus::Unavailable { reason },
                observed_at: std::time::SystemTime::now().into(),
            },
        }
    }
    async fn live_probe(
        &mut self,
        instance: InstanceId,
        target: ConfigurationRef,
    ) -> Result<ExitCapability, Reason> {
        self.ready().await.map_err(|_| Reason::OwnershipConflict)?;
        self.verify().await.map_err(|_| Reason::RouteConflict)?;
        if let Some(probe) = &mut self.probe {
            probe.finish().await.map_err(|error| error.reason)?;
        }
        self.quiesce()
            .await
            .map_err(|_| Reason::OwnershipConflict)?;
        self.expected
            .lock()
            .map_err(|_| Reason::OwnershipConflict)?
            .probe_tun = None;
        let owned = self.owned_interfaces();
        let links = super::super::collect(
            self.handle.link().get().execute(),
            self.options.max_kernel_objects,
        )
        .await
        .map_err(|_| Reason::RouteConflict)?;
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
            owned_tables.extend(
                owner
                    .role
                    .audit()
                    .await
                    .map_err(|_| Reason::OwnershipConflict)?,
            );
        }
        if let Some(owner) = &self.egress {
            owned_tables.push((
                "inet".into(),
                owner
                    .capture
                    .audit()
                    .await
                    .map_err(|_| Reason::OwnershipConflict)?,
            ));
        }
        let probe = self
            .probe
            .get_or_insert_with(|| super::probe::ExitProbe::live(self.expected.clone()));
        probe.owned_tables = owned_tables;
        let observation = probe.observe(instance, target, &interfaces).await;
        probe.finish().await.map_err(|error| error.reason)?;
        self.quiesce()
            .await
            .map_err(|_| Reason::OwnershipConflict)?;
        self.expected
            .lock()
            .map_err(|_| Reason::OwnershipConflict)?
            .probe_tun = None;
        Ok(observation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let mut changes = host.network_changes();
        changes.borrow_and_update();
        let instance = "11111111-1111-4111-8111-111111111111".parse().unwrap();
        let target = ConfigurationRef {
            revision: meshway_model::Revision::MIN,
            execution_publication: "22222222-2222-4222-8222-222222222222".parse().unwrap(),
            digest: meshway_model::Digest::from_bytes([3; 32]),
        };
        let before = std::time::SystemTime::now();
        let expected = std::env::var("MESHWAY_X3C_PROBE").unwrap_or_else(|_| "ready".into());
        let mut commands = super::super::command::Commands::default();
        if expected == "firewall" {
            super::super::objects::mutate(&mut commands, &serde_json::json!({"nftables": [
                {"create": {"table": {"family": "inet", "name": "x3c_foreign_sentinel"}}},
                {"add": {"chain": {"family": "inet", "table": "x3c_foreign_sentinel", "name": "forward", "type": "filter", "hook": "forward", "prio": 0, "policy": "drop"}}}
            ]})).await.unwrap();
        }
        let observation = host.probe_exit(instance, target.clone()).await;
        match expected.as_str() {
            "ready" => {
                assert_eq!(observation.status, ExitStatus::Ready);
                assert_eq!(
                    observation.supported_families,
                    vec![meshway_model::exit::ExitFamily::Ipv4]
                );
            }
            "forwarding" => assert_eq!(
                observation.status,
                ExitStatus::Unavailable {
                    reason: Reason::IpForwardDisabled
                }
            ),
            "rpf" => assert_eq!(
                observation.status,
                ExitStatus::Unavailable {
                    reason: Reason::ReversePathFilterConflict
                }
            ),
            "firewall" => assert_eq!(
                observation.status,
                ExitStatus::Unavailable {
                    reason: Reason::FirewallConflict
                }
            ),
            _ => panic!("unknown fixture case"),
        }
        assert_eq!(observation.instance_id, instance);
        assert_eq!(observation.target, target);
        assert!(observation.installed_families.is_empty());
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
