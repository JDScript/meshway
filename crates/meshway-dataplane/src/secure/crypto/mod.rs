use super::{
    Error, MAX_PLAINTEXT,
    codec::{self, Context, Frame, Proof},
};
use crate::{Packet, PacketFacts};
use ed25519_dalek::{Signature, VerifyingKey};
use meshway_configuration::execution::{LogicalPath, NodeConfiguration};
use meshway_control::{DataPlaneIdentity, packet_proof_message};
use meshway_model::{BoundNodeKey, Digest, KeyAlgorithm, NodeId};
use meshway_policy::admission::{Carrier, admit_circuit_carrier, circuit_permit, path_carrier};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

const NOISE: &str = "Noise_IK_25519_ChaChaPoly_SHA256";
const PROLOGUE: &[u8] = b"meshway/circuit/noise-prologue/candidate-v3\0";
const PROOF: &[u8] = b"meshway/circuit/hop-proof/candidate-v3\0";

pub(super) fn keys(
    cfg: &NodeConfiguration,
    node: NodeId,
) -> Result<(&BoundNodeKey, &BoundNodeKey), Error> {
    let keys = if node == cfg.node_id {
        (&cfg.self_keys.control_key, &cfg.self_keys.dataplane_key)
    } else {
        let peer = cfg.peers.get(&node).ok_or(Error::Context)?;
        (&peer.control_key, &peer.dataplane_key)
    };
    if keys.0.public_key.algorithm != KeyAlgorithm::Ed25519
        || keys.1.public_key.algorithm != KeyAlgorithm::X25519
    {
        return Err(Error::Context);
    }
    Ok(keys)
}

pub(super) fn prologue(cfg: &NodeConfiguration, context: &Context) -> Result<Vec<u8>, Error> {
    let mut out = PROLOGUE.to_vec();
    out.extend_from_slice(codec::MAGIC);
    context.encode(&mut out)?;
    encode_keys(cfg, &context.path, &mut out)?;
    Ok(out)
}

fn encode_keys(cfg: &NodeConfiguration, path: &[NodeId], out: &mut Vec<u8>) -> Result<(), Error> {
    for node in path {
        codec::id(out, node);
        let (control, dataplane) = keys(cfg, *node)?;
        for key in [control, dataplane] {
            codec::id(out, key.id);
            out.extend_from_slice(&key.generation.get().to_be_bytes());
            out.extend_from_slice(key.public_key.bytes.as_bytes());
        }
    }
    Ok(())
}

/// Relays cannot inspect the Noise prologue. Its key-set digest is also in the
/// signed header, so every node checks identical current Control/DP bindings.
pub(super) fn key_digest(cfg: &NodeConfiguration, path: &[NodeId]) -> Result<Digest, Error> {
    if !(2..=codec::MAX_PATH).contains(&path.len()) {
        return Err(Error::Context);
    }
    let mut bytes = b"meshway/circuit/key-set/candidate-v3\0".to_vec();
    encode_keys(cfg, path, &mut bytes)?;
    Ok(Digest::from_bytes(Sha256::digest(bytes).into()))
}

pub(super) fn build(
    identity: &DataPlaneIdentity,
    context: &Context,
    initiator: bool,
) -> Result<snow::HandshakeState, Error> {
    let cfg = identity
        .snapshot()
        .configuration()
        .map_err(|_| Error::Authority)?;
    let prologue = prologue(cfg, context)?;
    let remote = if initiator {
        *context.path.last().ok_or(Error::Context)?
    } else {
        context.path[0]
    };
    let public = keys(cfg, remote)?.1.public_key.bytes;
    identity
        .with_noise_key(|private| build_with_key(private, public.as_bytes(), &prologue, initiator))
        .map_err(|_| Error::Authority)?
}

fn build_with_key(
    private: &[u8; 32],
    remote: &[u8; 32],
    prologue: &[u8],
    initiator: bool,
) -> Result<snow::HandshakeState, Error> {
    let builder = snow::Builder::new(NOISE.parse().map_err(|_| Error::Crypto)?)
        .local_private_key(private)
        .map_err(|_| Error::Crypto)?
        .prologue(prologue)
        .map_err(|_| Error::Crypto)?;
    // Snow generates a fresh ephemeral using its maintained default resolver.
    if initiator {
        builder
            .remote_public_key(remote)
            .map_err(|_| Error::Crypto)?
            .build_initiator()
    } else {
        builder.build_responder()
    }
    .map_err(|_| Error::Crypto)
}

