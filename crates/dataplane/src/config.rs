use crate::{Error, Identity, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fmt, net::SocketAddr};
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct PrivateKey(pub(crate) Zeroizing<[u8; 32]>);

impl PrivateKey {
    pub fn from_hex(value: &str) -> Result<Self> {
        let mut bytes = Zeroizing::new([0; 32]);
        hex::decode_to_slice(value, bytes.as_mut())
            .map_err(|_| Error::InvalidConfig("invalid private key"))?;
        Ok(Self(bytes))
    }

    /// Explicit secret export for private configuration persistence only.
    pub fn to_hex(&self) -> String {
        hex::encode(self.0.as_ref())
    }
}

impl fmt::Debug for PrivateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PrivateKey([REDACTED])")
    }
}

impl Serialize for PrivateKey {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let encoded = Zeroizing::new(self.to_hex());
        serializer.serialize_str(&encoded)
    }
}

impl<'de> Deserialize<'de> for PrivateKey {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Zeroizing::new(String::deserialize(deserializer)?);
        Self::from_hex(&value).map_err(|_| serde::de::Error::custom("invalid private key"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    NativeUdp,
    NativeTcp,
    Vless,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerConfig {
    pub node_id: String,
    pub public_key: String,
    /// Allowed hop carriers. Empty (including omission) means endpoint identity only.
    #[serde(default)]
    pub transports: Vec<TransportKind>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListenerConfig {
    pub transport: TransportKind,
    pub bind: SocketAddr,
    #[serde(default)]
    pub id: Option<String>,
    /// None retains explicit legacy carrier grants; Some([]) denies all peers.
    #[serde(default)]
    pub allowed_peers: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutgoingCandidate {
    pub peer: String,
    pub transport: TransportKind,
    pub endpoint: String,
    #[serde(default)]
    pub priority: u16,
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Limits {
    /// Combined live channels and unfinished hop handshakes/confirmations.
    pub max_connections: usize,
    pub queue_capacity: usize,
    /// Completed UDP/dispatch frames between cooperative yields. Zero disables
    /// explicit yielding for same-build A/B comparisons; it does not change queues.
    pub work_budget_frames: usize,
    /// Requested SO_RCVBUF for every native UDP socket. Zero preserves the OS
    /// default; a nonzero request may be clamped/accounted differently by the OS.
    pub udp_receive_buffer_bytes: usize,
    /// Disable Nagle on native TCP dialed AND accepted sockets. False retains
    /// the old OS-default behavior for same-binary performance comparisons.
    pub native_tcp_nodelay: bool,
    /// L bounds each source setup/cache pool. Receiving packet contexts are <= L,
    /// each with one active and one pending key, plus <= L separate echo setups:
    /// at most 3L receiving E2E states, without unrelated-context rollover borrowing.
    pub max_pending_probes: usize,
    pub handshake_timeout_ms: u64,
    pub probe_timeout_ms: u64,
    pub reconnect_delay_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_connections: 32,
            queue_capacity: 64,
            work_budget_frames: 16,
            udp_receive_buffer_bytes: 0,
            native_tcp_nodelay: true,
            max_pending_probes: 64,
            handshake_timeout_ms: 3_000,
            probe_timeout_ms: 10_000,
            reconnect_delay_ms: 500,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabConfig {
    pub node_id: String,
    pub private_key: PrivateKey,
    pub public_key: String,
    pub trusted_peers: Vec<PeerConfig>,
    pub allowed_paths: Vec<Vec<String>>,
    #[serde(default)]
    pub listeners: Vec<ListenerConfig>,
    #[serde(default)]
    pub outgoing: Vec<OutgoingCandidate>,
    #[serde(default)]
    pub limits: Limits,
}

pub(crate) fn public_key(value: &str) -> Result<[u8; 32]> {
    let mut result = [0; 32];
    hex::decode_to_slice(value, &mut result)
        .map_err(|_| Error::InvalidConfig("invalid public key"))?;
    if result == [0; 32] {
        return Err(Error::InvalidConfig("invalid public key"));
    }
    Ok(result)
}

pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub(crate) fn validate_context_id(id: Option<&str>) -> Result<()> {
    if id.is_some_and(|id| {
        id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
    }) {
        return Err(Error::InvalidConfig("invalid local context id"));
    }
    Ok(())
}

impl LabConfig {
    pub fn validate(&self) -> Result<()> {
        if !valid_id(&self.node_id) {
            return Err(Error::InvalidConfig("invalid node id"));
        }
        let identity = Identity::from_private_key(self.private_key.clone());
        if public_key(&self.public_key)? != identity.public_key_bytes() {
            return Err(Error::InvalidConfig("private/public key mismatch"));
        }
        if self.trusted_peers.len() > 64
            || self.allowed_paths.len() > 128
            || self.listeners.len() > 8
            || self.outgoing.len() > 32
        {
            return Err(Error::InvalidConfig("configuration count limit"));
        }
        let mut ids = BTreeSet::from([self.node_id.as_str()]);
        let mut keys = BTreeSet::from([identity.public_key_bytes()]);
        for peer in &self.trusted_peers {
            if !valid_id(&peer.node_id)
                || !ids.insert(peer.node_id.as_str())
                || !keys.insert(public_key(&peer.public_key)?)
            {
                return Err(Error::InvalidConfig("duplicate or invalid peer identity"));
            }
            if peer.transports.len() > 3
                || peer.transports.iter().collect::<BTreeSet<_>>().len() != peer.transports.len()
            {
                return Err(Error::InvalidConfig("duplicate or invalid peer transports"));
            }
        }
        let mut paths = BTreeSet::new();
        for path in &self.allowed_paths {
            if !(2..=8).contains(&path.len())
                || !path.contains(&self.node_id)
                || path.iter().collect::<BTreeSet<_>>().len() != path.len()
                || path.iter().any(|id| !ids.contains(id.as_str()))
            {
                return Err(Error::InvalidConfig("invalid authorized path"));
            }
            if !paths.insert(path) {
                return Err(Error::InvalidConfig("duplicate authorized path"));
            }
        }
        let mut listener_ids = BTreeSet::new();
        for listener in &self.listeners {
            validate_context_id(listener.id.as_deref())?;
            if listener
                .id
                .as_ref()
                .is_some_and(|id| !listener_ids.insert(id))
            {
                return Err(Error::InvalidConfig("duplicate listener id"));
            }
            self.validate_listener_peers(listener.allowed_peers.as_deref(), listener.transport)?;
            if listener.transport == TransportKind::Vless {
                return Err(Error::InvalidConfig(
                    "VLESS listeners must be attached externally",
                ));
            }
        }
        let mut candidates = BTreeSet::new();
        let mut candidate_ids = BTreeSet::new();
        for candidate in &self.outgoing {
            validate_context_id(candidate.id.as_deref())?;
            if candidate
                .id
                .as_ref()
                .is_some_and(|id| !candidate_ids.insert(id))
            {
                return Err(Error::InvalidConfig("duplicate candidate id"));
            }
            if candidate.peer == self.node_id
                || !ids.contains(candidate.peer.as_str())
                || candidate.endpoint.len() > 256
                || candidate.endpoint.is_empty()
                || !candidates.insert((&candidate.peer, candidate.transport, &candidate.endpoint))
            {
                return Err(Error::InvalidConfig("invalid outgoing candidate"));
            }
            if !self.trusted_peers.iter().any(|peer| {
                peer.node_id == candidate.peer && peer.transports.contains(&candidate.transport)
            }) {
                return Err(Error::InvalidConfig(
                    "outgoing carrier not authorized for peer",
                ));
            }
            if candidate.transport != TransportKind::Vless
                && candidate.endpoint.parse::<SocketAddr>().is_err()
            {
                return Err(Error::InvalidConfig(
                    "native endpoint must be a socket address",
                ));
            }
        }
        let limits = &self.limits;
        if !(1..=128).contains(&limits.max_connections)
            || !(1..=256).contains(&limits.queue_capacity)
            || limits.work_budget_frames > 64
            || limits.udp_receive_buffer_bytes > 4 * 1024 * 1024
            || !(1..=128).contains(&limits.max_pending_probes)
            || !(50..=30_000).contains(&limits.handshake_timeout_ms)
            || !(100..=60_000).contains(&limits.probe_timeout_ms)
            || !(50..=30_000).contains(&limits.reconnect_delay_ms)
        {
            return Err(Error::InvalidConfig("invalid runtime limits"));
        }
        Ok(())
    }

    pub(crate) fn validate_listener_peers(
        &self,
        allowed_peers: Option<&[String]>,
        transport: TransportKind,
    ) -> Result<()> {
        if let Some(peers) = allowed_peers {
            let mut unique = BTreeSet::new();
            if peers.len() > self.trusted_peers.len()
                || peers.iter().any(|id| {
                    !unique.insert(id)
                        || !self
                            .trusted_peers
                            .iter()
                            .any(|peer| &peer.node_id == id && peer.transports.contains(&transport))
                })
            {
                return Err(Error::InvalidConfig("invalid listener peer grant"));
            }
        }
        Ok(())
    }
}
