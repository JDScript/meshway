//! Signed challenge bootstrap only. This is not an application encryption layer.
use crate::plan::Plan;
use crate::{Carrier, Error, scope::Scope};
use ed25519_dalek::{Signature, VerifyingKey};
use meshway_engine::{DataPlaneKeys, packet_proof_message};
use meshway_model::{BoundNodeKey, Digest, NetworkId, NodeId};
use meshway_transport::raw::Stream;
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(crate) const MAGIC: &[u8; 8] = b"MWADJv1\0";
pub(crate) const MAX_MESSAGE: usize = 2048;
const INIT: &[u8] = b"meshway/adjacency/linkInit/v1\0";
const CHALLENGE: &[u8] = b"meshway/adjacency/linkChallenge/v1\0";
const ACK: &[u8] = b"meshway/adjacency/linkAck/v1\0";

mod exchange;
pub(crate) use exchange::incoming_bound;
pub(crate) use exchange::{Initiation, Kind, MAX_PROBE, Transcript, decode, encode, incoming};

fn keys(cfg: &Plan, node: NodeId) -> Result<(&BoundNodeKey, &BoundNodeKey), Error> {
    cfg.keys(node)
}

fn context(cfg: &Plan, carrier: &Carrier) -> Result<Vec<u8>, Error> {
    encode_context(
        cfg.network,
        carrier,
        [keys(cfg, carrier.dialer)?, keys(cfg, carrier.listener)?],
    )
}

fn encode_context(
    network: NetworkId,
    carrier: &Carrier,
    keys: [(&BoundNodeKey, &BoundNodeKey); 2],
) -> Result<Vec<u8>, Error> {
    let key = carrier.listener_key.as_bytes();
    if key.is_empty() || key.len() > 128 {
        return Err(Error::Configuration);
    }
    let mut out = Vec::with_capacity(210);
    out.extend_from_slice(network.as_uuid().as_bytes());
    out.extend_from_slice(carrier.dialer.as_uuid().as_bytes());
    out.extend_from_slice(carrier.listener.as_uuid().as_bytes());
    out.push(match carrier.protocol {
        meshway_model::TransportProtocol::NativeTcp => 1,
        meshway_model::TransportProtocol::Vless => 2,
        meshway_model::TransportProtocol::Trojan => 3,
        meshway_model::TransportProtocol::NativeUdp => 4,
    });
    out.push(u8::try_from(key.len()).map_err(|_| Error::Configuration)?);
    out.extend_from_slice(key);
    let mut bindings = Sha256::new();
    bindings.update(b"meshway/adjacency/key-bindings/v1\0");
    for (control, dataplane) in keys {
        for key in [control, dataplane] {
            bindings.update(key.id.as_uuid().as_bytes());
            bindings.update(key.generation.get().to_be_bytes());
            bindings.update(key.public_key.bytes.as_bytes());
        }
    }
    out.extend_from_slice(&bindings.finalize());
    Ok(out)
}

fn digest(domain: &[u8], context: &[u8], client: &[u8; 32], server: &[u8; 32]) -> Digest {
    let mut hash = Sha256::new();
    hash.update(domain);
    hash.update(context);
    hash.update(client);
    hash.update(server);
    Digest::from_bytes(hash.finalize().into())
}

fn verify(cfg: &Plan, peer: NodeId, digest: Digest, signature: &[u8]) -> Result<(), Error> {
    let key = keys(cfg, peer)?.0;
    verify_key(cfg.network, peer, key, digest, signature)
}

fn verify_key(
    network: NetworkId,
    peer: NodeId,
    key: &BoundNodeKey,
    digest: Digest,
    signature: &[u8],
) -> Result<(), Error> {
    let public = VerifyingKey::from_bytes(key.public_key.bytes.as_bytes())
        .map_err(|_| Error::Authentication)?;
    let signature = Signature::from_slice(signature).map_err(|_| Error::Authentication)?;
    public
        .verify_strict(
            &packet_proof_message(network, peer, key, digest),
            &signature,
        )
        .map_err(|_| Error::Authentication)
}

/// The dialer an initiation names (context bytes 16..32). Only a lookup key:
/// the full context and signature are still verified for that one carrier.
pub(crate) fn claimed_dialer(intro: &[u8]) -> Option<NodeId> {
    let hex: String = intro
        .get(16..32)?
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            let dash = if matches!(index, 4 | 6 | 8 | 10) {
                "-"
            } else {
                ""
            };
            format!("{dash}{byte:02x}")
        })
        .collect();
    hex.parse().ok()
}

/// Random identity of one circuit owner lifetime (a Manager and the
/// Managers it restarts into). Every adjacency nonce this owner signs starts
/// with it, so a peer learns from an authenticated transcript whether a new
/// channel comes from the same process state or from a restarted peer
/// without any change to the message layout.
pub type Boot = [u8; 16];

