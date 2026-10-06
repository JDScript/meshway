use super::{Error, MAX_FRAME, MAX_PLAINTEXT};
use crate::{PacketFacts, TcpSegment, Transport};
use meshway_configuration::execution::{BusinessProtocol, CircuitAuthorization};
use meshway_model::{Digest, EventId, KeyId, NetworkId, NodeId, Version};
use meshway_policy::admission::{Carrier, Traffic};
use meshway_transport_spec::{Port, TransportProtocol};
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    str::FromStr,
};

pub(super) const MAGIC: &[u8; 8] = b"MWCIR\0\0\x04";
pub(super) const MAX_PATH: usize = 8;

/// Fixed session routing prefix only; never an authentication decision.
pub(super) fn session_id(bytes: &[u8]) -> Result<Option<[u8; 32]>, Error> {
    if bytes.len() > MAX_FRAME || bytes.get(..8) != Some(MAGIC) {
        return Err(Error::Wire);
    }
    match bytes.get(8) {
        Some(1 | 2) => Ok(None),
        Some(3..=5) => Ok(Some(
            bytes
                .get(9..41)
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
}

impl Kind {
    pub fn handshake(self) -> bool {
        matches!(self, Self::Init | Self::Response)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Context {
    pub network: NetworkId,
    pub authorization: CircuitAuthorization,
    pub path: Vec<NodeId>,
    pub circuit: [u8; 32],
    pub permit: Digest,
    pub keys: Digest,
}

pub(super) struct Frame {
    pub context: Context,
    pub kind: Kind,
    pub reply: bool,
    pub sequence: u64,
    pub facts: Option<PacketFacts>,
    pub probe: Option<ProbeMetadata>,
    pub body: Vec<u8>,
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
    pub carriers: Vec<Carrier>,
    /// One chained authenticator for each remaining verifier, in traversal order.
    pub tags: Vec<[u8; 16]>,
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
        match self.authorization {
            CircuitAuthorization::Business(value) => {
                out.push(1);
                id(out, value.as_uuid());
            }
            CircuitAuthorization::Diagnostic(value) => {
                out.push(2);
                id(out, value.as_uuid());
            }
        }
        id(out, self.path[0].as_uuid()); // Explicit original origin, checked against the path.
        out.push(self.path.len() as u8);
        for node in &self.path {
            id(out, node.as_uuid());
        }
        out.extend_from_slice(&self.circuit);
        out.extend_from_slice(self.permit.as_bytes());
        out.extend_from_slice(self.keys.as_bytes());
        Ok(())
    }
}

impl Frame {
    pub fn validate(&self) -> Result<(), Error> {
        if (self.kind == Kind::Response) != self.exchange.is_some()
            || self
                .exchange
                .as_ref()
                .is_some_and(|exchange| exchange.public.len() != self.context.path.len())
            || (self.kind.handshake()
                && (!self.auth.carriers.is_empty() || !self.auth.tags.is_empty()))
            || self.auth.carriers.len() >= self.context.path.len()
            || self.auth.tags.len() >= self.context.path.len()
        {
            return Err(Error::Wire);
        }
        let diagnostic = matches!(
            self.context.authorization,
            CircuitAuthorization::Diagnostic(_)
        );
        let probe = matches!(self.kind, Kind::ProbeRequest | Kind::ProbeReply);
        if self.probe.is_some() != probe
            || (probe && !diagnostic)
            || (self.kind == Kind::Data && diagnostic)
        {
            return Err(Error::Wire);
        }
        match (self.kind, self.reply, self.facts) {
            (Kind::Init, false, None) if self.sequence == 0 && self.body.len() == 96 => {}
            (Kind::Response, true, None) if self.sequence == 0 && self.body.len() == 48 => {}
            (Kind::Data, _, Some(facts)) if self.sequence != u64::MAX => {
                facts.validate().map_err(|_| Error::Wire)?;
                if facts.len > MAX_PLAINTEXT || self.body.len() != facts.len + 16 {
                    return Err(Error::Wire);
                }
            }
            (Kind::ProbeRequest, false, None) | (Kind::ProbeReply, true, None)
                if self.sequence != u64::MAX && self.body.len() == 48 => {}
            _ => return Err(Error::Wire),
        }
        if self.proofs.len() >= self.context.path.len() {
            return Err(Error::Wire);
        }
        Ok(())
    }

    pub fn base(&self) -> Result<Vec<u8>, Error> {
        self.validate()?;
        let mut out = Vec::with_capacity(self.body.len() + 512);
        out.extend_from_slice(MAGIC);
        out.push(match self.kind {
            Kind::Init => 1,
            Kind::Response => 2,
            Kind::Data => 3,
            Kind::ProbeRequest => 4,
            Kind::ProbeReply => 5,
        });
        if self.kind.handshake() {
            self.context.encode(&mut out)?;
        } else {
            out.extend_from_slice(&self.context.circuit);
        }
        out.push(u8::from(self.reply));
        out.extend_from_slice(&self.sequence.to_be_bytes());
        if let Some(facts) = self.facts {
            encode_facts(&mut out, facts);
        }
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
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("codec.encode");
        let _profile = PROFILE.enter();
        let mut out = self.base()?;
        if self.kind.handshake() {
            out.push(self.proofs.len() as u8);
            for proof in &self.proofs {
                proof.encode(&mut out)?;
            }
        } else {
            if !self.proofs.is_empty()
                || self.auth.carriers.is_empty()
                || self.auth.tags.len() != self.context.path.len() - self.auth.carriers.len()
            {
                return Err(Error::Wire);
            }
            out.push(self.auth.carriers.len() as u8);
            for carrier in &self.auth.carriers {
                encode_compact_carrier(&mut out, &self.context, carrier)?;
            }
            for tag in &self.auth.tags {
                out.extend_from_slice(tag);
            }
        }
        if out.len() > MAX_FRAME {
            return Err(Error::Wire);
        }
        Ok(out)
    }

    pub fn decode(bytes: &[u8], retained: Option<&Context>) -> Result<Self, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("codec.decode");
        let _profile = PROFILE.enter();
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
            _ => return Err(Error::Wire),
        };
        let context = if kind.handshake() {
            let network = r.id()?;
            let authorization = match r.byte()? {
                1 => CircuitAuthorization::Business(r.id()?),
                2 => CircuitAuthorization::Diagnostic(r.id()?),
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
            let permit = Digest::from_bytes(r.array()?);
            let keys = Digest::from_bytes(r.array()?);
            Context {
                network,
                authorization,
                path,
                circuit,
                permit,
                keys,
            }
        } else {
            let circuit: [u8; 32] = r.array()?;
            let context = retained
                .filter(|context| context.circuit == circuit)
                .ok_or(Error::State)?;
            context.clone()
        };
        let reply = r.boolean()?;
        let sequence = r.u64()?;
        let facts = if kind == Kind::Data {
            Some(decode_facts(&mut r)?)
        } else {
            None
        };
        let probe = if matches!(kind, Kind::ProbeRequest | Kind::ProbeReply) {
            Some(ProbeMetadata {
                measurement_id: r.id()?,
                expires_at_ms: r.u64()?,
            })
        } else {
            None
        };
        let length = usize::try_from(r.u32()?).map_err(|_| Error::Wire)?;
        if length > MAX_PLAINTEXT + 16 {
            return Err(Error::Wire);
        }
        let body = r.take(length)?.to_vec();
        let exchange = if kind == Kind::Response {
            let init = r.array()?;
            let public = (0..context.path.len())
                .map(|_| r.array())
                .collect::<Result<_, _>>()?;
            Some(Exchange { init, public })
        } else {
            None
        };
        let count = usize::from(r.byte()?);
        if count == 0 || count >= context.path.len() {
            return Err(Error::Wire);
        }
        let mut proofs = Vec::with_capacity(count);
        let mut auth = HopAuth::default();
        if kind.handshake() {
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
            for _ in 0..count {
                auth.carriers
                    .push(decode_compact_carrier(&mut r, &context)?);
            }
            for _ in count..context.path.len() {
                auth.tags.push(r.array()?);
            }
        }
        if !r.0.is_empty() {
            return Err(Error::Wire);
        }
        let frame = Self {
            context,
            kind,
            reply,
            sequence,
            facts,
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

fn encode_compact_carrier(
    out: &mut Vec<u8>,
    context: &Context,
    carrier: &Carrier,
) -> Result<(), Error> {
    for node in [carrier.dialer, carrier.listener] {
        let position = context
            .path
            .iter()
            .position(|candidate| *candidate == node)
            .ok_or(Error::Wire)?;
        out.push(position as u8);
    }
    encode_listener(out, carrier)
}

fn decode_compact_carrier(r: &mut Reader<'_>, context: &Context) -> Result<Carrier, Error> {
    let dialer = *context
        .path
        .get(usize::from(r.byte()?))
        .ok_or(Error::Wire)?;
    let listener = *context
        .path
        .get(usize::from(r.byte()?))
        .ok_or(Error::Wire)?;
    decode_listener(r, dialer, listener)
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

fn encode_ip(out: &mut Vec<u8>, ip: IpAddr) {
    match ip {
        IpAddr::V4(ip) => {
            out.push(4);
            out.extend_from_slice(&ip.octets());
        }
        IpAddr::V6(ip) => {
            out.push(6);
            out.extend_from_slice(&ip.octets());
        }
    }
}
fn decode_ip(r: &mut Reader<'_>) -> Result<IpAddr, Error> {
    match r.byte()? {
        4 => Ok(Ipv4Addr::from(r.array::<4>()?).into()),
        6 => Ok(Ipv6Addr::from(r.array::<16>()?).into()),
        _ => Err(Error::Wire),
    }
}
fn encode_facts(out: &mut Vec<u8>, facts: PacketFacts) {
    out.extend_from_slice(&(facts.len as u32).to_be_bytes());
    encode_ip(out, facts.traffic.source);
    encode_ip(out, facts.traffic.destination);
    for port in [facts.traffic.source_port, facts.traffic.destination_port] {
        out.extend_from_slice(&port.map_or(0, Port::get).to_be_bytes());
    }
    match facts.transport {
        Transport::Tcp(segment) => {
            out.push(1);
            out.extend_from_slice(&segment.sequence.to_be_bytes());
            out.extend_from_slice(&segment.acknowledgement.to_be_bytes());
            out.push(segment.flags);
            out.extend_from_slice(&segment.payload_len.to_be_bytes());
        }
        Transport::Udp => out.push(2),
        Transport::Echo {
            reply,
            identifier,
            sequence,
        } => {
            out.push(3);
            out.push(u8::from(reply));
            out.extend_from_slice(&identifier.to_be_bytes());
            out.extend_from_slice(&sequence.to_be_bytes());
        }
    }
}
fn decode_facts(r: &mut Reader<'_>) -> Result<PacketFacts, Error> {
    let len = usize::try_from(r.u32()?).map_err(|_| Error::Wire)?;
    let source = decode_ip(r)?;
    let destination = decode_ip(r)?;
    let mut port = || {
        let n = r.u16()?;
        if n == 0 {
            Ok(None)
        } else {
            Port::new(n).map(Some).map_err(|_| Error::Wire)
        }
    };
    let source_port = port()?;
    let destination_port = port()?;
    let (protocol, transport) = match r.byte()? {
        1 => (
            BusinessProtocol::Tcp,
            Transport::Tcp(TcpSegment {
                sequence: r.u32()?,
                acknowledgement: r.u32()?,
                flags: r.byte()?,
                payload_len: r.u16()?,
            }),
        ),
        2 => (BusinessProtocol::Udp, Transport::Udp),
        3 => (
            BusinessProtocol::IcmpEcho,
            Transport::Echo {
                reply: r.boolean()?,
                identifier: r.u16()?,
                sequence: r.u16()?,
            },
        ),
        _ => return Err(Error::Wire),
    };
    let facts = PacketFacts {
        len,
        traffic: Traffic {
            source,
            destination,
            protocol,
            source_port,
            destination_port,
        },
        transport,
    };
    facts.validate().map_err(|_| Error::Wire)?;
    Ok(facts)
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
    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_be_bytes(self.array()?))
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
