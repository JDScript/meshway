use super::{Error, MAX_FRAME, MAX_PLAINTEXT};
use crate::PacketBuffer;
mod data;
use super::CircuitGrant;
use crate::netmap::Purpose;
use meshway_model::{Digest, EventId, KeyId, NetworkId, NodeId, Version};
use meshway_policy::admission::Carrier;
use meshway_policy::compile::netmap::Target;
use meshway_transport_spec::TransportProtocol;
use std::{str::FromStr, sync::Arc};

pub(super) const MAGIC: &[u8; 8] = b"MWCIR\0\0\x06";
pub(super) const DATA_HEADER: usize = data::HEADER;
pub(super) const MAX_PATH: usize = 8;
/// Bound of each ordered exit node and waypoint list in a netmap grant.
pub(super) const MAX_TARGET_NODES: usize = 8;
static ENCODE: meshway_logging::profile::Metric =
    meshway_logging::profile::Metric::new("codec.encode");
static DECODE: meshway_logging::profile::Metric =
    meshway_logging::profile::Metric::new("codec.decode");

/// Scheduling hint for carriers: a well-formed candidate frame that is not a
/// Data frame. Never an authentication or admission decision.
pub fn is_control_frame(bytes: &[u8]) -> bool {
    bytes.get(..8) == Some(MAGIC) && bytes.get(8).is_some_and(|kind| *kind != 3)
}

