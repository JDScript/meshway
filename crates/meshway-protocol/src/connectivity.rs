//! Short-lived UDP acquisition contracts. State remains the candidate owner.
//!
//! References and STUN metadata neither prove address ownership nor authorize
//! business packets. An active attempt permits only bounded acquisition probes;
//! installation still requires a fresh signed source challenge, current exact
//! carrier/business authority, and the original complete logical path.

use crate::{
    InstanceRequest,
    object::{wire_object, wire_tagged},
};
use meshway_configuration::{execution::Carrier, transport::Endpoint};
use meshway_model::{
    Digest, NetmapRef, NetworkId, NodeId, RequestId, Sequence, StateEpoch, UdpTraversalId,
    Version,
    observations::EndpointKind,
    state::{EndpointCandidate, ResourceRef, SocketNetwork},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{fmt, net::SocketAddr};

pub const MAX_STUN_SERVERS: usize = 4;
/// Per-node live attempts, including waiting reservations.
pub const MAX_UDP_TRAVERSAL_ATTEMPTS: usize = 32;
/// Absolute offers and request-id deduplication never outlive this window.
pub const MAX_UDP_TRAVERSAL_LIFETIME_MS: u64 = 10_000;
pub const MAX_SOCKET_ID_BYTES: usize = 256;

wire_object! {
/// Exact Node-reported socket binding checked against the current instance and execution target.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct SocketReference {
    /// Node that owns and reports the actual socket.
    pub node_id: NodeId,
    /// Owning runtime instance and its Server-issued generation.
    pub instance: InstanceRequest,
    /// Exact installed netmap associated with the socket.
    pub netmap: NetmapRef,
    /// Node-assigned socket identifier of 1–256 printable non-space ASCII bytes.
    #[serde(deserialize_with = "socket_id")]
    pub socket_id: String [min_length = 1, max_length = 256, pattern = "^[!-~]{1,256}(?![\\s\\S])"],
    /// Socket incarnation within the runtime instance, preventing reuse of stale references.
    pub socket_generation: Sequence,
    /// Configured resource for which this socket was created.
    pub resource: ResourceRef,
} }

fn socket_id<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = String::deserialize(d)?;
    if !valid_socket_id(&value) {
        return Err(serde::de::Error::custom("invalid socket identifier"));
    }
    Ok(value)
}

fn valid_socket_id(value: &str) -> bool {
    // The schema's final negative lookahead is an exact end anchor in JS too;
    // `$` alone also accepts a position before a trailing line terminator.
    !value.is_empty()
        && value.len() <= MAX_SOCKET_ID_BYTES
        && value.bytes().all(|byte| (b'!'..=b'~').contains(&byte))
}

fn stun_server_id<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = String::deserialize(d)?;
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(serde::de::Error::custom("invalid STUN server identifier"));
    }
    Ok(value)
}

wire_object! {
/// Immutable reference to an original State-owned endpoint observation without disclosing its private source details.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct CandidateReference {
    /// Exact socket that produced and will use the candidate.
    pub socket: SocketReference,
    /// Digest of the original address, source, observation lifetime and socket binding, excluding report sequence and receipt time.
    pub observation_digest: Digest,
} }

impl CandidateReference {
    /// Use the original State observation, before peer redaction. The caller
    /// resolves inherited socket configuration to the exact installed reference.
    /// No report receipt/sequence participates, so rereading or re-reporting the
    /// same observation cannot refresh its identity or its original expiry.
    pub fn from_observation(
        socket: SocketReference,
        network: SocketNetwork,
        bound_address: SocketAddr,
        candidate: &EndpointCandidate,
    ) -> Result<Self, serde_json::Error> {
        let observation_digest =
            candidate_observation_digest(&socket, network, bound_address, candidate)?;
        Ok(Self {
            socket,
            observation_digest,
        })
    }
}

/// Hash exact socket binding and the original (unredacted) observation. This
/// domain-separated encoding contains only typed structs/enums, never maps.
/// The bind address and origin details are committed but not disclosed by the
/// returned peer reference. This helper does not validate candidate authority.
pub fn candidate_observation_digest(
    socket: &SocketReference,
    network: SocketNetwork,
    bound_address: SocketAddr,
    candidate: &EndpointCandidate,
) -> Result<Digest, serde_json::Error> {
    if !valid_socket_id(&socket.socket_id) {
        return Err(<serde_json::Error as serde::ser::Error>::custom(
            "invalid socket identifier",
        ));
    }
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Observation<'a> {
        socket: &'a SocketReference,
        network: SocketNetwork,
        bound_address: SocketAddr,
        candidate: &'a EndpointCandidate,
    }
    canonical_digest(
        b"meshway/udp-candidate-observation/v1\0",
        &Observation {
            socket,
            network,
            bound_address,
            candidate,
        },
    )
}

