//! One canonical signed transcript for stream and datagram adjacency. UDP adds
//! explicit ACK confirmation and channel-bound health exchanges, not encryption.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Kind {
    Init = 1,
    Challenge = 2,
    Ack = 3,
    Ready = 4,
    Ping = 5,
    Pong = 6,
}

impl Kind {
    pub(super) fn from_byte(value: u8) -> Result<Self, Error> {
        match value {
            1 => Ok(Self::Init),
            2 => Ok(Self::Challenge),
            3 => Ok(Self::Ack),
            4 => Ok(Self::Ready),
            5 => Ok(Self::Ping),
            6 => Ok(Self::Pong),
            _ => Err(Error::Authentication),
        }
    }
    fn domain(self) -> &'static [u8] {
        match self {
            Self::Init => INIT,
            Self::Challenge => CHALLENGE,
            Self::Ack => ACK,
            Self::Ready => b"meshway/adjacency/udpReady/v1\0",
            Self::Ping => b"meshway/adjacency/udpPing/v1\0",
            Self::Pong => b"meshway/adjacency/udpPong/v1\0",
        }
    }
}

pub(crate) fn encode(kind: Kind, body: &[u8]) -> Result<Vec<u8>, Error> {
    if body.is_empty() || body.len() > MAX_MESSAGE - 11 {
        return Err(Error::Authentication);
    }
    let mut message = Vec::with_capacity(11 + body.len());
    message.extend_from_slice(MAGIC);
    message.push(kind as u8);
    message.extend_from_slice(
        &u16::try_from(body.len())
            .map_err(|_| Error::Authentication)?
            .to_be_bytes(),
    );
    message.extend_from_slice(body);
    Ok(message)
}

/// A Ping filled with zeros to exactly `datagram` bytes: a path size probe.
/// Its signed body is the ordinary Ping; the filler carries nothing.
pub(crate) fn encode_padded(kind: Kind, body: &[u8], datagram: usize) -> Result<Vec<u8>, Error> {
    let mut message = encode(kind, body)?;
    if kind != Kind::Ping || datagram < message.len() || datagram > MAX_MESSAGE {
        return Err(Error::Authentication);
    }
    message.resize(datagram, 0);
    Ok(message)
}

pub(crate) fn decode(message: &[u8]) -> Result<(Kind, &[u8]), Error> {
    if message.len() < 12 || message.len() > MAX_MESSAGE || &message[..8] != MAGIC {
        return Err(Error::Authentication);
    }
    let length = usize::from(u16::from_be_bytes([message[9], message[10]]));
    let kind = Kind::from_byte(message[8])?;
    let end = 11 + length;
    // Only a Ping may carry filler (a size probe), and only zeros.
    if end > message.len()
        || (end < message.len()
            && (kind != Kind::Ping || message[end..].iter().any(|byte| *byte != 0)))
    {
        return Err(Error::Authentication);
    }
    Ok((kind, &message[11..end]))
}

pub(crate) struct Initiation {
    context: Vec<u8>,
    pub client: [u8; 32],
}

impl Initiation {
    /// A restarted UDP listener can verify an old channel's signed Ping without
    /// possessing its transient transcript. The fresh probe nonce becomes the
    /// client challenge; a NEW server nonce and a signed ACK are still required.
    pub fn from_probe(
        cfg: &Plan,
        peer: NodeId,
        carrier: &Carrier,
        body: &[u8],
    ) -> Result<([u8; 32], Self), Error> {
        if carrier.protocol != meshway_model::TransportProtocol::NativeUdp
            || carrier.dialer != peer
            || carrier.listener != cfg.node
            || body.len() != 160
        {
            return Err(Error::Authentication);
        }
        let context = context(cfg, carrier)?;
        let client = body[32..64].try_into().map_err(|_| Error::Authentication)?;
        let channel: [u8; 32] = body[64..96].try_into().map_err(|_| Error::Authentication)?;
        let mut health_context = context.clone();
        health_context.extend_from_slice(&channel);
        if parse_response(body, &health_context, &client)? != channel {
            return Err(Error::Authentication);
        }
        verify(
            cfg,
            peer,
            digest(Kind::Ping.domain(), &health_context, &client, &channel),
            &body[96..],
        )?;
        Ok((channel, Self { context, client }))
    }

    pub fn new(
        keys: &DataPlaneKeys,
        cfg: &Plan,
        carrier: &Carrier,
        boot: &Boot,
    ) -> Result<(Self, Vec<u8>), Error> {
        Self::new_bound(keys, cfg, carrier, None, boot)
    }

    pub fn new_bound(
        keys: &DataPlaneKeys,
        cfg: &Plan,
        carrier: &Carrier,
        binding: Option<meshway_model::Digest>,
        boot: &Boot,
    ) -> Result<(Self, Vec<u8>), Error> {
        let context = bound_context(cfg, carrier, binding)?;
        let client = boot_nonce(boot)?;
        let signature = cfg.sign(keys, digest(INIT, &context, &client, &[0; 32]))?;
        let mut intro = context.clone();
        intro.extend_from_slice(&client);
        intro.extend_from_slice(&signature);
        Ok((Self { context, client }, intro))
    }

    pub fn respond(self, boot: &Boot) -> Result<Transcript, Error> {
        Ok(Transcript {
            context: self.context,
            client: self.client,
            server: boot_nonce(boot)?,
        })
    }

    pub fn challenge(&self, cfg: &Plan, peer: NodeId, body: &[u8]) -> Result<Transcript, Error> {
        let server = parse_response(body, &self.context, &self.client)?;
        let transcript = Transcript {
            context: self.context.clone(),
            client: self.client,
            server,
        };
        transcript.verify(cfg, peer, Kind::Challenge, body)?;
        Ok(transcript)
    }
}