pub(super) fn write_handshake(
    noise: &mut snow::HandshakeState,
    length: usize,
) -> Result<Vec<u8>, Error> {
    let mut out = vec![0; length];
    if noise
        .write_message(&[], &mut out)
        .map_err(|_| Error::Crypto)?
        != length
    {
        return Err(Error::Crypto);
    }
    Ok(out)
}

pub(super) fn read_handshake(
    noise: &mut snow::HandshakeState,
    bytes: &[u8],
    length: usize,
    expected: &[u8; 32],
) -> Result<(), Error> {
    if bytes.len() != length {
        return Err(Error::Wire);
    }
    let mut scratch = Zeroizing::new([0u8; 96]);
    if noise
        .read_message(bytes, &mut scratch[..])
        .map_err(|_| Error::Crypto)?
        != 0
        || noise.get_remote_static() != Some(expected.as_slice())
    {
        return Err(Error::Crypto);
    }
    Ok(())
}

pub(super) fn encrypt(
    noise: &snow::StatelessTransportState,
    nonce: u64,
    packet: Packet,
) -> Result<Vec<u8>, Error> {
    static PROFILE: meshway_logging::profile::Metric =
        meshway_logging::profile::Metric::new("crypto.encrypt");
    let _profile = PROFILE.enter();
    let plaintext = Zeroizing::new(packet.into_bytes());
    if plaintext.len() > MAX_PLAINTEXT {
        return Err(Error::Unsupported);
    }
    let mut out = vec![0; plaintext.len() + 16];
    if noise
        .write_message(nonce, &plaintext, &mut out)
        .map_err(|_| Error::Crypto)?
        != out.len()
    {
        return Err(Error::Crypto);
    }
    Ok(out)
}

pub(super) fn decrypt(
    noise: &snow::StatelessTransportState,
    nonce: u64,
    bytes: &[u8],
    facts: PacketFacts,
) -> Result<Packet, Error> {
    static PROFILE: meshway_logging::profile::Metric =
        meshway_logging::profile::Metric::new("crypto.decrypt_parse");
    let _profile = PROFILE.enter();
    let expected = bytes
        .len()
        .checked_sub(16)
        .filter(|length| *length <= MAX_PLAINTEXT)
        .ok_or(Error::Wire)?;
    let mut plaintext = Zeroizing::new(vec![0; expected]);
    let length = noise
        .read_message(nonce, bytes, &mut plaintext)
        .map_err(|_| Error::Crypto)?;
    if length != expected {
        return Err(Error::Crypto);
    }
    let packet = Packet::parse(plaintext[..length].to_vec()).map_err(|_| Error::Packet)?;
    exact_facts(packet, facts)
}

fn exact_facts(packet: Packet, facts: PacketFacts) -> Result<Packet, Error> {
    if packet.facts() != facts {
        let _plaintext = Zeroizing::new(packet.into_bytes());
        return Err(Error::Packet);
    }
    Ok(packet)
}

fn transcript(frame: &Frame) -> Result<Sha256, Error> {
    static PROFILE: meshway_logging::profile::Metric =
        meshway_logging::profile::Metric::new("crypto.transcript");
    let _profile = PROFILE.enter();
    let mut hash = Sha256::new();
    hash.update(PROOF);
    hash.update(frame.base()?);
    Ok(hash)
}

fn proof_digest(hash: &Sha256, position: usize, proof: &Proof) -> Result<Digest, Error> {
    let mut hash = hash.clone();
    hash.update([position as u8]);
    let mut prefix = Vec::new();
    proof.prefix(&mut prefix)?;
    hash.update(prefix);
    Ok(Digest::from_bytes(hash.finalize().into()))
}

fn extend(hash: &mut Sha256, proof: &Proof) -> Result<(), Error> {
    let mut bytes = Vec::new();
    proof.encode(&mut bytes)?;
    hash.update(bytes);
    Ok(())
}