/// Fixed session routing prefix only; never an authentication decision.
pub(super) fn session_id(bytes: &[u8]) -> Result<Option<[u8; 32]>, Error> {
    if bytes.len() > MAX_FRAME || bytes.get(..8) != Some(MAGIC) {
        return Err(Error::Wire);
    }
    match bytes.get(8) {
        Some(1 | 2) => Ok(None),
        Some(3..=7) => Ok(Some(
            bytes
                .get(if bytes[8] == 3 { 10..42 } else { 9..41 })
                .ok_or(Error::Wire)?
                .try_into()
                .map_err(|_| Error::Wire)?,
        )),
        _ => Err(Error::Wire),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Init,
    Response,
    Data,
    ProbeRequest,
    ProbeReply,
    DnsRequest,
    DnsReply,
}

impl Kind {
    pub fn handshake(self) -> bool {
        matches!(self, Self::Init | Self::Response)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Context {
    pub network: NetworkId,
    pub grant: CircuitGrant,
    pub path: Vec<NodeId>,
    pub circuit: [u8; 32],
    /// Path authorization digest (netmap) or legacy permit digest.
    pub authorization: Digest,
    pub keys: Digest,
    /// Origin UTC time of the INIT. Receivers accept it only within the
    /// handshake window, which bounds every closed-circuit replay tombstone.
    pub initiated_at_ms: u64,
}

pub(super) struct Frame {
    pub context: Arc<Context>,
    pub kind: Kind,
    pub reply: bool,
    pub sequence: u64,
    pub probe: Option<ProbeMetadata>,
    pub body: PacketBuffer,
    pub proofs: Vec<Proof>,
    pub exchange: Option<Exchange>,
    pub auth: HopAuth,
}

/// Signed response: the INIT body digest and complete ordered ephemeral list.
pub(super) struct Exchange {
    pub init: [u8; 32],
    pub public: Vec<[u8; 32]>,
}

#[derive(Default)]
pub(super) struct HopAuth {
    pub tag: Option<[u8; 16]>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ProbeMetadata {
    pub measurement_id: EventId,
    pub expires_at_ms: u64,
}

pub(super) struct Proof {
    pub key: KeyId,
    pub generation: Version,
    pub carrier: Carrier,
    pub signature: [u8; 64],
    pub ephemeral: [u8; 32],
}

pub(super) fn id(out: &mut Vec<u8>, value: &uuid::Uuid) {
    out.extend_from_slice(value.as_bytes());
}

impl Context {
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<(), Error> {
        if !(2..=MAX_PATH).contains(&self.path.len())
            || self
                .path
                .iter()
                .enumerate()
                .any(|(i, n)| self.path[..i].contains(n))
        {
            return Err(Error::Wire);
        }
        id(out, self.network.as_uuid());
        out.push(match self.grant.purpose {
            Purpose::Business => 3,
            Purpose::Diagnostic => 4,
            Purpose::Dns => 5,
        });
        encode_target(out, &self.grant.target)?;
        id(out, self.path[0].as_uuid()); // Explicit original origin, checked against the path.
        out.push(self.path.len() as u8);
        for node in &self.path {
            id(out, node.as_uuid());
        }
        out.extend_from_slice(&self.circuit);
        out.extend_from_slice(self.authorization.as_bytes());
        out.extend_from_slice(self.keys.as_bytes());
        out.extend_from_slice(&self.initiated_at_ms.to_be_bytes());
        Ok(())
    }

    pub fn diagnostic(&self) -> bool {
        self.grant.diagnostic()
    }
}

fn encode_nodes(out: &mut Vec<u8>, nodes: &[NodeId], minimum: usize) -> Result<(), Error> {
    if !(minimum..=MAX_TARGET_NODES).contains(&nodes.len())
        || nodes
            .iter()
            .enumerate()
            .any(|(i, n)| nodes[..i].contains(n))
    {
        return Err(Error::Wire);
    }
    out.push(nodes.len() as u8);
    for node in nodes {
        id(out, node.as_uuid());
    }
    Ok(())
}

fn encode_target(out: &mut Vec<u8>, target: &Target) -> Result<(), Error> {
    match target {
        Target::Mesh { node } => {
            out.push(1);
            id(out, node.as_uuid());
        }
        Target::Exit { nodes, via } => {
            out.push(2);
            encode_nodes(out, nodes, 1)?;
            encode_nodes(out, via, 0)?;
        }
    }
    Ok(())
}

fn decode_nodes(r: &mut Reader<'_>, minimum: usize) -> Result<Vec<NodeId>, Error> {
    let count = usize::from(r.byte()?);
    if !(minimum..=MAX_TARGET_NODES).contains(&count) {
        return Err(Error::Wire);
    }
    let mut nodes = Vec::with_capacity(count);
    for _ in 0..count {
        let node = r.id()?;
        if nodes.contains(&node) {
            return Err(Error::Wire);
        }
        nodes.push(node);
    }
    Ok(nodes)
}

fn decode_target(r: &mut Reader<'_>) -> Result<Target, Error> {
    match r.byte()? {
        1 => Ok(Target::Mesh { node: r.id()? }),
        2 => Ok(Target::Exit {
            nodes: decode_nodes(r, 1)?,
            via: decode_nodes(r, 0)?,
        }),
        _ => Err(Error::Wire),
    }
}

impl Frame {
    /// Data takes its header from headroom and overwrites its sole hop tag in
    /// tailroom. Setup/probe encoding remains bounded and outside this hot path.
    pub fn encode_owned(mut self) -> Result<PacketBuffer, Error> {
        if self.kind != Kind::Data {
            return self.encode().map(Into::into);
        }
        let _profile = ENCODE.enter();
        self.validate()?;
        if !self.proofs.is_empty() {
            return Err(Error::Wire);
        }
        let length = self.body.len() as u32;
        let tag = self.auth.tag.ok_or(Error::Wire)?;
        let header = self
            .body
            .prepend(data::HEADER)
            .map_err(|_| Error::Capacity)?;
        header[..8].copy_from_slice(MAGIC);
        header[8] = 3;
        header[9] = u8::from(self.reply);
        header[10..42].copy_from_slice(&self.context.circuit);
        header[42..50].copy_from_slice(&self.sequence.to_be_bytes());
        header[50..54].copy_from_slice(&length.to_be_bytes());
        self.body
            .extend(16)
            .map_err(|_| Error::Capacity)?
            .copy_from_slice(&tag);
        Ok(self.body)
    }

    pub fn decode_owned(
        mut bytes: PacketBuffer,
        kept: Option<&Arc<Context>>,
    ) -> Result<Self, Error> {
        if bytes.get(8) != Some(&3) {
            return Self::decode(&bytes, kept);
        }
        let _profile = DECODE.enter();
        let view = data::DataView::decode(&bytes)?;
        let context = kept
            .filter(|context| &context.circuit == view.circuit)
            .ok_or(Error::State)?
            .clone();
        let (reply, sequence, length, tag) = (view.reply, view.sequence, view.body.len(), view.tag);
        bytes.trim_front(data::HEADER);
        bytes.truncate(length);
        let frame = Self {
            context,
            kind: Kind::Data,
            reply,
            sequence,
            probe: None,
            body: bytes,
            proofs: Vec::new(),
            exchange: None,
            auth: HopAuth { tag: Some(tag) },
        };
        frame.validate()?;
        Ok(frame)
    }

    pub fn validate(&self) -> Result<(), Error> {
        if (self.kind == Kind::Response) != self.exchange.is_some()
            || self
                .exchange
                .as_ref()
                .is_some_and(|exchange| exchange.public.len() != self.context.path.len())
            || (self.kind.handshake() && self.auth.tag.is_some())
        {
            return Err(Error::Wire);
        }
        let diagnostic = self.context.diagnostic();
        let probe = matches!(self.kind, Kind::ProbeRequest | Kind::ProbeReply);
        let dns = matches!(self.kind, Kind::DnsRequest | Kind::DnsReply);
        if self.probe.is_some() != (probe || dns)
            || (probe && !diagnostic)
            || (dns && self.context.grant.purpose != Purpose::Dns)
            || (self.kind == Kind::Data && self.context.grant.purpose != Purpose::Business)
        {
            return Err(Error::Wire);
        }
        match (self.kind, self.reply) {
            (Kind::Init, false) if self.sequence == 0 && self.body.len() == 96 => {}
            (Kind::Response, true) if self.sequence == 0 && self.body.len() == 48 => {}
            (Kind::Data, _)
                if self.sequence != u64::MAX
                    && (16..=MAX_PLAINTEXT + 16).contains(&self.body.len()) => {}
            (Kind::ProbeRequest, false) | (Kind::ProbeReply, true)
                if self.sequence != u64::MAX && self.body.len() == 48 => {}
            (Kind::DnsRequest, false) | (Kind::DnsReply, true)
                if self.sequence != u64::MAX
                    && (29..=super::MAX_DNS_BYTES + 17).contains(&self.body.len()) => {}
            _ => return Err(Error::Wire),
        }
        if self.proofs.len() >= self.context.path.len() {
            return Err(Error::Wire);
        }
        Ok(())
    }

    pub fn base(&self) -> Result<Vec<u8>, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::detail("codec.base", 1);
        let _profile = PROFILE.enter();
        self.validate()?;
        let mut out = Vec::with_capacity(self.body.len() + 512);
        out.extend_from_slice(MAGIC);
        out.push(match self.kind {
            Kind::Init => 1,
            Kind::Response => 2,
            Kind::Data => 3,
            Kind::ProbeRequest => 4,
            Kind::ProbeReply => 5,
            Kind::DnsRequest => 6,
            Kind::DnsReply => 7,
        });
        if self.kind == Kind::Data {
            out.push(u8::from(self.reply));
        }
        if self.kind.handshake() {
            self.context.encode(&mut out)?;
        } else {
            out.extend_from_slice(&self.context.circuit);
        }
        if self.kind != Kind::Data {
            out.push(u8::from(self.reply));
        }
        out.extend_from_slice(&self.sequence.to_be_bytes());
        if let Some(probe) = self.probe {
            id(&mut out, probe.measurement_id.as_uuid());
            out.extend_from_slice(&probe.expires_at_ms.to_be_bytes());
        }
        out.extend_from_slice(&(self.body.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.body);
        if let Some(exchange) = &self.exchange {
            out.extend_from_slice(&exchange.init);
            for public in &exchange.public {
                out.extend_from_slice(public);
            }
        }
        Ok(out)
    }

    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let _profile = ENCODE.enter();
        let mut out = self.base()?;
        if self.kind.handshake() {
            out.push(self.proofs.len() as u8);
            for proof in &self.proofs {
                proof.encode(&mut out)?;
            }
        } else {
            if !self.proofs.is_empty() {
                return Err(Error::Wire);
            }
            out.extend_from_slice(&self.auth.tag.ok_or(Error::Wire)?);
        }
        if out.len() > MAX_FRAME {
            return Err(Error::Wire);
        }
        Ok(out)
    }

    pub fn decode(bytes: &[u8], kept: Option<&Arc<Context>>) -> Result<Self, Error> {
        if bytes.get(8) == Some(&3) {
            return Self::decode_owned(bytes.into(), kept);
        }
        let _profile = DECODE.enter();
        if bytes.len() > MAX_FRAME {
            return Err(Error::Wire);
        }
        let mut r = Reader(bytes);
        if r.take(8)? != MAGIC {
            return Err(Error::Wire);
        }
        let kind = match r.byte()? {
            1 => Kind::Init,
            2 => Kind::Response,
            3 => Kind::Data,
            4 => Kind::ProbeRequest,
            5 => Kind::ProbeReply,
            6 => Kind::DnsRequest,
            7 => Kind::DnsReply,
            _ => return Err(Error::Wire),
        };
        let flags = if kind == Kind::Data {
            Some(r.boolean()?)
        } else {
            None
        };
        let context = if kind.handshake() {
            let network = r.id()?;
            let grant = match r.byte()? {
                3 => CircuitGrant {
                    purpose: Purpose::Business,
                    target: decode_target(&mut r)?,
                },
                4 => CircuitGrant {
                    purpose: Purpose::Diagnostic,
                    target: decode_target(&mut r)?,
                },
                5 => CircuitGrant {
                    purpose: Purpose::Dns,
                    target: decode_target(&mut r)?,
                },
                _ => return Err(Error::Wire),
            };
            let origin: NodeId = r.id()?;
            let count = usize::from(r.byte()?);
            if !(2..=MAX_PATH).contains(&count) {
                return Err(Error::Wire);
            }
            let mut path = Vec::with_capacity(count);
            for _ in 0..count {
                let node = r.id()?;
                if path.contains(&node) {
                    return Err(Error::Wire);
                }
                path.push(node);
            }
            if path[0] != origin {
                return Err(Error::Wire);
            }
            let circuit = r.array()?;
            let authorization = Digest::from_bytes(r.array()?);
            let keys = Digest::from_bytes(r.array()?);
            let initiated_at_ms = r.u64()?;
            Arc::new(Context {
                network,
                grant,
                path,
                circuit,
                authorization,
                keys,
                initiated_at_ms,
            })
        } else {
            let circuit: [u8; 32] = r.array()?;
            let context = kept
                .filter(|context| context.circuit == circuit)
                .ok_or(Error::State)?;
            context.clone()
        };
        let reply = match flags {
            Some(reply) => reply,
            None => r.boolean()?,
        };
        let sequence = r.u64()?;
        let probe = if matches!(
            kind,
            Kind::ProbeRequest | Kind::ProbeReply | Kind::DnsRequest | Kind::DnsReply
        ) {
            Some(ProbeMetadata {
                measurement_id: r.id()?,
                expires_at_ms: r.u64()?,
            })
        } else {
            None
        };
        let length = usize::try_from(r.u32()?).map_err(|_| Error::Wire)?;
        if length > MAX_PLAINTEXT + 16
            || (matches!(kind, Kind::DnsRequest | Kind::DnsReply)
                && length > super::MAX_DNS_BYTES + 17)
        {
            return Err(Error::Wire);
        }
        let body = PacketBuffer::from(r.take(length)?);
        let exchange = if kind == Kind::Response {
            let init = r.array()?;
            let public = (0..context.path.len())
                .map(|_| r.array())
                .collect::<Result<_, _>>()?;
            Some(Exchange { init, public })
        } else {
            None
        };
        let mut proofs = Vec::new();
        let mut auth = HopAuth::default();
        if kind.handshake() {
            let count = usize::from(r.byte()?);
            if count == 0 || count >= context.path.len() {
                return Err(Error::Wire);
            }
            for _ in 0..count {
                proofs.push(Proof {
                    key: r.id()?,
                    generation: Version::new(r.u64()?).map_err(|_| Error::Wire)?,
                    carrier: decode_carrier(&mut r)?,
                    ephemeral: r.array()?,
                    signature: r.array()?,
                });
            }
        } else {
            auth.tag = Some(r.array()?);
        }
        if !r.0.is_empty() {
            return Err(Error::Wire);
        }
        let frame = Self {
            context,
            kind,
            reply,
            sequence,
            probe,
            body,
            proofs,
            exchange,
            auth,
        };
        frame.validate()?;
        Ok(frame)
    }
}

impl Proof {
    pub fn prefix(&self, out: &mut Vec<u8>) -> Result<(), Error> {
        id(out, self.key.as_uuid());
        out.extend_from_slice(&self.generation.get().to_be_bytes());
        encode_carrier(out, &self.carrier)?;
        out.extend_from_slice(&self.ephemeral);
        Ok(())
    }
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<(), Error> {
        self.prefix(out)?;
        out.extend_from_slice(&self.signature);
        Ok(())
    }
}

pub(super) fn encode_carrier(out: &mut Vec<u8>, carrier: &Carrier) -> Result<(), Error> {
    if carrier.listener_key.is_empty() || carrier.listener_key.len() > 255 {
        return Err(Error::Unsupported);
    }
    id(out, carrier.dialer.as_uuid());
    id(out, carrier.listener.as_uuid());
    encode_listener(out, carrier)
}

fn encode_listener(out: &mut Vec<u8>, carrier: &Carrier) -> Result<(), Error> {
    if carrier.listener_key.is_empty() || carrier.listener_key.len() > 255 {
        return Err(Error::Unsupported);
    }
    out.push(match carrier.protocol {
        TransportProtocol::NativeUdp => 1,
        TransportProtocol::NativeTcp => 2,
        TransportProtocol::Vless => 3,
        TransportProtocol::Trojan => 4,
    });
    out.push(carrier.listener_key.len() as u8);
    out.extend_from_slice(carrier.listener_key.as_bytes());
    Ok(())
}

fn decode_carrier(r: &mut Reader<'_>) -> Result<Carrier, Error> {
    let dialer = r.id()?;
    let listener = r.id()?;
    decode_listener(r, dialer, listener)
}

fn decode_listener(r: &mut Reader<'_>, dialer: NodeId, listener: NodeId) -> Result<Carrier, Error> {
    let protocol = match r.byte()? {
        1 => TransportProtocol::NativeUdp,
        2 => TransportProtocol::NativeTcp,
        3 => TransportProtocol::Vless,
        4 => TransportProtocol::Trojan,
        _ => return Err(Error::Wire),
    };
    let length = usize::from(r.byte()?);
    if length == 0 {
        return Err(Error::Wire);
    }
    let listener_key = std::str::from_utf8(r.take(length)?)
        .map_err(|_| Error::Wire)?
        .to_owned();
    Ok(Carrier {
        dialer,
        listener,
        listener_key,
        protocol,
    })
}

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let (head, tail) = self.0.split_at_checked(n).ok_or(Error::Wire)?;
        self.0 = tail;
        Ok(head)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        self.take(N)?.try_into().map_err(|_| Error::Wire)
    }
    fn byte(&mut self) -> Result<u8, Error> {
        Ok(self.array::<1>()?[0])
    }
    fn boolean(&mut self) -> Result<bool, Error> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::Wire),
        }
    }
    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_be_bytes(self.array()?))
    }
    fn id<T: FromStr>(&mut self) -> Result<T, Error> {
        uuid::Uuid::from_bytes(self.array()?)
            .hyphenated()
            .to_string()
            .parse()
            .map_err(|_| Error::Wire)
    }
}

#[cfg(test)]
mod tests;