fn canonical_digest<T: Serialize>(domain: &[u8], value: &T) -> Result<Digest, serde_json::Error> {
    let encoded = serde_json::to_vec(value)?;
    let mut hash = Sha256::new();
    hash.update(domain);
    hash.update(encoded);
    Ok(Digest::from_bytes(hash.finalize().into()))
}

wire_object! {
/// Explicit STUN discovery endpoint published by the Server operator.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct StunServer {
    /// Operator metadata identifier containing 1–64 ASCII letters, digits, underscores or hyphens.
    #[serde(deserialize_with = "stun_server_id")]
    pub id: String [min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9_-]{1,64}(?![\\s\\S])"],
    /// Explicit operator metadata; a hostname is allowed here, not in probe targets.
    pub endpoint: Endpoint,
} }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConnectivityLimitExceeded {
    pub maximum: usize,
}

impl fmt::Display for ConnectivityLimitExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "connectivity collection exceeds {} items", self.maximum)
    }
}
impl std::error::Error for ConnectivityLimitExceeded {}

// Concrete wrappers keep bounds in both the runtime value and its wire schema.
macro_rules! bounded_collection {
    ($name:ident, $item:ty, $max:ident) => {
        /// Bounded Server response collection whose maximum item count is enforced during decoding.
        #[derive(Clone, Debug, PartialEq, Eq, Serialize)]
        #[serde(transparent)]
        pub struct $name(
            /// Returned entries serialized directly as the bounded array.
            Vec<$item>,
        );

        impl $name {
            pub fn as_slice(&self) -> &[$item] {
                &self.0
            }
            pub fn into_vec(self) -> Vec<$item> {
                self.0
            }
        }
        impl TryFrom<Vec<$item>> for $name {
            type Error = ConnectivityLimitExceeded;
            fn try_from(value: Vec<$item>) -> Result<Self, Self::Error> {
                if value.len() > $max {
                    Err(ConnectivityLimitExceeded { maximum: $max })
                } else {
                    Ok(Self(value))
                }
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct Visitor;
                impl<'de> serde::de::Visitor<'de> for Visitor {
                    type Value = $name;
                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        write!(f, "an array of at most {} items", $max)
                    }
                    fn visit_seq<A: serde::de::SeqAccess<'de>>(
                        self,
                        mut seq: A,
                    ) -> Result<Self::Value, A::Error> {
                        let mut items = Vec::new();
                        while let Some(item) = seq.next_element()? {
                            if items.len() == $max {
                                return Err(serde::de::Error::custom(ConnectivityLimitExceeded {
                                    maximum: $max,
                                }));
                            }
                            items.push(item);
                        }
                        Ok($name(items))
                    }
                }
                d.deserialize_seq(Visitor)
            }
        }
        impl utoipa::PartialSchema for $name {
            fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
                let mut schema = <Vec<$item> as utoipa::PartialSchema>::schema();
                if let utoipa::openapi::RefOr::T(utoipa::openapi::schema::Schema::Array(array)) =
                    &mut schema
                {
                    array.max_items = Some($max);
                }
                schema
            }
        }
        impl utoipa::ToSchema for $name {
            fn schemas(
                schemas: &mut Vec<(
                    String,
                    utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>,
                )>,
            ) {
                <Vec<$item> as utoipa::ToSchema>::schemas(schemas);
            }
        }
    };
}

bounded_collection!(StunServers, StunServer, MAX_STUN_SERVERS);
bounded_collection!(
    UdpTraversalAttempts,
    UdpTraversalAttempt,
    MAX_UDP_TRAVERSAL_ATTEMPTS
);

meshway_serde::string_enum! {
/// Expected authority/CAS races are closed outcomes, not authentication errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum UdpTraversalUnavailableReason {
    /// The instance, execution target, authority deadline or State epoch no longer matches.
    ScopeChanged,
    /// The referenced State candidate or its original socket observation is no longer current.
    CandidateChanged,
    /// The submitted offer version or digest differs from the stored immutable offer.
    AttemptChanged,
    /// The attempt reached its original absolute deadline.
    AttemptExpired,
    /// No eligible attempt, candidate pair or shared authorized UDP adjacency is available.
    NotAvailable,
    /// A bounded attempt or coordination resource limit prevents admission.
    Capacity,
    /// An existing requestId was retried with different immutable input.
    RequestChanged,
}
}

