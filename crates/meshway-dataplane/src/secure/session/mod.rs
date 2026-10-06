//! Circuit-owned ephemeral agreement and directional, verifier-specific hop MACs.
//! Public contributions are authenticated by the existing ordered handshake
//! signatures. A verifier recomputes the chain with every preceding node; an
//! intermediate node possesses only its own pair keys, never another row.
use super::{
    Error,
    codec::{self, Context, Exchange, Frame},
};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use meshway_configuration::execution::{LogicalPath, NodeConfiguration};
use meshway_model::NodeId;
use meshway_policy::admission::{Carrier, admit_circuit_carrier, circuit_permit, path_carrier};
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

const AGREEMENT: &[u8] = b"meshway/circuit/session-agreement/candidate-v4\0";
const KEY: &[u8] = b"meshway/circuit/directional-hop-key/candidate-v4\0";
const CHAIN: &[u8] = b"meshway/circuit/mac-chain/candidate-v4\0";

struct Pending {
    secret: StaticSecret,
    init: Option<[u8; 32]>,
    prefix: Vec<[u8; 32]>,
}

#[derive(Default)]
pub(super) struct Session {
    public: [u8; 32],
    pending: Option<Pending>,
    keys: Option<Keys>,
}

pub(super) struct Keys {
    position: usize,
    outgoing: Zeroizing<[[u8; 32]; codec::MAX_PATH]>,
    incoming: Zeroizing<[[u8; 32]; codec::MAX_PATH]>,
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
            outgoing: Zeroizing::new([[0; 32]; codec::MAX_PATH]),
            incoming: Zeroizing::new([[0; 32]; codec::MAX_PATH]),
        };
        for (peer, public) in exchange.public.iter().enumerate() {
            if peer == position {
                continue;
            }
            let shared = pending.secret.diffie_hellman(&PublicKey::from(*public));
            if !shared.was_contributory() {
                return Err(Error::Crypto);
            }
            let hkdf = Hkdf::<Sha256>::new(Some(&salt), shared.as_bytes());
            for (signer, verifier, output) in [
                (position, peer, &mut keys.outgoing[peer]),
                (peer, position, &mut keys.incoming[peer]),
            ] {
                let mut info = KEY.to_vec();
                info.extend_from_slice(&[signer as u8, verifier as u8]);
                hkdf.expand(&info, output).map_err(|_| Error::Crypto)?;
            }
        }
        Ok(keys)
    }

    pub fn establish(&mut self, keys: Keys) {
        self.keys = Some(keys);
        self.pending = None; // StaticSecret zeroizes the retained ephemeral.
    }

    pub fn verify(
        &self,
        cfg: &NodeConfiguration,
        frame: &Frame,
        incoming: &Carrier,
    ) -> Result<NodeId, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("crypto.verify_mac_chain");
        let _profile = PROFILE.enter();
        let selected = circuit_permit(cfg, frame.context.authorization)
            .map_err(|_| Error::Context)?
            .paths
            .iter()
            .find(|path| path.nodes == frame.context.path)
            .ok_or(Error::Context)?;
        let previous = self.verify_chain(frame, incoming, selected)?;
        admit_circuit_carrier(
            cfg,
            frame.context.authorization,
            &frame.context.path,
            previous,
            incoming,
        )
        .map_err(|_| Error::Policy)?;
        Ok(previous)
    }

    fn verify_chain(
        &self,
        frame: &Frame,
        incoming: &Carrier,
        selected: &LogicalPath,
    ) -> Result<NodeId, Error> {
        let keys = self.keys.as_ref().ok_or(Error::State)?;
        if selected.nodes != frame.context.path {
            return Err(Error::Context);
        }
        let count = ordered(frame.context.path.len(), frame.reply, keys.position);
        if frame.kind.handshake()
            || count == 0
            || frame.auth.carriers.len() != count
            || frame.auth.tags.len() != frame.context.path.len() - count
            || frame.auth.carriers.last() != Some(incoming)
        {
            return Err(Error::Proof);
        }
        let mut hash = transcript(frame)?;
        let mut tag = [0; 16];
        for (index, carrier) in frame.auth.carriers.iter().enumerate() {
            let signer = ordered(frame.context.path.len(), frame.reply, index);
            let next = ordered(frame.context.path.len(), frame.reply, index + 1);
            if !adjacent(&frame.context, signer, next, carrier) || !path_carrier(selected, carrier)
            {
                return Err(Error::Proof);
            }
            extend(&mut hash, carrier)?;
            tag = authenticate(&keys.incoming[signer], &hash, &tag);
        }
        if !bool::from(tag.ct_eq(&frame.auth.tags[0])) {
            return Err(Error::Proof);
        }
        let previous =
            frame.context.path[ordered(frame.context.path.len(), frame.reply, count - 1)];
        Ok(previous)
    }

    pub fn append(&self, frame: &mut Frame, carrier: Carrier) -> Result<(), Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("crypto.append_mac");
        let _profile = PROFILE.enter();
        let keys = self.keys.as_ref().ok_or(Error::State)?;
        let count = ordered(frame.context.path.len(), frame.reply, keys.position);
        let remaining = frame.context.path.len() - count - 1;
        if frame.kind.handshake() || remaining == 0 || frame.auth.carriers.len() != count {
            return Err(Error::State);
        }
        if count == 0 {
            if !frame.auth.tags.is_empty() {
                return Err(Error::Proof);
            }
            frame.auth.tags = vec![[0; 16]; remaining];
        } else {
            if frame.auth.tags.len() != remaining + 1 {
                return Err(Error::Proof);
            }
            frame.auth.tags.remove(0);
        }
        frame.auth.carriers.push(carrier);
        let mut hash = transcript(frame)?;
        for carrier in &frame.auth.carriers {
            extend(&mut hash, carrier)?;
        }
        for (offset, tag) in frame.auth.tags.iter_mut().enumerate() {
            let verifier = ordered(frame.context.path.len(), frame.reply, count + 1 + offset);
            *tag = authenticate(&keys.outgoing[verifier], &hash, tag);
        }
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

fn transcript(frame: &Frame) -> Result<Sha256, Error> {
    let mut hash = Sha256::new();
    hash.update(CHAIN);
    hash.update(frame.base()?);
    Ok(hash)
}

fn extend(hash: &mut Sha256, carrier: &Carrier) -> Result<(), Error> {
    let mut bytes = Vec::new();
    codec::encode_carrier(&mut bytes, carrier)?;
    hash.update(bytes);
    Ok(())
}

fn authenticate(key: &[u8; 32], hash: &Sha256, previous: &[u8; 16]) -> [u8; 16] {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts a 32-byte key");
    mac.update(CHAIN);
    mac.update(&hash.clone().finalize());
    mac.update(previous);
    let result = mac.finalize().into_bytes();
    let mut tag = [0; 16];
    tag.copy_from_slice(&result[..16]);
    tag
}

#[cfg(test)]
mod tests;
