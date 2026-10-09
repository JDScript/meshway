use super::*;
use crate::{DataPlaneIdentity, Started};
use ed25519_dalek::SigningKey;
use meshway_configuration::{
    AddressPools, Input, NodeAddresses, NodeIdentity, NodeSettings, SubjectContext,
    execution::TrustedNode,
};
use meshway_model::{
    BoundNodeKey, Digest, KeyAlgorithm, NetworkId, NodeId, NodePublicKey, Revision, Version,
};
use meshway_policy::compile::netmap::{NetmapInput, PreviousNetmap, compile_netmap};
use std::{collections::BTreeMap, time::Duration};
use zeroize::Zeroizing;

const NOISE: [u8; 32] = [5; 32];

fn network() -> NetworkId {
    "00000000-0000-4003-8000-000000000001".parse().unwrap()
}
fn node() -> NodeId {
    "00000000-0000-4000-8000-000000000001".parse().unwrap()
}

fn keys() -> TrustedNode {
    let key = |id: &str, algorithm, bytes: [u8; 32]| BoundNodeKey {
        id: id.parse().unwrap(),
        generation: Version::MIN,
        public_key: NodePublicKey {
            algorithm,
            bytes: Digest::from_bytes(bytes),
        },
    };
    TrustedNode {
        control_key: key(
            "00000000-0000-4002-8000-000000000001",
            KeyAlgorithm::Ed25519,
            SigningKey::from_bytes(&[4; 32]).verifying_key().to_bytes(),
        ),
        dataplane_key: key(
            "00000000-0000-4002-8000-000000000002",
            KeyAlgorithm::X25519,
            curve25519_dalek::montgomery::MontgomeryPoint::mul_base_clamped(NOISE).to_bytes(),
        ),
        capabilities: BTreeMap::new(),
    }
}

/// A real one-Node netmap; `previous` keeps the serial sequence.
fn compiled(hostname: &str, address: &str, previous: Option<&CompiledNetmap>) -> CompiledNetmap {
    let mut cfg = meshway_configuration::new_network(
        AddressPools {
            ipv4: vec!["100.64.0.0/24".into()],
            ipv6: vec![],
        },
        "test.internal".into(),
    )
    .unwrap();
    cfg.tags_owners.insert("tag:server".into(), vec![]);
    cfg.nodes.insert(
        node(),
        NodeSettings {
            hostname: hostname.into(),
            remark: String::new(),
            identity: NodeIdentity::Tag {
                tags: vec!["tag:server".into()],
            },
            enabled: true,
            addresses: NodeAddresses {
                ipv4: Some(address.parse().unwrap()),
                ipv6: None,
            },
            listeners: BTreeMap::new(),
            dialers: Input::Omitted,
            exit_enabled: false,
        },
    );
    let previous = previous
        .map(|old| BTreeMap::from([(node(), PreviousNetmap::from(old))]))
        .unwrap_or_default();
    compile_netmap(NetmapInput {
        network: network(),
        revision: Revision::MIN,
        configuration: &cfg,
        subjects: &SubjectContext::default(),
        inventory: &BTreeMap::from([(node(), keys())]),
        rule_sets: &BTreeMap::new(),
        selected_profiles: &BTreeMap::new(),
        selection_versions: &BTreeMap::new(),
        previous: &previous,
    })
    .unwrap()
    .nodes
    .remove(&node())
    .unwrap()
}

struct Issuer {
    clock: Arc<Clock>,
    validity: Arc<AtomicBool>,
    invalidated: CancellationToken,
    lineage: Arc<()>,
}

impl Issuer {
    fn new() -> Self {
        Self {
            clock: Arc::new(Clock::new().unwrap()),
            validity: Arc::new(AtomicBool::new(true)),
            invalidated: CancellationToken::new(),
            lineage: Arc::new(()),
        }
    }
    fn lease(&self, netmap: &CompiledNetmap, validity_ms: u64) -> (NetmapLease, Deadline) {
        let started = Started::now(&self.clock).unwrap();
        let lease = NetmapLease::new(
            netmap.reference(),
            started.wall(),
            started.wall() + validity_ms,
        )
        .unwrap();
        (
            lease,
            Deadline::from_response(started, lease.expires_at_ms).unwrap(),
        )
    }
    fn issue(
        &self,
        netmap: &CompiledNetmap,
        validity_ms: u64,
    ) -> Result<(ExecutionSnapshot, NetmapRenewal), Error> {
        let (lease, deadline) = self.lease(netmap, validity_ms);
        ExecutionSnapshot::issue_netmap(NetmapIssue {
            netmap: Arc::new(netmap.clone()),
            lease,
            deadline,
            instance: None,
            validity: Arc::clone(&self.validity),
            invalidated: self.invalidated.clone(),
            clock: Arc::clone(&self.clock),
            lineage: Arc::downgrade(&self.lineage),
        })
    }
    /// What the owner does on a new serial: invalidate every old guard.
    fn invalidate(&mut self) {
        self.validity
            .store(false, std::sync::atomic::Ordering::Release);
        self.invalidated.cancel();
        self.validity = Arc::new(AtomicBool::new(true));
        self.invalidated = CancellationToken::new();
    }
}