wire_object! {
/// Node query for current STUN metadata and its own valid UDP acquisition attempts without renewing them.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct QueryUdpTraversalRequest {
    /// Current runtime instance making the query.
    pub instance: InstanceRequest,
    /// Exact current netmap against which acquisition scope is checked.
    pub netmap: NetmapRef,
} }

wire_tagged! { tag = "status";
/// A current-authority snapshot. Reading it never renews attempts or observations.
#[derive(Clone, Debug, PartialEq, Eq)] pub enum QueryUdpTraversalResponse {
    /// Current coordination metadata under the caller's checked execution scope.
    Current {
        /// State continuity epoch required by subsequent prepare, accept and cancel requests.
        state_epoch: StateEpoch,
        /// Server evaluation time in milliseconds since the Unix epoch, serialized as UTC RFC 3339.
        #[serde(rename = "asOf", with = "meshway_model::timestamp")]
        as_of_ms: u64 [value_type = meshway_model::timestamp::Timestamp],
        /// Metadata deadline in milliseconds since the Unix epoch, serialized as UTC RFC 3339 and bounded by current authority and 30 seconds.
        #[serde(rename = "expiresAt", with = "meshway_model::timestamp")]
        expires_at_ms: u64 [value_type = meshway_model::timestamp::Timestamp],
        /// Explicitly configured STUN endpoints; an empty list supplies no discovery server.
        stun_servers: StunServers,
        /// Valid nonterminal attempts involving the authenticated Node.
        attempts: UdpTraversalAttempts,
    },
    /// Current scope or coordination availability prevents returning usable metadata.
    Unavailable {
        /// Closed coordination outcome requiring the caller to reevaluate acquisition.
        reason: UdpTraversalUnavailableReason
    },
} }

wire_object! {
/// Node request for an immutable short-lived offer bound to an exact authorized UDP carrier and candidate pair.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct PrepareUdpTraversalRequest {
    /// Current source runtime instance preparing the attempt.
    pub instance: InstanceRequest,
    /// Source's exact current netmap.
    pub netmap: NetmapRef,
    /// State epoch obtained from current coordination metadata.
    pub state_epoch: StateEpoch,
    /// Stable identifier reused with identical immutable input when retrying preparation.
    pub request_id: RequestId,
    /// Absolute requested deadline, unchanged on retry, never a duration.
    #[serde(rename = "expiresAt", with = "meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type = meshway_model::timestamp::Timestamp],
    /// Adjacent peer participating in the authorized UDP acquisition.
    pub peer_id: NodeId,
    /// Exact Native UDP carrier authorized by both participants' complete-path permits.
    pub carrier: Carrier,
    /// Original State-owned candidate from the preparing Node's actual socket.
    pub local_candidate: CandidateReference,
    /// Original State-owned candidate from the peer's actual socket.
    pub peer_candidate: CandidateReference,
} }

wire_object! {
/// Server projection of one offer participant's fixed candidate and numeric probe destination.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct UdpTraversalParticipant {
    /// Exact State observation and socket binding committed by the offer.
    pub candidate: CandidateReference,
    /// Original State address, copied without origin/observer/interface disclosure.
    /// Server and Node must validate a numeric, eligible unicast UDP target.
    pub address: String,
    /// Coarse origin projected from the referenced original State observation.
    pub source: EndpointKind,
} }

wire_object! {
/// Server-issued immutable candidate pair for bounded UDP acquisition, valid only until its original deadline.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct UdpTraversalOffer {
    /// Server-assigned acquisition attempt identifier used by accept and cancel.
    pub attempt_id: UdpTraversalId,
    /// Original preparation request identifier retained for idempotent retries.
    pub request_id: RequestId,
    /// Network containing both authenticated participants.
    pub network_id: NetworkId,
    /// State continuity epoch under which the original candidates were selected.
    pub state_epoch: StateEpoch,
    /// Immutable offer version; accepting does not increment it.
    pub version: Version,
    /// Domain-separated digest committing every immutable offer field except this digest itself.
    pub digest: Digest,
    /// Random 32 bytes, public attempt separation, never a credential.
    pub nonce: Digest,
    /// Exact authorized Native UDP adjacency, independent of which participant prepared the offer.
    pub carrier: Carrier,
    /// Prepare caller orientation, identical for both recipients.
    pub local: UdpTraversalParticipant,
    /// Remote participant relative to the original prepare caller, unchanged for both recipients.
    pub peer: UdpTraversalParticipant,
    /// Fixed minimum of requested/max-window and every authority/fact deadline.
    #[serde(rename = "expiresAt", with = "meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type = meshway_model::timestamp::Timestamp],
} }

