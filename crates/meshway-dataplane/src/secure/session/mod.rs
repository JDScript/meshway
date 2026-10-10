//! Circuit-owned ephemeral agreement and directional adjacent hop MACs.
//! Public contributions are authenticated by the existing ordered handshake
//! signatures. Each verifier authenticates its immediate neighbour; end-to-end
//! Noise authenticates the original producer at the endpoint.
use super::{
    Error,
    codec::{self, Context, Exchange, Frame},
};
use hkdf::Hkdf;
use hmac::{Hmac, KeyInit, Mac};
use meshway_configuration::execution::LogicalPath;
use meshway_model::NodeId;
use meshway_policy::admission::{Carrier, path_carrier};
use sha2::{Digest as _, Sha256};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

const AGREEMENT: &[u8] = b"meshway/circuit/session-agreement/v5\0";
const KEY: &[u8] = b"meshway/circuit/directional-hop-key/v5\0";
const MAC: &[u8] = b"meshway/circuit/hop-mac/v5";
// HKDF and handshake hashing retain sha2 0.10, shared with Snow. Only the
// per-packet MAC uses 0.11's runtime hardware SHA2 detection. Its zeroize
// feature clears the retained inner/outer hash states when dropped.
type HopMac = Hmac<sha2_fast::Sha256>;

struct Pending {
    secret: StaticSecret,
    init: Option<[u8; 32]>,
    prefix: Vec<[u8; 32]>,
}

#[derive(Default)]
pub(super) struct Session {
    public: [u8; 32],
    pending: Option<Pending>,
    keys: Option<Arc<Keys>>,
}

pub(super) struct Keys {
    position: usize,
    // Slot 0 is the preceding neighbour, slot 1 the following neighbour.
    outgoing: [Option<HopMac>; 2],
    incoming: [Option<HopMac>; 2],
}

impl Session {
    pub fn new() -> Result<Self, Error> {
        let mut seed = Zeroizing::new([0; 32]);
        getrandom::fill(&mut seed[..]).map_err(|_| Error::Crypto)?;
        let secret = StaticSecret::from(*seed);
        let public = PublicKey::from(&secret).to_bytes();
        Ok(Self {
            public,
            pending: Some(Pending {
                secret,
                init: None,
                prefix: Vec::new(),
            }),
            keys: None,
        })
    }

    pub fn public(&self) -> [u8; 32] {
        self.public
    }

    pub fn remember_init(&mut self, frame: &Frame) -> Result<(), Error> {
        let pending = self.pending.as_mut().ok_or(Error::State)?;
        if pending.init.is_some() || frame.kind != codec::Kind::Init {
            return Err(Error::State);
        }
        pending.init = Some(Sha256::digest(&frame.body).into());
        pending.prefix = frame.proofs.iter().map(|proof| proof.ephemeral).collect();
        pending.prefix.push(self.public);
        Ok(())
    }

    pub fn exchange(&self) -> Result<Exchange, Error> {
        let pending = self.pending.as_ref().ok_or(Error::State)?;
        Ok(Exchange {
            init: pending.init.ok_or(Error::State)?,
            public: pending.prefix.clone(),
        })
    }

    pub fn derive(&self, frame: &Frame, local: NodeId) -> Result<Keys, Error> {
        let pending = self.pending.as_ref().ok_or(Error::State)?;
        let exchange = frame.exchange.as_ref().ok_or(Error::Proof)?;
        let position = frame
            .context
            .path
            .iter()
            .position(|node| *node == local)
            .ok_or(Error::Context)?;
        if frame.kind != codec::Kind::Response
            || exchange.public.len() != frame.context.path.len()
            || exchange.public.get(position) != Some(&self.public)
            || pending.prefix.len() != position + 1
            || !exchange.public.starts_with(&pending.prefix)
            || Some(exchange.init) != pending.init
        {
            return Err(Error::Proof);
        }
        let mut transcript = AGREEMENT.to_vec();
        frame.context.encode(&mut transcript)?;
        transcript.extend_from_slice(&exchange.init);
        transcript.extend_from_slice(&frame.body);
        for public in &exchange.public {
            transcript.extend_from_slice(public);
        }
        let salt = Sha256::digest(transcript);
        let mut keys = Keys {
            position,
            outgoing: [None, None],
            incoming: [None, None],
        };
        for (peer, public) in exchange.public.iter().enumerate() {
            if peer.abs_diff(position) != 1 {
                continue;
            }
            let shared = pending.secret.diffie_hellman(&PublicKey::from(*public));
            if !shared.was_contributory() {
                return Err(Error::Crypto);
            }
            let hkdf = Hkdf::<Sha256>::new(Some(&salt), shared.as_bytes());
            let slot = usize::from(peer > position);
            for (signer, verifier, output) in [
                (position, peer, &mut keys.outgoing[slot]),
                (peer, position, &mut keys.incoming[slot]),
            ] {
                let mut info = KEY.to_vec();
                info.extend_from_slice(&[signer as u8, verifier as u8]);
                let mut raw = Zeroizing::new([0; 32]);
                hkdf.expand(&info, &mut raw[..])
                    .map_err(|_| Error::Crypto)?;
                *output =
                    Some(HopMac::new_from_slice(&raw[..]).expect("HMAC accepts a 32-byte key"));
            }
        }
        Ok(keys)
    }

    pub fn establish(&mut self, keys: Keys) {
        self.keys = Some(Arc::new(keys));
        self.pending = None; // StaticSecret zeroizes the retained ephemeral.
    }