#[tokio::test]
async fn same_content_renewal_raises_every_clone_without_cancelling_a_guard() {
    let issuer = Issuer::new();
    let netmap = compiled("a", "100.64.0.1", None);
    let (first, renewal) = issuer.issue(&netmap, 300).unwrap();
    let held = first.clone();
    let waiter = tokio::spawn({
        let held = held.clone();
        async move { held.invalidated().await }
    });
    let before = first.expires_at_ms();
    let (lease, deadline) = issuer.lease(&netmap, 60_000);
    renewal.renew(lease, deadline).unwrap();
    let renewed = renewal.snapshot(None).unwrap();
    assert!(renewed.same_netmap(&first) && held.same_netmap(&renewed));
    assert!(renewed.netmap_continuation_from(&first).is_err());
    assert!(held.expires_at_ms() >= before + 59_000);
    assert_eq!(held.lease(), Some(lease));
    tokio::time::sleep(Duration::from_millis(400)).await;
    held.ensure_current().unwrap();
    let reading = held.netmap_reading().unwrap();
    assert_eq!(reading.lease, lease);
    assert_eq!(reading.leased().netmap.serial, netmap.netmap.serial);
    assert!(
        !waiter.is_finished(),
        "renewal must not end a waiting guard"
    );
    issuer.invalidated.cancel();
    tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .unwrap()
        .unwrap();
    assert!(held.ensure_current().is_err() && renewed.ensure_current().is_err());
}

#[tokio::test]
async fn expired_lease_is_never_revived_by_a_late_renewal() {
    let issuer = Issuer::new();
    let netmap = compiled("a", "100.64.0.1", None);
    let (snapshot, renewal) = issuer.issue(&netmap, 50).unwrap();
    tokio::time::timeout(Duration::from_secs(1), snapshot.invalidated())
        .await
        .unwrap();
    assert_eq!(snapshot.ensure_current(), Err(Error::Expired));
    let (lease, deadline) = issuer.lease(&netmap, 60_000);
    assert_eq!(renewal.renew(lease, deadline), Err(Error::Expired));
    assert!(snapshot.ensure_current().is_err() && renewal.snapshot(None).is_err());
}

#[test]
fn renewal_rejects_other_content_older_issue_and_foreign_clock() {
    let issuer = Issuer::new();
    let netmap = compiled("a", "100.64.0.1", None);
    let other = compiled("b", "100.64.0.1", Some(&netmap));
    let (_, renewal) = issuer.issue(&netmap, 60_000).unwrap();
    let (lease, deadline) = issuer.lease(&other, 60_000);
    assert_eq!(renewal.renew(lease, deadline), Err(Error::Contract));
    let current = renewal.lease();
    let older = NetmapLease::new(
        netmap.reference(),
        current.issued_at_ms - 1,
        current.expires_at_ms,
    )
    .unwrap();
    let started = Started::now(&issuer.clock).unwrap();
    let deadline = Deadline::from_response(started, older.expires_at_ms).unwrap();
    assert_eq!(renewal.renew(older, deadline), Err(Error::Contract));
    let foreign = Issuer::new();
    let (lease, deadline) = foreign.lease(&netmap, 60_000);
    assert_eq!(renewal.renew(lease, deadline), Err(Error::Contract));
}

