//! Netmap distribution: the long-poll `POST /agent-api/v1/sync`.
//!
//! A sync request names the netmap and peer-fact cursor the Node holds. The
//! Server answers at once when either is behind; otherwise it holds the
//! request until a committed change wakes it, the peer-fact source reports a
//! change or a traversal attempt the Node does not hold yet, a newer request
//! of the same Node replaces it, or the hold times out. A hold never outlasts
//! the lease the request names (one this process issued to the Node) less
//! [`SYNC_RENEWAL_MARGIN`], nor the earliest freshness bound of the peer
//! facts the answer carrying that lease gave it less
//! [`FACTS_RENEWAL_MARGIN`]. Every answer carries a fresh lease for the current netmap. Content is
//! stored by `store::netmap` and only changes in stable-input transactions.
pub(crate) mod associations;
mod hub;
pub(crate) mod records;
pub mod signing;
mod sync;

pub(crate) use hub::Hub;
pub use sync::{NoPeerFacts, PeerFactsChanged, PeerFactsRead, PeerFactsSource};

use crate::{error::Error, execution::proof, secrets::Sealer, store::Handle};
use meshway_model::InstanceId;
use meshway_protocol::netmap::{SyncRequest, SyncResponse};
use std::sync::Arc;
use std::time::Duration;

/// Default lease validity of every sync response.
pub const DEFAULT_NETMAP_VALIDITY: Duration = Duration::from_secs(24 * 60 * 60);
/// Default nominal hold of a sync request without changes.
pub const DEFAULT_SYNC_HOLD: Duration = Duration::from_secs(50);
/// Largest configurable nominal hold; with +10% jitter a hold stays below 60 s.
pub const MAX_SYNC_HOLD: Duration = Duration::from_secs(54);
/// A held sync answers at least this long before the lease the Node holds
/// ends: the Node's largest transport backoff (15 s), so even a failed
/// answer leaves it one retry before it fails closed.
pub const SYNC_RENEWAL_MARGIN: Duration = Duration::from_secs(15);

/// A held sync answers at least this long before the earliest freshness
/// bound of the peer facts the Node holds, so a reporter's renewal, which
/// wakes no hold, reaches the Node before the fact it renews expires. It
/// covers the answer and the Node's immediate next request; a late answer
/// only shows the peer `offline` briefly (facts never refuse a flow).
pub const FACTS_RENEWAL_MARGIN: Duration = Duration::from_secs(5);

/// The hold of one request: the nominal hold, cut so the answer leaves the
/// Node [`SYNC_RENEWAL_MARGIN`] of the lease it names and
/// [`FACTS_RENEWAL_MARGIN`] of the facts that lease's answer carried. A
/// request naming no lease this process issued to it (its first request to
/// this process, or a lease it cannot prove) is answered at once.
fn hold_within(hold: Duration, held: Option<hub::Held>, now_ms: u64) -> Duration {
    let Some(held) = held else {
        return Duration::ZERO;
    };
    let before = |until: u64, margin: Duration| {
        Duration::from_millis(until.saturating_sub(now_ms)).saturating_sub(margin)
    };
    let hold = hold.min(before(held.lease_until_ms, SYNC_RENEWAL_MARGIN));
    match held.facts_until_ms {
        Some(until) => hold.min(before(until, FACTS_RENEWAL_MARGIN)),
        None => hold,
    }
}

/// Earliest freshness bound among the peer facts an answer issued at
/// `now_ms` carries (`updated` and `refreshed` together name every visible
/// fact the Node then holds; it only ever raises them). A fact within
/// [`FACTS_RENEWAL_MARGIN`] of its bound is left out: an answer now is the
/// last that could renew it, and when its reporter stopped, holding for it
/// again would only answer once more with the same bound.
fn facts_renewal_until_ms(
    facts: Option<&meshway_protocol::netmap::PeerFactsDelta>,
    now_ms: u64,
) -> Option<u64> {
    let facts = facts?;
    let floor = now_ms.saturating_add(u64::try_from(FACTS_RENEWAL_MARGIN.as_millis()).ok()?);
    facts
        .updated
        .values()
        .map(|facts| facts.expires_at)
        .chain(facts.refreshed.values().copied())
        .filter_map(|at| u64::try_from(at.timestamp_millis()).ok())
        .filter(|at| *at > floor)
        .min()
}