    /// Authenticate the adjacent hop on the owner's admitted path. The caller
    /// checks the incoming carrier against its current adjacency admission.
    pub fn verify(
        &self,
        selected: &LogicalPath,
        frame: &Frame,
        incoming: &Carrier,
    ) -> Result<NodeId, Error> {
        let (previous, key) = self.prepare_verify(selected, frame, incoming)?;
        key.verify(frame)?;
        Ok(previous)
    }

    pub fn prepare_verify(
        &self,
        selected: &LogicalPath,
        frame: &Frame,
        incoming: &Carrier,
    ) -> Result<(NodeId, MacKey), Error> {
        self.prepare_chain(frame, incoming, selected)
    }

    fn prepare_chain(
        &self,
        frame: &Frame,
        incoming: &Carrier,
        selected: &LogicalPath,
    ) -> Result<(NodeId, MacKey), Error> {
        let keys = self.keys.as_ref().ok_or(Error::State)?;
        if selected.nodes != frame.context.path {
            return Err(Error::Context);
        }
        let count = ordered(frame.context.path.len(), frame.reply, keys.position);
        if frame.kind.handshake() || count == 0 || frame.auth.tag.is_none() {
            return Err(Error::Proof);
        }
        let signer = ordered(frame.context.path.len(), frame.reply, count - 1);
        if !adjacent(&frame.context, signer, keys.position, incoming)
            || !path_carrier(selected, incoming)
        {
            return Err(Error::Proof);
        }
        let previous =
            frame.context.path[ordered(frame.context.path.len(), frame.reply, count - 1)];
        Ok((
            previous,
            MacKey {
                keys: keys.clone(),
                slot: usize::from(signer > keys.position),
                incoming: true,
            },
        ))
    }

    #[cfg(test)]
    fn verify_chain(
        &self,
        frame: &Frame,
        incoming: &Carrier,
        selected: &LogicalPath,
    ) -> Result<NodeId, Error> {
        let (previous, key) = self.prepare_chain(frame, incoming, selected)?;
        key.verify(frame)?;
        Ok(previous)
    }

    pub fn append(&self, frame: &mut Frame, carrier: Carrier) -> Result<(), Error> {
        self.prepare_append(frame, &carrier)?.append(frame)
    }

    pub fn prepare_append(&self, frame: &Frame, carrier: &Carrier) -> Result<MacKey, Error> {
        let keys = self.keys.as_ref().ok_or(Error::State)?;
        let count = ordered(frame.context.path.len(), frame.reply, keys.position);
        let remaining = frame.context.path.len() - count - 1;
        if frame.kind.handshake() || remaining == 0 {
            return Err(Error::State);
        }
        let verifier = ordered(frame.context.path.len(), frame.reply, count + 1);
        if !adjacent(&frame.context, keys.position, verifier, carrier) {
            return Err(Error::Proof);
        }
        Ok(MacKey {
            keys: keys.clone(),
            slot: usize::from(verifier > keys.position),
            incoming: false,
        })
    }

    pub fn owns(&self, key: &MacKey) -> bool {
        self.keys
            .as_ref()
            .is_some_and(|keys| Arc::ptr_eq(keys, &key.keys))
    }
}

/// Opaque immutable key selection. No state or authority is available to crypto workers.
pub(super) struct MacKey {
    keys: Arc<Keys>,
    slot: usize,
    incoming: bool,
}

impl MacKey {
    fn tag(&self, frame: &Frame) -> Result<[u8; 16], Error> {
        let keys = if self.incoming {
            &self.keys.incoming
        } else {
            &self.keys.outgoing
        };
        authenticate(keys[self.slot].as_ref().ok_or(Error::State)?, frame)
    }

    pub fn verify(&self, frame: &Frame) -> Result<(), Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("crypto.verify_hop_mac");
        let _profile = PROFILE.enter();
        if bool::from(self.tag(frame)?.ct_eq(&frame.auth.tag.ok_or(Error::Proof)?)) {
            Ok(())
        } else {
            Err(Error::Proof)
        }
    }

    pub fn append(&self, frame: &mut Frame) -> Result<(), Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("crypto.append_mac");
        let _profile = PROFILE.enter();
        frame.auth.tag = Some(self.tag(frame)?);
        Ok(())
    }
}

fn ordered(length: usize, reply: bool, position: usize) -> usize {
    if reply {
        length - position - 1
    } else {
        position
    }
}

fn adjacent(context: &Context, signer: usize, next: usize, carrier: &Carrier) -> bool {
    let a = context.path[signer];
    let b = context.path[next];
    (carrier.dialer == a && carrier.listener == b) || (carrier.dialer == b && carrier.listener == a)
}

fn authenticate(key: &HopMac, frame: &Frame) -> Result<[u8; 16], Error> {
    frame.validate()?;
    let mut mac = key.clone();
    mac.update(MAC);
    mac.update(&frame.context.circuit);
    mac.update(&[u8::from(frame.reply)]);
    mac.update(&frame.sequence.to_be_bytes());
    if let Some(probe) = frame.probe {
        mac.update(b"meshway/circuit/exchange/v6");
        mac.update(&[match frame.kind {
            codec::Kind::ProbeRequest => 4,
            codec::Kind::ProbeReply => 5,
            codec::Kind::DnsRequest => 6,
            codec::Kind::DnsReply => 7,
            _ => return Err(Error::Wire),
        }]);
        mac.update(probe.measurement_id.as_uuid().as_bytes());
        mac.update(&probe.expires_at_ms.to_be_bytes());
    }
    mac.update(&(frame.body.len() as u32).to_be_bytes());
    mac.update(&frame.body);
    let result = mac.finalize().into_bytes();
    let mut tag = [0; 16];
    tag.copy_from_slice(&result[..16]);
    Ok(tag)
}

#[cfg(test)]
mod tests;