#[test]
fn issue_requires_the_exact_verified_content_and_lease_deadline() {
    let issuer = Issuer::new();
    let netmap = compiled("a", "100.64.0.1", None);
    let other = compiled("b", "100.64.0.1", Some(&netmap));
    let (lease, deadline) = issuer.lease(&other, 60_000);
    let issue = |netmap: CompiledNetmap, lease, deadline| {
        ExecutionSnapshot::issue_netmap(NetmapIssue {
            netmap: Arc::new(netmap),
            lease,
            deadline,
            instance: None,
            validity: Arc::clone(&issuer.validity),
            invalidated: issuer.invalidated.clone(),
            clock: Arc::clone(&issuer.clock),
            lineage: Arc::downgrade(&issuer.lineage),
        })
        .map(|_| ())
    };
    assert_eq!(
        issue(netmap.clone(), lease, deadline.clone()),
        Err(Error::Contract)
    );
    let mut forged = netmap.clone();
    forged.netmap.self_.hostname = "forged".into();
    let (lease, deadline) = issuer.lease(&forged, 60_000);
    assert_eq!(issue(forged, lease, deadline), Err(Error::Contract));
    let (lease, _) = issuer.lease(&netmap, 60_000);
    let (_, shorter) = issuer.lease(&netmap, 30_000);
    assert_eq!(issue(netmap.clone(), lease, shorter), Err(Error::Contract));
}

#[tokio::test]
async fn newer_serial_continues_only_with_the_same_live_own_identity() {
    let mut issuer = Issuer::new();
    let first = compiled("a", "100.64.0.1", None);
    let (old, _) = issuer.issue(&first, 60_000).unwrap();
    let second = compiled("b", "100.64.0.1", Some(&first));
    assert!(second.netmap.serial > first.netmap.serial);
    issuer.invalidate();
    let (next, _) = issuer.issue(&second, 60_000).unwrap();
    assert!(old.ensure_current().is_err() && !next.same_netmap(&old));
    assert_eq!(
        next.netmap_continuation_from(&old).unwrap().reference(),
        first.reference()
    );
    assert!(old.netmap_continuation_from(&next).is_err());
    // A same-serial re-issue is not a continuation.
    let (again, _) = issuer.issue(&second, 60_000).unwrap();
    assert!(again.netmap_continuation_from(&next).is_err());
    // Own addresses changed: hard barrier.
    let moved = compiled("b", "100.64.0.2", Some(&second));
    let (moved, _) = issuer.issue(&moved, 60_000).unwrap();
    assert!(moved.netmap_continuation_from(&next).is_err());
    // Another lineage (stop / re-enrollment) never continues.
    let other = Issuer::new();
    let third = compiled("c", "100.64.0.1", Some(&second));
    let (foreign, _) = other.issue(&third, 60_000).unwrap();
    assert!(foreign.netmap_continuation_from(&next).is_err());
    // An expired predecessor terminated everything.
    let (short, _) = issuer.issue(&second, 30).unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    let (later, _) = issuer.issue(&third, 60_000).unwrap();
    assert!(later.netmap_continuation_from(&short).is_err());
}

#[test]
fn netmap_snapshots_reject_legacy_accessors_and_bind_packet_keys() {
    let issuer = Issuer::new();
    let netmap = compiled("a", "100.64.0.1", None);
    let (snapshot, renewal) = issuer.issue(&netmap, 60_000).unwrap();
    assert!(snapshot.is_netmap());
    assert_eq!(snapshot.netmap_ref(), Some(netmap.reference()));
    assert!(matches!(snapshot.configuration(), Err(Error::Contract)));
    assert!(matches!(snapshot.reading(), Err(Error::Contract)));
    assert!(snapshot.continuation_from(&snapshot).is_err());
    assert_eq!(snapshot.instance_binding(), None);
    let instance = meshway_protocol::InstanceRequest {
        instance_id: "00000000-0000-4000-8000-0000000000aa".parse().unwrap(),
        generation: Version::MIN,
    };
    let bound = renewal.snapshot(Some(instance.clone())).unwrap();
    assert_eq!(bound.instance_binding(), Some(instance));
    assert!(bound.same_netmap(&snapshot) && !bound.same_execution(&snapshot));
    let identity = DataPlaneIdentity::new(
        snapshot.clone(),
        SigningKey::from_bytes(&[4; 32]),
        Zeroizing::new(NOISE),
    )
    .unwrap();
    identity.sign_packet(Digest::from_bytes([1; 32])).unwrap();
    assert!(
        DataPlaneIdentity::new(
            snapshot,
            SigningKey::from_bytes(&[3; 32]),
            Zeroizing::new(NOISE)
        )
        .is_err()
    );
}
