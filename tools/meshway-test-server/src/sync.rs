//! Fixture netmap authority: the fixed signing key, signed sync replies and a
//! scripted sync transport for the production `Control`.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use meshway_configuration::netmap::CompiledNetmap;
use meshway_control::{KnownPeers, PeerFactsSink, SyncFuture, SyncTransport};
use meshway_control_client::ClientError;
use meshway_model::{
    KeyAlgorithm, NetmapLease, NetmapRef, NetworkId, NodeId, NodePublicKey, Version,
};
use meshway_protocol::{
    ErrorResponse, NetmapSignature, NetmapSigner, ServerSigningKey, SyncNetmap, SyncRequest,
    SyncResponse, netmap_lease_transcript,
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

/// The fixed fixture signing key and its public binding.
pub fn server_key() -> (SigningKey, ServerSigningKey) {
    let secret = SigningKey::from_bytes(&[42; 32]);
    let public = ServerSigningKey {
        key_id: "00000000-0000-4009-8000-000000000001"
            .parse()
            .expect("fixture key ID"),
        key_generation: Version::MIN,
        public_key: NodePublicKey {
            algorithm: KeyAlgorithm::Ed25519,
            bytes: secret.verifying_key().to_bytes().into(),
        },
    };
    (secret, public)
}

/// Signature of `lease` for one Node by an explicit key.
pub fn sign_with(
    secret: &SigningKey,
    key: &ServerSigningKey,
    network: NetworkId,
    node: NodeId,
    lease: &NetmapLease,
) -> NetmapSignature {
    let message = netmap_lease_transcript(
        network,
        node,
        lease,
        NetmapSigner::Server,
        key.key_id,
        key.key_generation,
    );
    NetmapSignature {
        key_id: key.key_id,
        key_generation: key.key_generation,
        signature: URL_SAFE_NO_PAD.encode(secret.sign(&message).to_bytes()),
    }
}

/// Signature of `lease` for one Node by the fixture key.
pub fn sign(network: NetworkId, node: NodeId, lease: &NetmapLease) -> NetmapSignature {
    let (secret, key) = server_key();
    sign_with(&secret, &key, network, node, lease)
}

/// Current wall time in Unix milliseconds.
pub fn now_ms() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).expect("time after 1970")
}

/// A lease for `netmap` starting now.
pub fn lease(netmap: NetmapRef, validity_ms: u64) -> NetmapLease {
    let now = now_ms();
    NetmapLease::new(netmap, now, now + validity_ms).expect("valid fixture lease")
}

/// A fixture-signed response without peer facts.
pub fn signed(
    network: NetworkId,
    node: NodeId,
    lease: NetmapLease,
    netmap: SyncNetmap,
) -> SyncResponse {
    SyncResponse {
        signature: sign(network, node, &lease),
        lease,
        netmap,
        peer_facts: None,
        key_rotation: None,
        traversal_requests: Default::default(),
        associations: None,
    }
}

/// A fixture-signed DNS association overlay update for one Node.
pub fn sign_associations(
    network: NetworkId,
    node: NodeId,
    overlay: meshway_configuration::netmap::DnsAssociationOverlay,
) -> meshway_protocol::DnsAssociationsUpdate {
    let (secret, key) = server_key();
    let digest = overlay.digest().expect("overlay digest");
    let message = meshway_protocol::dns_associations_transcript(
        network,
        node,
        &overlay.netmap,
        &digest,
        key.key_id,
        key.key_generation,
    );
    meshway_protocol::DnsAssociationsUpdate {
        overlay,
        signature: NetmapSignature {
            key_id: key.key_id,
            key_generation: key.key_generation,
            signature: URL_SAFE_NO_PAD.encode(secret.sign(&message).to_bytes()),
        },
        rotation_signature: None,
    }
}

/// A fixture-signed full (`full`) or unchanged reply for `netmap`'s Node.
pub fn reply(netmap: &CompiledNetmap, validity_ms: u64, full: bool) -> SyncResponse {
    signed(
        netmap.netmap.network_id,
        netmap.netmap.node_id,
        lease(netmap.reference(), validity_ms),
        if full {
            SyncNetmap::Full {
                netmap: Box::new(netmap.clone()),
            }
        } else {
            SyncNetmap::Unchanged
        },
    )
}

pub type Reply = Result<SyncResponse, ClientError<ErrorResponse>>;

/// Sync replies in order; records each request. An empty script behaves like
/// an unreachable Server.
#[derive(Default)]
pub struct Script {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<SyncRequest>>,
}

impl Script {
    pub fn new() -> Arc<Self> {
        Arc::default()
    }

    pub fn push(&self, reply: Reply) {
        self.replies.lock().expect("script lock").push_back(reply);
    }

    pub fn requests(&self) -> Vec<SyncRequest> {
        self.requests.lock().expect("script lock").clone()
    }

    pub fn calls(&self) -> usize {
        self.requests.lock().expect("script lock").len()
    }
}

impl SyncTransport for Script {
    fn sync<'a>(&'a self, request: &'a SyncRequest, token: &'a str) -> SyncFuture<'a> {
        assert!(!token.is_empty(), "sync carries the Node session");
        self.requests
            .lock()
            .expect("script lock")
            .push(request.clone());
        let reply = self.replies.lock().expect("script lock").pop_front();
        Box::pin(async move { reply.unwrap_or(Err(ClientError::Network { status: None })) })
    }
}

/// A peer-fact sink that records every publication of Control's facts.
#[derive(Default)]
pub struct Facts {
    pub published: Vec<KnownPeers>,
}

impl PeerFactsSink for Facts {
    fn publish(&mut self, facts: KnownPeers) {
        self.published.push(facts);
    }
}