/// Lease validity and hold timeout, validated when the App opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncTiming {
    pub validity: Duration,
    pub hold: Duration,
}

#[cfg(test)]
mod timing_tests {
    use super::*;

    #[test]
    fn configured_validity_has_an_availability_floor_and_authority_ceiling() {
        for (seconds, valid) in [
            (59, false),
            (60, true),
            (300, true),
            (86400, true),
            (86401, false),
        ] {
            assert_eq!(
                SyncTiming {
                    validity: Duration::from_secs(seconds),
                    ..SyncTiming::default()
                }
                .valid(),
                valid
            );
        }
    }

    fn lease(until: u64) -> Option<hub::Held> {
        Some(hub::Held {
            lease_until_ms: until,
            facts_until_ms: None,
        })
    }

    #[test]
    fn a_hold_leaves_the_node_a_renewal_before_its_lease_ends() {
        let hold = Duration::from_secs(55);
        // A 60 s lease with 50 s left: answered 15 s before it ends.
        assert_eq!(
            hold_within(hold, lease(60_000), 10_000),
            Duration::from_secs(35)
        );
        assert_eq!(hold_within(hold, lease(86_400_000), 0), hold);
        assert_eq!(hold_within(hold, lease(20_000), 10_000), Duration::ZERO);
        assert_eq!(hold_within(hold, lease(5_000), 10_000), Duration::ZERO);
        assert_eq!(hold_within(hold, None, 0), Duration::ZERO);
    }

    #[test]
    fn a_hold_answers_before_the_facts_the_node_holds_expire() {
        let hold = Duration::from_secs(55);
        let held = |facts| {
            Some(hub::Held {
                lease_until_ms: 86_400_000,
                facts_until_ms: facts,
            })
        };
        // A fact delivered with 40 s left (its reporter's last 20 s report
        // was 20 s old): answered 5 s before it expires.
        assert_eq!(
            hold_within(hold, held(Some(40_000)), 0),
            Duration::from_secs(35)
        );
        // Facts that outlast the nominal hold, or none, do not shorten it.
        assert_eq!(hold_within(hold, held(Some(70_000)), 0), hold);
        assert_eq!(hold_within(hold, held(None), 0), hold);
        // The lease still bounds it when it ends first.
        assert_eq!(
            hold_within(
                hold,
                Some(hub::Held {
                    lease_until_ms: 30_000,
                    facts_until_ms: Some(40_000)
                }),
                0
            ),
            Duration::from_secs(15)
        );
    }

    #[test]
    fn the_facts_bound_skips_facts_no_renewal_can_keep_fresh() {
        use chrono::TimeZone;
        let at = |ms: i64| chrono::Utc.timestamp_millis_opt(ms).unwrap();
        let node = |n: u128| meshway_model::NodeId::new(uuid::Uuid::from_u128(n)).unwrap();
        let facts = |expires_at| meshway_protocol::netmap::PeerFacts {
            node_id: node(1),
            listeners: Default::default(),
            exit: None,
            connected: Default::default(),
            observed_at: at(0),
            expires_at,
        };
        let delta = meshway_protocol::netmap::PeerFactsDelta {
            base: None,
            cursor: meshway_protocol::netmap::StateCursor {
                epoch: meshway_model::StateEpoch::new(uuid::Uuid::from_u128(9)).unwrap(),
                sequence: meshway_model::Sequence::MIN,
            },
            netmap_serial: meshway_model::Version::new(1).unwrap(),
            updated: [(node(1), facts(at(52_000)))].into(),
            removed: vec![],
            refreshed: [(node(2), at(41_000)), (node(3), at(14_000))].into(),
        };
        // Issued at 10 s: node 3 (4 s left, its reporter stopped) can no
        // longer be renewed in time; node 2 bounds the next hold.
        assert_eq!(facts_renewal_until_ms(Some(&delta), 10_000), Some(41_000));
        // Issued at 36 s, the answer that bound was for: only node 1 is left,
        // so a stopped reporter never makes the next hold answer at once.
        assert_eq!(facts_renewal_until_ms(Some(&delta), 36_000), Some(52_000));
        assert_eq!(facts_renewal_until_ms(Some(&delta), 60_000), None);
        assert_eq!(facts_renewal_until_ms(None, 0), None);
    }
}

