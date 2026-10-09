use super::{
    Error, MAX_PLAINTEXT,
    codec::{self, Context, Frame, Proof},
};
#[cfg(test)]
use crate::{Packet, PacketBuffer};
mod noise;
use crate::authority::View;
use ed25519_dalek::{Signature, VerifyingKey};
use meshway_authority::{DataPlaneIdentity, packet_proof_message};
use meshway_configuration::execution::LogicalPath;
use meshway_model::{BoundNodeKey, Digest, NodeId};
use meshway_policy::admission::{Carrier, path_carrier};
pub(super) use noise::Transport;
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

const NOISE: &str = "Noise_IK_25519_ChaChaPoly_SHA256";
const PROLOGUE: &[u8] = b"meshway/circuit/noise-prologue/candidate-v6\0";
const PROOF: &[u8] = b"meshway/circuit/hop-proof/candidate-v6\0";

pub(super) fn keys<'a>(
    view: &View<'a>,
    node: NodeId,
) -> Result<(&'a BoundNodeKey, &'a BoundNodeKey), Error> {
    view.keys(node).ok_or(Error::Context)
}

pub(super) fn prologue(cfg: &View<'_>, context: &Context) -> Result<Vec<u8>, Error> {
    let mut out = PROLOGUE.to_vec();
    out.extend_from_slice(codec::MAGIC);
    context.encode(&mut out)?;
    encode_keys(cfg, &context.path, &mut out)?;
    Ok(out)
}

fn encode_keys(cfg: &View<'_>, path: &[NodeId], out: &mut Vec<u8>) -> Result<(), Error> {
    for node in path {
        codec::id(out, node.as_uuid());
        let (control, dataplane) = keys(cfg, *node)?;
        for key in [control, dataplane] {
            codec::id(out, key.id.as_uuid());
            out.extend_from_slice(&key.generation.get().to_be_bytes());
            out.extend_from_slice(key.public_key.bytes.as_bytes());
        }
    }
    Ok(())
}

/// Relays cannot inspect the Noise prologue. Its key-set digest is also in the
/// signed header, so every node checks identical current Control/DP bindings.
pub(super) fn key_digest(cfg: &View<'_>, path: &[NodeId]) -> Result<Digest, Error> {
    if !(2..=codec::MAX_PATH).contains(&path.len()) {
        return Err(Error::Context);
    }
    let mut bytes = b"meshway/circuit/key-set/candidate-v6\0".to_vec();
    encode_keys(cfg, path, &mut bytes)?;
    Ok(Digest::from_bytes(Sha256::digest(bytes).into()))
}

pub(super) fn build(
    identity: &DataPlaneIdentity,
    cfg: &View<'_>,
    context: &Context,
    initiator: bool,
) -> Result<snow::HandshakeState, Error> {
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

#[cfg(test)]
pub(super) fn encrypt(
    noise: &Transport,
    nonce: u64,
    packet: Packet,
) -> Result<PacketBuffer, Error> {
    static PROFILE: meshway_logging::profile::Metric =
        meshway_logging::profile::Metric::new("crypto.encrypt");
    let _profile = PROFILE.enter();
    let mut buffer = packet.into_buffer();
    noise.encrypt(nonce, &mut buffer)?;
    Ok(buffer)
}

#[cfg(test)]
pub(super) fn decrypt(
    noise: &Transport,
    nonce: u64,
    mut bytes: PacketBuffer,
) -> Result<Packet, Error> {
    static PROFILE: meshway_logging::profile::Metric =
        meshway_logging::profile::Metric::new("crypto.decrypt_parse");
    let _profile = PROFILE.enter();
    noise.decrypt(nonce, &mut bytes)?;
    Packet::parse(bytes).map_err(|_| Error::Packet)
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
    cfg: &View<'_>,
    frame: &mut Frame,
    carrier: Carrier,
    ephemeral: [u8; 32],
) -> Result<(), Error> {
    if !frame.kind.handshake() {
        return Err(Error::State);
    }
    static PROFILE: meshway_logging::profile::Metric =
        meshway_logging::profile::Metric::new("crypto.append");
    let _profile = PROFILE.enter();
    let key = &cfg.self_keys().control_key;
    let mut hash = transcript(frame)?;
    for proof in &frame.proofs {
        extend(&mut hash, proof)?;
    }
    let mut proof = Proof {
        key: key.id,
        generation: key.generation,
        carrier,
        signature: [0; 64],
        ephemeral,
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

/// Verify the whole chain, including the producer, against the exact path
/// this owner admitted for the circuit. Peer identity is derived solely from
/// current keys, complete path position and the actual carrier. The caller
/// still checks the incoming carrier against its own adjacency admission.
pub(super) fn verify(
    cfg: &View<'_>,
    frame: &Frame,
    incoming: &Carrier,
    selected: &LogicalPath,
) -> Result<NodeId, Error> {
    if !frame.kind.handshake() {
        return Err(Error::Proof);
    }
    let context = &frame.context;
    if context.network != cfg.network() {
        return Err(Error::Context);
    }
    verify_chain(frame, incoming, cfg.node(), selected, |node| {
        keys(cfg, node).map(|keys| keys.0)
    })
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
        if let Some(exchange) = &frame.exchange {
            let position = context
                .path
                .iter()
                .position(|node| *node == signer)
                .ok_or(Error::Proof)?;
            if exchange.public.get(position) != Some(&proof.ephemeral) {
                return Err(Error::Proof);
            }
        }
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