pub(crate) fn incoming(
    cfg: &Plan,
    carriers: &[(NodeId, Carrier)],
    intro: &[u8],
) -> Result<(NodeId, Carrier, Initiation), Error> {
    for (peer, carrier) in carriers {
        let context = context(cfg, carrier)?;
        if intro.len() != context.len() + 96 || !intro.starts_with(&context) {
            continue;
        }
        let client = intro[context.len()..context.len() + 32]
            .try_into()
            .map_err(|_| Error::Authentication)?;
        verify(
            cfg,
            *peer,
            digest(INIT, &context, &client, &[0; 32]),
            &intro[context.len() + 32..],
        )?;
        return Ok((*peer, carrier.clone(), Initiation { context, client }));
    }
    Err(Error::Authentication)
}

fn bound_context(
    cfg: &Plan,
    carrier: &Carrier,
    binding: Option<meshway_model::Digest>,
) -> Result<Vec<u8>, Error> {
    let mut context = context(cfg, carrier)?;
    if let Some(binding) = binding {
        context.extend_from_slice(b"meshway/adjacency/traversal/v1\0");
        context.extend_from_slice(binding.as_bytes());
    }
    Ok(context)
}

pub(crate) fn incoming_bound(
    cfg: &Plan,
    peer: NodeId,
    carrier: &Carrier,
    binding: meshway_model::Digest,
    intro: &[u8],
) -> Result<Initiation, Error> {
    let context = bound_context(cfg, carrier, Some(binding))?;
    if intro.len() != context.len() + 96 || !intro.starts_with(&context) {
        return Err(Error::Authentication);
    }
    let client = intro[context.len()..context.len() + 32]
        .try_into()
        .map_err(|_| Error::Authentication)?;
    verify(
        cfg,
        peer,
        digest(INIT, &context, &client, &[0; 32]),
        &intro[context.len() + 32..],
    )?;
    Ok(Initiation { context, client })
}

pub(crate) struct Transcript {
    context: Vec<u8>,
    pub client: [u8; 32],
    server: [u8; 32],
}

impl Transcript {
    /// Only the caller owning this exact live channel and its outstanding probe
    /// may use the result. Channel compare-and-replace remains the manager's job.
    pub fn probe_challenge(
        &self,
        cfg: &Plan,
        peer: NodeId,
        probe: [u8; 32],
        body: &[u8],
    ) -> Result<Self, Error> {
        Initiation {
            context: self.context.clone(),
            client: probe,
        }
        .challenge(cfg, peer, body)
    }

    /// The signed boot prefix of the peer's own nonce in this transcript:
    /// the listener's server nonce for a dialer, the dialer's client nonce
    /// (or its health probe nonce on passive recovery) for a listener.
    pub fn peer_boot(&self, dialer: bool) -> Boot {
        let nonce = if dialer { &self.server } else { &self.client };
        let mut boot = [0; 16];
        boot.copy_from_slice(&nonce[..16]);
        boot
    }

    pub fn id(&self) -> [u8; 32] {
        digest(
            b"meshway/adjacency/channel/v1\0",
            &self.context,
            &self.client,
            &self.server,
        )
        .into_bytes()
    }

    pub fn message(&self, keys: &DataPlaneKeys, cfg: &Plan, kind: Kind) -> Result<Vec<u8>, Error> {
        if !matches!(kind, Kind::Challenge | Kind::Ack | Kind::Ready) {
            return Err(Error::Authentication);
        }
        let proof = cfg.sign(
            keys,
            digest(kind.domain(), &self.context, &self.client, &self.server),
        )?;
        Ok(response(&self.context, &self.client, &self.server, proof))
    }

    pub fn verify(&self, cfg: &Plan, peer: NodeId, kind: Kind, body: &[u8]) -> Result<(), Error> {
        if !matches!(kind, Kind::Challenge | Kind::Ack | Kind::Ready)
            || parse_response(body, &self.context, &self.client)? != self.server
        {
            return Err(Error::Authentication);
        }
        verify(
            cfg,
            peer,
            digest(kind.domain(), &self.context, &self.client, &self.server),
            &body[96..],
        )
    }

    fn health_context(&self) -> Vec<u8> {
        let mut context = self.context.clone();
        context.extend_from_slice(&self.id());
        context
    }

    pub fn health(
        &self,
        keys: &DataPlaneKeys,
        cfg: &Plan,
        kind: Kind,
        nonce: &[u8; 32],
    ) -> Result<Vec<u8>, Error> {
        if !matches!(kind, Kind::Ping | Kind::Pong) {
            return Err(Error::Authentication);
        }
        let context = self.health_context();
        let id = self.id();
        let signature = cfg.sign(keys, digest(kind.domain(), &context, nonce, &id))?;
        Ok(response(&context, nonce, &id, signature))
    }

    pub fn verify_health(
        &self,
        cfg: &Plan,
        peer: NodeId,
        kind: Kind,
        body: &[u8],
    ) -> Result<[u8; 32], Error> {
        if !matches!(kind, Kind::Ping | Kind::Pong) || body.len() != 160 {
            return Err(Error::Authentication);
        }
        let nonce = body[32..64].try_into().map_err(|_| Error::Authentication)?;
        let context = self.health_context();
        let id = self.id();
        if parse_response(body, &context, &nonce)? != id {
            return Err(Error::Authentication);
        }
        verify(
            cfg,
            peer,
            digest(kind.domain(), &context, &nonce, &id),
            &body[96..],
        )?;
        Ok(nonce)
    }
}
