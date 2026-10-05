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
    ConfigurationRef, Digest, NetworkId, NodeId, RequestId, Sequence, StateEpoch, UdpTraversalId,
    Version,
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

wire_object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct SocketReference {
    pub node_id: NodeId,
    pub instance: InstanceRequest,
    pub configuration: ConfigurationRef,
    #[serde(deserialize_with = "socket_id")]
    pub socket_id: String [min_length = 1, max_length = 256, pattern = "^[!-~]{1,256}(?![\\s\\S])"],
    pub socket_generation: Sequence,
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

wire_object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct CandidateReference {
    pub socket: SocketReference,
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

wire_object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct StunServer {
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
        #[derive(Clone, Debug, PartialEq, Eq, Serialize)]
        #[serde(transparent)]
        pub struct $name(Vec<$item>);

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
    ScopeChanged,
    CandidateChanged,
    AttemptChanged,
    AttemptExpired,
    NotAvailable,
    Capacity,
    /// An existing requestId was retried with different immutable input.
    RequestChanged,
}
}

wire_object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct QueryUdpTraversalRequest {
    pub instance: InstanceRequest,
    pub configuration: ConfigurationRef,
} }

wire_tagged! { tag = "status";
/// A current-authority snapshot. Reading it never renews attempts or observations.
#[derive(Clone, Debug, PartialEq, Eq)] pub enum QueryUdpTraversalResponse {
    Current {
        state_epoch: StateEpoch,
        #[serde(rename = "asOf", with = "meshway_model::timestamp")]
        as_of_ms: u64 [value_type = meshway_model::timestamp::Timestamp],
        #[serde(rename = "expiresAt", with = "meshway_model::timestamp")]
        expires_at_ms: u64 [value_type = meshway_model::timestamp::Timestamp],
        stun_servers: StunServers,
        attempts: UdpTraversalAttempts,
    },
    Unavailable { reason: UdpTraversalUnavailableReason },
} }

wire_object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct PrepareUdpTraversalRequest {
    pub instance: InstanceRequest,
    pub configuration: ConfigurationRef,
    pub state_epoch: StateEpoch,
    pub request_id: RequestId,
    /// Absolute requested deadline, unchanged on retry, never a duration.
    #[serde(rename = "expiresAt", with = "meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type = meshway_model::timestamp::Timestamp],
    pub peer_id: NodeId,
    pub carrier: Carrier,
    pub local_candidate: CandidateReference,
    pub peer_candidate: CandidateReference,
} }

wire_object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct UdpTraversalParticipant {
    pub candidate: CandidateReference,
    /// Original State address, copied without origin/observer/interface disclosure.
    /// Server and Node must validate a numeric, eligible unicast UDP target.
    pub address: String,
} }

wire_object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct UdpTraversalOffer {
    pub attempt_id: UdpTraversalId,
    pub request_id: RequestId,
    pub network_id: NetworkId,
    pub state_epoch: StateEpoch,
    /// Immutable offer version; accepting does not increment it.
    pub version: Version,
    pub digest: Digest,
    /// Random 32 bytes, public attempt separation, never a credential.
    pub nonce: Digest,
    pub carrier: Carrier,
    /// Prepare caller orientation, identical for both recipients.
    pub local: UdpTraversalParticipant,
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
pub enum UdpTraversalPhase {
    Waiting,
    Active,
}
}

wire_object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct UdpTraversalAttempt {
    pub offer: UdpTraversalOffer,
    /// Active only after both actual socket receivers reserved bounded slots.
    pub phase: UdpTraversalPhase,
} }

wire_tagged! { tag = "status"; #[derive(Clone, Debug, PartialEq, Eq)] pub enum PrepareUdpTraversalResponse {
    Offer { offer: UdpTraversalOffer },
    Unavailable { reason: UdpTraversalUnavailableReason },
} }

wire_object! {
/// Shared accept/cancel CAS body. attemptId is the path parameter.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct UdpTraversalAttemptRequest {
    pub instance: InstanceRequest,
    pub configuration: ConfigurationRef,
    pub state_epoch: StateEpoch,
    pub version: Version,
    pub digest: Digest,
} }

wire_tagged! { tag = "status"; #[derive(Clone, Debug, PartialEq, Eq)] pub enum AcceptUdpTraversalResponse {
    Waiting { offer: UdpTraversalOffer },
    Active { offer: UdpTraversalOffer },
    Unavailable { reason: UdpTraversalUnavailableReason },
} }

wire_tagged! { tag = "status";
/// Cancellation ends unfinished acquisition only, never an established business link.
#[derive(Clone, Debug, PartialEq, Eq)] pub enum CancelUdpTraversalResponse {
    Cancelled,
    Unavailable { reason: UdpTraversalUnavailableReason },
} }