impl UdpTraversalOffer {
    /// Commits every immutable field except digest itself. Phase/acceptance and
    /// query times are deliberately excluded. Callers compare this with digest.
    pub fn compute_digest(&self) -> Result<Digest, serde_json::Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Offer<'a> {
            attempt_id: UdpTraversalId,
            request_id: RequestId,
            network_id: NetworkId,
            state_epoch: StateEpoch,
            version: Version,
            nonce: Digest,
            carrier: &'a Carrier,
            local: &'a UdpTraversalParticipant,
            peer: &'a UdpTraversalParticipant,
            expires_at_ms: u64,
        }
        canonical_digest(
            b"meshway/udp-traversal-offer/v1\0",
            &Offer {
                attempt_id: self.attempt_id,
                request_id: self.request_id,
                network_id: self.network_id,
                state_epoch: self.state_epoch,
                version: self.version,
                nonce: self.nonce,
                carrier: &self.carrier,
                local: &self.local,
                peer: &self.peer,
                expires_at_ms: self.expires_at_ms,
            },
        )
    }
}

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
/// Server-recorded bilateral acceptance phase, distinct from an authenticated channel being established.
pub enum UdpTraversalPhase {
    /// At least one participant has not yet accepted the fixed offer.
    Waiting,
    /// Both participants accepted the offer after reserving their actual receiver slots.
    Active,
}
}

wire_object! {
/// Current acceptance state paired with the attempt's unchanged Server-issued offer.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct UdpTraversalAttempt {
    /// Immutable candidate pair, authority binding and original acquisition deadline.
    pub offer: UdpTraversalOffer,
    /// Active only after both actual socket receivers reserved bounded slots.
    pub phase: UdpTraversalPhase,
} }

wire_tagged! { tag = "status";
/// Server result of preparing or recovering an immutable UDP acquisition offer.
#[derive(Clone, Debug, PartialEq, Eq)] pub enum PrepareUdpTraversalResponse {
    /// A fixed offer is available for bilateral acceptance.
    Offer {
        /// Original offer, including its unchanged deadline on an identical retry.
        offer: UdpTraversalOffer
    },
    /// Acquisition cannot be prepared under the supplied bindings or current limits.
    Unavailable {
        /// Closed coordination condition that prevented preparation.
        reason: UdpTraversalUnavailableReason
    },
} }

wire_object! {
/// Shared accept/cancel CAS body. attemptId is the path parameter.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct UdpTraversalAttemptRequest {
    /// Current participant runtime instance performing the operation.
    pub instance: InstanceRequest,
    /// Participant's exact current netmap, matching its offer socket binding.
    pub netmap: NetmapRef,
    /// State epoch in which the offer was prepared.
    pub state_epoch: StateEpoch,
    /// Immutable offer version required by the compare-and-swap check.
    pub version: Version,
    /// Immutable offer digest required by the compare-and-swap check.
    pub digest: Digest,
} }

wire_tagged! { tag = "status";
/// Server result of accepting the exact offer after the caller reserves its actual UDP receiver slot.
#[derive(Clone, Debug, PartialEq, Eq)] pub enum AcceptUdpTraversalResponse {
    /// The caller accepted and the other participant has not yet accepted.
    Waiting {
        /// Unchanged offer awaiting the peer's acceptance before its deadline.
        offer: UdpTraversalOffer
    },
    /// Both participants accepted, allowing bounded acquisition under the original offer.
    Active {
        /// Unchanged offer whose bindings and deadline constrain acquisition probes.
        offer: UdpTraversalOffer
    },
    /// The offer cannot be accepted under its current bindings or lifecycle.
    Unavailable {
        /// Closed coordination condition that prevented acceptance.
        reason: UdpTraversalUnavailableReason
    },
} }

wire_tagged! { tag = "status";
/// Cancellation ends unfinished acquisition only, never an established business link.
#[derive(Clone, Debug, PartialEq, Eq)] pub enum CancelUdpTraversalResponse {
    /// The unfinished acquisition is cancelled, including an identical cancellation retry.
    Cancelled,
    /// The attempt could not be cancelled under the supplied current bindings.
    Unavailable {
        /// Closed coordination condition that prevented cancellation.
        reason: UdpTraversalUnavailableReason
    },
} }
