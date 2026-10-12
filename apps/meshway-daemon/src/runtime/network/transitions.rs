//! How the network owner turns Control's offers and Engine timer events into
//! Engine transitions; the owner itself is Linux-only.
use meshway_engine::{Components, Desired, Engine, Netmap};

/// What an offered netmap changes in the installed desired state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Change {
    /// Same content, artifacts and associations on a healthy installation:
    /// only Engine's lease timer moves.
    Renewal,
    /// Same content and artifacts with another (or no) association overlay:
    /// a new desired state that every component applies, kept work and all.
    Associations,
    /// Other content or artifacts, or a degraded installation to retry.
    Content,
}

pub(super) fn change(desired: Option<&Desired>, degraded: bool, netmap: &Netmap) -> Change {
    let Some(desired) = desired.filter(|desired| {
        !degraded
            && desired.reference() == netmap.content.reference()
            && desired.same_artifacts(&netmap.artifacts)
    }) else {
        return Change::Content;
    };
    if desired.overlay() == netmap.associations.as_ref() {
        Change::Renewal
    } else {
        Change::Associations
    }
}

/// Consume one Engine timer event. `stop` ends the diagnostics the lease
/// allowed, before withdrawal; it runs only when the lease ended or the
/// Engine was left without a desired state, never when only some
/// associations ended. Returns whether the Engine was withdrawn and whether
/// its cleanup failed.
pub(super) async fn expire<C: Components>(
    engine: &mut Engine<C>,
    mut stop: impl AsyncFnMut(),
) -> (bool, bool) {
    let lease_ended = engine.lease_ended();
    if lease_ended {
        stop().await;
    }
    let failed = engine.expire().await.is_err();
    let withdrawn = engine.desired().is_none();
    if withdrawn && !lease_ended {
        stop().await;
    }
    (withdrawn, failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshway_configuration::netmap::{
        AssociationRecordType, CompiledNetmap, DnsAssociation, DnsAssociationOverlay,
    };
    use meshway_model::NetmapLease;
    use std::{sync::Arc, time::Duration};

    #[derive(Default)]
    struct Components;
    impl meshway_engine::Components for Components {
        fn exit_destinations(&self) -> std::collections::BTreeSet<std::net::IpAddr> {
            Default::default()
        }
        async fn apply_packets(
            &mut self,
            _: Option<&Desired>,
        ) -> Result<(), meshway_engine::Error> {
            Ok(())
        }
        async fn apply_adjacencies(
            &mut self,
            _: Option<&Desired>,
        ) -> Result<(), meshway_engine::Error> {
            Ok(())
        }
        async fn apply_host(&mut self, _: Option<&Desired>) -> Result<(), meshway_engine::Error> {
            Ok(())
        }
        async fn apply_dns(&mut self, _: Option<&Desired>) -> Result<(), meshway_engine::Error> {
            Ok(())
        }
    }

    fn now_ms() -> u64 {
        u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap()
    }

    fn offer(
        content: &Arc<CompiledNetmap>,
        lease_ms: u64,
        associations: Option<Arc<DnsAssociationOverlay>>,
    ) -> Netmap {
        let now = now_ms();
        Netmap {
            content: content.clone(),
            lease: NetmapLease::new(content.reference(), now, now + lease_ms).unwrap(),
            artifacts: Default::default(),
            associations,
        }
    }

    fn overlay(content: &CompiledNetmap, expires_at_ms: u64) -> Arc<DnsAssociationOverlay> {
        Arc::new(DnsAssociationOverlay {
            netmap: content.reference(),
            associations: vec![DnsAssociation {
                source: content.netmap.node_id,
                generation: meshway_model::Digest::from_bytes([7; 32]),
                name: "chat.example".into(),
                record_type: AssociationRecordType::A,
                cnames: vec![],
                addresses: vec![meshway_configuration::execution::ExecutionAddress(
                    "203.0.113.9".parse().unwrap(),
                )],
                expires_at_ms,
            }],
        })
    }

    #[tokio::test]
    async fn an_overlay_change_is_a_desired_change_not_a_renewal() {
        let mut engine = Engine::new(Components);
        let content = Arc::new(meshway_test_server::single_node("a", None));
        let plain = offer(&content, 600_000, None);
        assert_eq!(change(None, false, &plain), Change::Content);
        engine.reconfig(Some(&plain)).await.unwrap();
        let desired = engine.desired();
        assert_eq!(change(desired, false, &plain), Change::Renewal);
        assert_eq!(change(desired, true, &plain), Change::Content);
        let associated = offer(
            &content,
            600_000,
            Some(overlay(&content, now_ms() + 60_000)),
        );
        assert_eq!(change(desired, false, &associated), Change::Associations);
        engine.reconfig(Some(&associated)).await.unwrap();
        let desired = engine.desired();
        assert_eq!(change(desired, false, &associated), Change::Renewal);
        // A cleared overlay (Some → None) is a change as well.
        assert_eq!(change(desired, false, &plain), Change::Associations);
        // So is it to the offer deduplication.
        assert_ne!(
            super::super::offered(&Some(plain)),
            super::super::offered(&Some(associated))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn only_an_ended_lease_stops_diagnostics() {
        let mut engine = Engine::new(Components);
        let content = Arc::new(meshway_test_server::single_node("a", None));
        let associated = offer(
            &content,
            120_000,
            Some(overlay(&content, now_ms() + 30_000)),
        );
        engine.reconfig(Some(&associated)).await.unwrap();
        let mut stops = 0;
        // The association ends first: the lease and its diagnostics stay.
        tokio::time::advance(Duration::from_secs(31)).await;
        let outcome = expire(&mut engine, async || stops += 1).await;
        assert_eq!((outcome, stops), ((false, false), 0));
        assert!(engine.desired().unwrap().overlay().is_some());
        // The lease ends: diagnostics stop once, the Engine withdraws.
        tokio::time::advance(Duration::from_secs(90)).await;
        let outcome = expire(&mut engine, async || stops += 1).await;
        assert_eq!((outcome, stops), ((true, false), 1));
    }
}