impl Default for SyncTiming {
    fn default() -> Self {
        Self {
            validity: DEFAULT_NETMAP_VALIDITY,
            hold: DEFAULT_SYNC_HOLD,
        }
    }
}

impl SyncTiming {
    pub(crate) fn valid(&self) -> bool {
        let validity = u64::try_from(self.validity.as_millis()).unwrap_or(u64::MAX);
        (meshway_model::netmap::MIN_NETMAP_VALIDITY_MS
            ..=meshway_model::netmap::MAX_NETMAP_VALIDITY_MS)
            .contains(&validity)
            && self.hold >= Duration::from_secs(1)
            && self.hold <= MAX_SYNC_HOLD
    }
    /// The nominal hold with uniform ±10% jitter, so Nodes that started
    /// together do not keep returning together.
    fn jittered_hold(&self) -> Duration {
        let mut sample = [0u8; 2];
        let fraction = match getrandom::fill(&mut sample) {
            Ok(()) => f64::from(u16::from_le_bytes(sample)) / f64::from(u16::MAX),
            Err(_) => 0.5,
        };
        self.hold.mul_f64(0.9 + 0.2 * fraction)
    }
}

/// Node-facing netmap distribution service.
#[derive(Clone)]
pub struct Netmaps {
    handle: Handle,
    sealer: Arc<Sealer>,
    boot: InstanceId,
    hub: Arc<Hub>,
    facts: Arc<dyn PeerFactsSource>,
    signer: Arc<signing::NetmapSigningKeys>,
    timing: SyncTiming,
    associations: associations::Shared,
}

