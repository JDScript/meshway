use crate::{Error, MAX_FRAME_SIZE, Result, TransportKind};

pub(super) const MAX_PAYLOAD: usize = 8_192;
pub(super) fn hop_prologue(transport: TransportKind) -> Vec<u8> {
    let mut result = b"meshway/lab/hop/3".to_vec();
    result.push(match transport {
        TransportKind::NativeUdp => 1,
        TransportKind::NativeTcp => 2,
        TransportKind::Vless => 3,
    });
    result
}

pub(super) const HOP_CONFIRM: &[u8] = b"meshway/hop/initiator-confirm/3";
pub(super) const HOP_ACK: &[u8] = b"meshway/hop/responder-ack/3";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    Init = 1,
    Ready = 2,
    Request = 3,
    Echo = 4,
    Packet = 5,
}

impl Phase {
    pub fn forward(self) -> bool {
        matches!(self, Self::Init | Self::Request | Self::Packet)
    }
}

pub(super) struct Packet {
    pub phase: Phase,
    pub id: [u8; 16],
    pub path: Vec<String>,
    pub body: Vec<u8>,
}

impl Packet {
    pub fn encode(&self) -> Result<Vec<u8>> {
        if !(2..=8).contains(&self.path.len()) {
            return Err(Error::Protocol);
        }
        let mut output = vec![1, self.phase as u8];
        output.extend_from_slice(&self.id);
        output.push(self.path.len() as u8);
        for id in &self.path {
            if !crate::config::valid_id(id) {
                return Err(Error::Protocol);
            }
            output.push(id.len() as u8);
            output.extend_from_slice(id.as_bytes());
        }
        output.extend_from_slice(&self.body);
        if output.len() > MAX_FRAME_SIZE - 24 {
            return Err(Error::Oversize);
        }
        Ok(output)
    }

    pub fn decode(input: &[u8]) -> Result<Self> {
        if input.len() < 19 || input.len() > MAX_FRAME_SIZE - 24 || input[0] != 1 {
            return Err(Error::Protocol);
        }
        let phase = match input[1] {
            1 => Phase::Init,
            2 => Phase::Ready,
            3 => Phase::Request,
            4 => Phase::Echo,
            5 => Phase::Packet,
            _ => return Err(Error::Protocol),
        };
        let id = input[2..18].try_into().map_err(|_| Error::Protocol)?;
        let count = input[18] as usize;
        if !(2..=8).contains(&count) {
            return Err(Error::Protocol);
        }
        let mut offset = 19;
        let mut path = Vec::with_capacity(count);
        for _ in 0..count {
            let length = *input.get(offset).ok_or(Error::Protocol)? as usize;
            offset += 1;
            if length == 0 || length > 64 {
                return Err(Error::Protocol);
            }
            let value = input.get(offset..offset + length).ok_or(Error::Protocol)?;
            let value = std::str::from_utf8(value)
                .map_err(|_| Error::Protocol)?
                .to_owned();
            if !crate::config::valid_id(&value) || path.contains(&value) {
                return Err(Error::Protocol);
            }
            path.push(value);
            offset += length;
        }
        let body = input[offset..].to_vec();
        if body.is_empty() {
            return Err(Error::Protocol);
        }
        Ok(Self {
            phase,
            id,
            path,
            body,
        })
    }

    pub fn e2e_prologue(&self) -> Vec<u8> {
        let mut result = b"meshway/lab/e2e/1".to_vec();
        result.extend_from_slice(&self.id);
        result.push(self.path.len() as u8);
        for node in &self.path {
            result.push(node.len() as u8);
            result.extend_from_slice(node.as_bytes());
        }
        result
    }
}