pub(super) fn append(
    identity: &DataPlaneIdentity,
    frame: &mut Frame,
    carrier: Carrier,
) -> Result<(), Error> {
    static PROFILE: meshway_logging::profile::Metric =
        meshway_logging::profile::Metric::new("crypto.append");
    let _profile = PROFILE.enter();
    let cfg = identity
        .snapshot()
        .configuration()
        .map_err(|_| Error::Authority)?;
    let key = &cfg.self_keys.control_key;
    let mut hash = transcript(frame)?;
    for proof in &frame.proofs {
        extend(&mut hash, proof)?;
    }
    let mut proof = Proof {
        key: key.id,
        generation: key.generation,
        carrier,
        signature: [0; 64],
    };
    let digest = proof_digest(&hash, frame.proofs.len(), &proof)?;
    proof.signature = {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("crypto.sign");
        let _profile = PROFILE.enter();
        identity.sign_packet(digest).map_err(|_| Error::Authority)?
    };
    frame.proofs.push(proof);
    Ok(())
}

fn ordered_node(context: &Context, reply: bool, position: usize) -> NodeId {
    context.path[if reply {
        context.path.len() - 1 - position
    } else {
        position
    }]
}

/// Verify the whole chain, including the producer. Peer identity is derived
/// solely from current keys, complete path position and the actual carrier.
pub(super) fn verify(
    cfg: &NodeConfiguration,
    frame: &Frame,
    incoming: &Carrier,
) -> Result<NodeId, Error> {
    let context = &frame.context;
    if context.network != cfg.network_id {
        return Err(Error::Context);
    }
    let permit = circuit_permit(cfg, context.authorization).map_err(|_| Error::Context)?;
    let selected = permit
        .paths
        .iter()
        .find(|path| path.nodes == context.path)
        .ok_or(Error::Context)?;
    let previous = verify_chain(frame, incoming, cfg.node_id, selected, |node| {
        keys(cfg, node).map(|keys| keys.0)
    })?;
    admit_circuit_carrier(
        cfg,
        context.authorization,
        &context.path,
        previous,
        incoming,
    )
    .map_err(|_| Error::Policy)?;
    Ok(previous)
}

fn verify_chain<'a>(
    frame: &Frame,
    incoming: &Carrier,
    local: NodeId,
    selected: &LogicalPath,
    key_for: impl Fn(NodeId) -> Result<&'a BoundNodeKey, Error>,
) -> Result<NodeId, Error> {
    static PROFILE: meshway_logging::profile::Metric =
        meshway_logging::profile::Metric::new("crypto.verify_chain");
    let _profile = PROFILE.enter();
    let context = &frame.context;
    if selected.nodes != context.path {
        return Err(Error::Context);
    }
    let position = context
        .path
        .iter()
        .position(|node| *node == local)
        .ok_or(Error::Context)?;
    let count = if frame.reply {
        context.path.len() - 1 - position
    } else {
        position
    };
    if count == 0 || frame.proofs.len() != count {
        return Err(Error::Proof);
    }
    let mut hash = transcript(frame)?;
    for (index, proof) in frame.proofs.iter().enumerate() {
        let signer = ordered_node(context, frame.reply, index);
        let next = ordered_node(context, frame.reply, index + 1);
        let carrier = &proof.carrier;
        if !((carrier.dialer == signer && carrier.listener == next)
            || (carrier.listener == signer && carrier.dialer == next))
            || !path_carrier(selected, carrier)
        {
            return Err(Error::Proof);
        }
        let key = key_for(signer)?;
        if proof.key != key.id || proof.generation != key.generation {
            return Err(Error::Proof);
        }
        let digest = proof_digest(&hash, index, proof)?;
        let message = packet_proof_message(context.network, signer, key, digest);
        {
            static PROFILE: meshway_logging::profile::Metric =
                meshway_logging::profile::Metric::new("crypto.verify_signature");
            let _profile = PROFILE.enter();
            VerifyingKey::from_bytes(key.public_key.bytes.as_bytes())
                .map_err(|_| Error::Proof)?
                .verify_strict(&message, &Signature::from_bytes(&proof.signature))
                .map_err(|_| Error::Proof)?;
        }
        extend(&mut hash, proof)?;
    }
    let previous = ordered_node(context, frame.reply, count - 1);
    if &frame.proofs.last().ok_or(Error::Proof)?.carrier != incoming {
        return Err(Error::Proof);
    }
    Ok(previous)
}

#[cfg(test)]
mod tests;