pub(crate) fn boot() -> Result<Boot, Error> {
    let mut boot = [0; 16];
    getrandom::fill(&mut boot).map_err(|_| Error::Authentication)?;
    Ok(boot)
}

/// A fresh nonce carrying this owner's signed boot prefix; 128 random bits
/// keep every transcript and health challenge unique.
pub(crate) fn boot_nonce(boot: &Boot) -> Result<[u8; 32], Error> {
    let mut nonce = nonce()?;
    nonce[..16].copy_from_slice(boot);
    Ok(nonce)
}

pub(crate) fn nonce() -> Result<[u8; 32], Error> {
    let mut nonce = [0; 32];
    getrandom::fill(&mut nonce).map_err(|_| Error::Authentication)?;
    Ok(nonce)
}

async fn write(stream: &mut Box<dyn Stream>, kind: u8, body: &[u8]) -> Result<(), Error> {
    let message = encode(Kind::from_byte(kind)?, body)?;
    stream.write_all(&message).await?;
    stream.flush().await?;
    Ok(())
}

async fn read(stream: &mut Box<dyn Stream>, kind: u8) -> Result<Vec<u8>, Error> {
    let mut header = [0; 11];
    stream.read_exact(&mut header).await?;
    if &header[..8] != MAGIC || header[8] != kind {
        return Err(Error::Authentication);
    }
    let length = usize::from(u16::from_be_bytes([header[9], header[10]]));
    if length == 0 || length > MAX_MESSAGE - 11 {
        return Err(Error::Authentication);
    }
    let mut body = vec![0; length];
    stream.read_exact(&mut body).await?;
    Ok(body)
}

fn response(context: &[u8], client: &[u8; 32], server: &[u8; 32], signature: [u8; 64]) -> Vec<u8> {
    let mut body = Vec::with_capacity(160);
    body.extend_from_slice(&Sha256::digest(context));
    body.extend_from_slice(client);
    body.extend_from_slice(server);
    body.extend_from_slice(&signature);
    body
}

fn parse_response(body: &[u8], context: &[u8], client: &[u8; 32]) -> Result<[u8; 32], Error> {
    if body.len() != 160 || body[..32] != Sha256::digest(context)[..] || body[32..64] != client[..]
    {
        return Err(Error::Authentication);
    }
    body[64..96].try_into().map_err(|_| Error::Authentication)
}

pub(crate) async fn dial(
    keys: &DataPlaneKeys,
    cfg: &Plan,
    scope: &Scope,
    mut stream: Box<dyn Stream>,
    carrier: &Carrier,
    boot: &Boot,
) -> Result<(Box<dyn Stream>, [u8; 32], Boot), Error> {
    scope.check()?;
    let (initiation, intro) = Initiation::new(keys, cfg, carrier, boot)?;
    scope.run(write(&mut stream, 1, &intro)).await?;
    let challenge = scope.run(read(&mut stream, 2)).await?;
    let transcript = initiation.challenge(cfg, carrier.listener, &challenge)?;
    scope
        .run(write(
            &mut stream,
            3,
            &transcript.message(keys, cfg, Kind::Ack)?,
        ))
        .await?;
    Ok((stream, transcript.id(), transcript.peer_boot(true)))
}

pub(crate) async fn accept(
    keys: &DataPlaneKeys,
    cfg: &Plan,
    scope: &Scope,
    mut stream: Box<dyn Stream>,
    carriers: impl Fn(NodeId) -> Option<Carrier>,
    boot: &Boot,
) -> Result<(Box<dyn Stream>, NodeId, Carrier, [u8; 32], Boot), Error> {
    let carriers = |intro: &[u8]| -> Result<Vec<(NodeId, Carrier)>, Error> {
        let peer = claimed_dialer(intro).ok_or(Error::Authentication)?;
        Ok(carriers(peer)
            .map(|carrier| (peer, carrier))
            .into_iter()
            .collect())
    };
    let intro = scope.run(read(&mut stream, 1)).await?;
    let (peer, carrier, initiation) = incoming(cfg, &carriers(&intro)?, &intro)?;
    let mut scope = scope.clone();
    scope.edge = Some((peer, carrier.clone()));
    scope.check()?;
    let transcript = initiation.respond(boot)?;
    scope
        .run(write(
            &mut stream,
            2,
            &transcript.message(keys, cfg, Kind::Challenge)?,
        ))
        .await?;
    let ack = scope.run(read(&mut stream, 3)).await?;
    transcript.verify(cfg, peer, Kind::Ack, &ack)?;
    scope.check()?;
    Ok((
        stream,
        peer,
        carrier,
        transcript.id(),
        transcript.peer_boot(false),
    ))
}

#[cfg(test)]
mod tests;
