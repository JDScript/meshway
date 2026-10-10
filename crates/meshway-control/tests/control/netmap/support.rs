//! Real one-Node netmaps for an enrolled identity and a scripted sync transport.
use ed25519_dalek::SigningKey;
use meshway_configuration::netmap::CompiledNetmap;
use meshway_model::NetmapLease;
use meshway_protocol::{
    EnrollmentResponse, NetmapSignature, ServerSigningKey, SyncNetmap, SyncResponse,
};
use meshway_test_server::configuration::{
    listener, native_udp, network, node, tag_node, trusted, trusted_keys,
};
use std::collections::BTreeMap;

pub use meshway_test_server::{Facts, Script, lease, server_key};

/// The binding's own netmap. A different `hostname` is different content;
/// `previous` continues its serial sequence.
pub fn compiled(
    binding: &EnrollmentResponse,
    hostname: &str,
    previous: Option<&CompiledNetmap>,
) -> CompiledNetmap {
    let mut cfg = network("test.internal");
    cfg.nodes.insert(
        binding.node_id,
        tag_node(hostname, 1, BTreeMap::new(), false),
    );
    meshway_test_server::configuration::compile_one(
        binding.network_id,
        &cfg,
        &BTreeMap::from([(
            binding.node_id,
            trusted_keys(&binding.keys, BTreeMap::new()),
        )]),
        binding.node_id,
        previous,
    )
}

/// The binding's netmap with one peer whose Native UDP listener has no
/// configured endpoint: only peer facts can make it dialable.
pub fn compiled_with_peer(
    binding: &EnrollmentResponse,
    hostname: &str,
) -> (CompiledNetmap, meshway_model::NodeId) {
    let peer = node(9);
    let mut cfg = network("test.internal");
    cfg.nodes.insert(
        binding.node_id,
        tag_node(hostname, 1, BTreeMap::new(), false),
    );
    cfg.nodes.insert(
        peer,
        tag_node(
            "peer",
            9,
            BTreeMap::from([(
                "udp".to_owned(),
                listener(serde_json::json!({
                    "type": "native_udp", "listen": "0.0.0.0", "listenPort": 41641,
                    "endpoints": [],
                })),
            )]),
            false,
        ),
    );
    let netmap = meshway_test_server::configuration::compile_one(
        binding.network_id,
        &cfg,
        &BTreeMap::from([
            (
                binding.node_id,
                trusted_keys(&binding.keys, BTreeMap::from([native_udp()])),
            ),
            (peer, trusted(9, false)),
        ]),
        binding.node_id,
        None,
    );
    (netmap, peer)
}

/// What the Server signs for `binding`'s Node.
pub fn sign_with(
    secret: &SigningKey,
    key: &ServerSigningKey,
    binding: &EnrollmentResponse,
    lease: &NetmapLease,
) -> NetmapSignature {
    meshway_test_server::sign_with(secret, key, binding.network_id, binding.node_id, lease)
}

pub fn sign(binding: &EnrollmentResponse, lease: &NetmapLease) -> NetmapSignature {
    meshway_test_server::sign(binding.network_id, binding.node_id, lease)
}

/// A signed response for `binding` with this lease and netmap part.
pub fn signed(
    binding: &EnrollmentResponse,
    lease: NetmapLease,
    netmap: SyncNetmap,
) -> SyncResponse {
    meshway_test_server::signed(binding.network_id, binding.node_id, lease, netmap)
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
