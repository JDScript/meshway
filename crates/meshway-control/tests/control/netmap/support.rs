//! Real one-Node netmaps for an enrolled identity and a scripted sync transport.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use meshway_configuration::{
    AddressPools, Input, NodeAddresses, NodeIdentity, NodeSettings, SubjectContext,
    execution::TrustedNode, netmap::CompiledNetmap,
};
use meshway_control::{FactsRejected, PeerFactsSink, SyncFuture, SyncTransport};
use meshway_control_client::ClientError;
use meshway_model::{KeyAlgorithm, NetmapLease, NetmapRef, NodePublicKey, Revision, Version};
use meshway_policy::compile::netmap::{NetmapInput, PreviousNetmap, compile_netmap};
use meshway_protocol::{
    EnrollmentResponse, ErrorResponse, NetmapSignature, NetmapSigner, PeerFactsDelta,
    ServerSigningKey, SyncNetmap, SyncRequest, SyncResponse, netmap_lease_transcript,
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};

/// The binding's own netmap. A different `hostname` is different content;
/// `previous` continues its serial sequence.
pub fn compiled(
    binding: &EnrollmentResponse,
    hostname: &str,
    previous: Option<&CompiledNetmap>,
) -> CompiledNetmap {
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
        binding.node_id,
        NodeSettings {
            hostname: hostname.into(),
            remark: String::new(),
            identity: NodeIdentity::Tag {
                tags: vec!["tag:server".into()],
            },
            enabled: true,
            addresses: NodeAddresses {
                ipv4: Some("100.64.0.1".parse().unwrap()),
                ipv6: None,
            },
            listeners: BTreeMap::new(),
            dialers: Input::Omitted,
            exit_enabled: false,
        },
    );
    let keys = TrustedNode {
        control_key: binding.keys.control.clone(),
        dataplane_key: binding.keys.dataplane.clone(),
        capabilities: BTreeMap::new(),
    };
    let previous = previous
        .map(|old| BTreeMap::from([(binding.node_id, PreviousNetmap::from(old))]))
        .unwrap_or_default();
    compile_netmap(NetmapInput {
        network: binding.network_id,
        revision: Revision::MIN,
        configuration: &cfg,
        subjects: &SubjectContext::default(),
        inventory: &BTreeMap::from([(binding.node_id, keys)]),
        rule_sets: &BTreeMap::new(),
        selected_profiles: &BTreeMap::new(),
        selection_versions: &BTreeMap::new(),
        previous: &previous,
    })
    .unwrap()
    .nodes
    .remove(&binding.node_id)
    .unwrap()
}

/// The fixture Server's netmap signing key, pinned through `Fault::ServerKey`.
pub fn server_key() -> (SigningKey, ServerSigningKey) {
    let secret = SigningKey::from_bytes(&[42; 32]);
    let public = ServerSigningKey {
        key_id: "00000000-0000-4009-8000-000000000001".parse().unwrap(),
        key_generation: Version::MIN,
        public_key: NodePublicKey {
            algorithm: KeyAlgorithm::Ed25519,
            bytes: secret.verifying_key().to_bytes().into(),
        },
    };
    (secret, public)
}

/// What the Server signs for `binding`'s Node.
pub fn sign_with(
    secret: &SigningKey,
    key: &ServerSigningKey,
    binding: &EnrollmentResponse,
    lease: &NetmapLease,
) -> NetmapSignature {
    let message = netmap_lease_transcript(
        binding.network_id,
        binding.node_id,
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

pub fn sign(binding: &EnrollmentResponse, lease: &NetmapLease) -> NetmapSignature {
    let (secret, key) = server_key();
    sign_with(&secret, &key, binding, lease)
}

pub fn lease(netmap: NetmapRef, validity_ms: u64) -> NetmapLease {
    let now = u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap();
    NetmapLease::new(netmap, now, now + validity_ms).unwrap()
}

/// A signed response for `binding` with this lease and netmap part.
pub fn signed(
    binding: &EnrollmentResponse,
    lease: NetmapLease,
    netmap: SyncNetmap,
) -> SyncResponse {
    SyncResponse {
        signature: sign(binding, &lease),
        lease,
        netmap,
        peer_facts: None,
    }
}

pub fn full(
    binding: &EnrollmentResponse,
    netmap: &CompiledNetmap,
    validity_ms: u64,
) -> SyncResponse {
    signed(
        binding,
        lease(netmap.reference(), validity_ms),
        SyncNetmap::Full {
            netmap: Box::new(netmap.clone()),
        },
    )
}

pub fn unchanged(
    binding: &EnrollmentResponse,
    netmap: &CompiledNetmap,
    validity_ms: u64,
) -> SyncResponse {
    signed(
        binding,
        lease(netmap.reference(), validity_ms),
        SyncNetmap::Unchanged,
    )
}

type Reply = Result<SyncResponse, ClientError<ErrorResponse>>;

/// Replies in order; records each request and the bearer it carried.
#[derive(Default)]
pub struct Script {
    replies: Mutex<VecDeque<Reply>>,
    pub requests: Mutex<Vec<SyncRequest>>,
}

impl Script {
    pub fn new() -> Arc<Self> {
        Arc::default()
    }
    pub fn push(&self, reply: Reply) {
        self.replies.lock().unwrap().push_back(reply);
    }
    pub fn requests(&self) -> Vec<SyncRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl SyncTransport for Script {
    fn sync<'a>(&'a self, request: &'a SyncRequest, token: &'a str) -> SyncFuture<'a> {
        assert!(!token.is_empty());
        self.requests.lock().unwrap().push(request.clone());
        let reply = self.replies.lock().unwrap().pop_front();
        Box::pin(async move {
            match reply {
                Some(reply) => reply,
                // An empty script behaves like an unreachable Server.
                None => Err(ClientError::Network { status: None }),
            }
        })
    }
}

#[derive(Default)]
pub struct Facts {
    pub applied: Vec<(NetmapRef, PeerFactsDelta)>,
    pub reject: bool,
}

impl PeerFactsSink for Facts {
    fn apply(&mut self, netmap: NetmapRef, facts: PeerFactsDelta) -> Result<(), FactsRejected> {
        if self.reject {
            return Err(FactsRejected);
        }
        self.applied.push((netmap, facts));
        Ok(())
    }
}