impl Netmaps {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        handle: Handle,
        sealer: Arc<Sealer>,
        boot: InstanceId,
        hub: Arc<Hub>,
        facts: Arc<dyn PeerFactsSource>,
        signer: Arc<signing::NetmapSigningKeys>,
        timing: SyncTiming,
        associations: associations::Shared,
    ) -> Self {
        Self {
            handle,
            sealer,
            boot,
            hub,
            facts,
            signer,
            timing,
            associations,
        }
    }

    /// Complete the standing association task of the calling exit for one
    /// source query (see `associations`), then wake both Nodes' syncs.
    pub async fn report_association(
        &self,
        token: &str,
        report: meshway_protocol::DnsAssociationReport,
    ) -> Result<meshway_protocol::DnsAssociationAccepted, Error> {
        let token = proof::token_hash(token)?;
        let (sealer, boot, store) = (self.sealer.clone(), self.boot, self.associations.clone());
        let (network, accepted, woken) = self
            .handle
            .read(move |tx| {
                let now = crate::store::execution::now()?;
                let principal =
                    crate::store::execution::authorize_node(tx, &sealer, boot, token, now)?;
                let now_ms =
                    u64::try_from(now).map_err(|_| Error::Internal("system clock before epoch"))?;
                let (accepted, woken) = store.complete(
                    tx,
                    &sealer,
                    principal.network,
                    principal.node,
                    &report,
                    now_ms,
                )?;
                Ok((principal.network, accepted, woken))
            })
            .await?;
        self.hub.wake(network, &woken);
        Ok(accepted)
    }

    /// Holds the request until something the Node lacks exists, then answers
    /// with the current lease; see the module documentation.
    pub async fn sync(&self, token: &str, request: SyncRequest) -> Result<SyncResponse, Error> {
        let token = proof::token_hash(token)?;
        let principal = {
            let (sealer, boot) = (self.sealer.clone(), self.boot);
            self.handle
                .read(move |tx| {
                    crate::store::execution::authorize_node(
                        tx,
                        &sealer,
                        boot,
                        token,
                        crate::store::execution::now()?,
                    )
                })
                .await?
        };
        records::Records::shared().received(
            principal.network,
            principal.node,
            chrono::Utc::now().timestamp_millis().max(0) as u64,
            request.server_key_generation,
        );
        let answered = |response: SyncResponse| {
            records::Records::shared().issued(principal.network, principal.node, response.lease);
            self.hub.issued(
                principal.network,
                principal.node,
                response.lease,
                facts_renewal_until_ms(response.peer_facts.as_ref(), response.lease.issued_at_ms),
            );
            response
        };
        // Register before the first read: a change committed after this point
        // wakes the waiter, one committed before it is visible to the read.
        let mut waiter = self.hub.register(principal.network, principal.node)?;
        // Only the lease the request names, checked against those this
        // process issued, and the facts its answer carried bound the hold:
        // an answer generated earlier may never have reached the Node.
        let deadline = tokio::time::Instant::now()
            + hold_within(
                self.timing.jittered_hold(),
                self.hub
                    .held(principal.network, principal.node, request.lease.as_ref()),
                chrono::Utc::now().timestamp_millis().max(0) as u64,
            );
        loop {
            let (response, changed) = self.read(token, &request).await?;
            if !matches!(
                response.netmap,
                meshway_protocol::netmap::SyncNetmap::Unchanged
            ) || response
                .peer_facts
                .as_ref()
                .is_some_and(|facts| facts.base.is_none() || Some(facts.cursor) != request.state)
                || response.associations.is_some()
                || response.traversal_requests.as_slice().iter().any(|asked| {
                    !request
                        .traversal_requests
                        .as_slice()
                        .contains(&asked.attempt_id)
                })
            {
                return Ok(answered(response));
            }
            let facts_changed = async move {
                match changed {
                    Some(changed) => changed.await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                _ = waiter.woken() => break,
                () = facts_changed => continue,
                () = tokio::time::sleep_until(deadline) => break,
            }
        }
        // Woken, replaced, closing or timed out: one final answer with a fresh lease.
        let response = self.read(token, &request).await?.0;
        Ok(answered(response))
    }

    /// Bytes of a rule-set artifact some netmap of the calling Node
    /// references while it can still be under a lease. Anything else, and an
    /// unknown digest, is `NotFound`.
    pub async fn artifact(&self, token: &str, digest: String) -> Result<Vec<u8>, Error> {
        let token = proof::token_hash(token)?;
        let digest: meshway_rules::Digest = digest.parse().map_err(|_| Error::NotFound)?;
        let (sealer, boot) = (self.sealer.clone(), self.boot);
        let validity = i64::try_from(self.timing.validity.as_millis())
            .map_err(|_| Error::Internal("netmap validity"))?;
        self.handle
            .read(move |tx| {
                let now = crate::store::execution::now()?;
                let principal =
                    crate::store::execution::authorize_node(tx, &sealer, boot, token, now)?;
                if !crate::store::resources::artifacts::referenced(
                    tx,
                    principal.network.as_uuid().as_bytes(),
                    principal.node.as_uuid().as_bytes(),
                    &digest,
                    now,
                    validity,
                )? {
                    return Err(Error::NotFound);
                }
                crate::store::resources::artifacts::load(tx, &digest)
            })
            .await
    }

    async fn read(
        &self,
        token: [u8; 32],
        request: &SyncRequest,
    ) -> Result<(SyncResponse, Option<PeerFactsChanged>), Error> {
        let (sealer, boot, facts, signer, validity, associations) = (
            self.sealer.clone(),
            self.boot,
            self.facts.clone(),
            self.signer.clone(),
            self.timing.validity,
            self.associations.clone(),
        );
        let request = request.clone();
        self.handle
            .read(move |tx| {
                sync::read(
                    tx,
                    &sealer,
                    boot,
                    token,
                    &request,
                    facts.as_ref(),
                    &sync::Issue {
                        signer: &signer,
                        validity,
                        associations: &associations,
                    },
                )
            })
            .await
    }
}
