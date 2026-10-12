use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

const COOKIE: [u8; 4] = 0x2112_a442_u32.to_be_bytes();
const HEADER: usize = 20;
const XOR_MAPPED: u16 = 0x0020;
const ERROR_CODE: u16 = 0x0009;
const UNKNOWN_ATTRIBUTES: u16 = 0x000a;
const FINGERPRINT: u16 = 0x8028;
/// Comprehension-optional filler of a size probe; a server ignores it and
/// answers with an ordinary small response.
const SIZE_FILLER: u16 = 0xc0de;
const MAX_UNKNOWN: usize = 16;
pub const MAX_DATAGRAM_SIZE: usize = 2048;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TransactionId([u8; 12]);

impl TransactionId {
    /// Creates a cryptographically random 96-bit ID for a new transaction.
    pub fn random() -> Result<Self, Error> {
        let mut bytes = [0; 12];
        getrandom::fill(&mut bytes).map_err(|_| Error::Random)?;
        Ok(Self(bytes))
    }

    /// Restores an existing ID (including literal test vectors). New outbound
    /// transactions should use `random`; retransmissions reuse their original ID.
    pub const fn from_bytes(bytes: [u8; 12]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 12] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Fingerprint {
    Omit,
    #[default]
    Include,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageKind {
    Request,
    SuccessResponse,
    ErrorResponse,
}

/// Classifies only RFC 8489 Binding request/success/error headers. This is a
/// demux hint, not validation: malformed bodies still require parser rejection.
/// Unsupported methods, indications, legacy STUN and Meshway magic return None.
pub fn classify(bytes: &[u8]) -> Option<MessageKind> {
    if bytes.len() < HEADER || bytes[4..8] != COOKIE {
        return None;
    }
    match [bytes[0], bytes[1]] {
        [0x00, 0x01] => Some(MessageKind::Request),
        [0x01, 0x01] => Some(MessageKind::SuccessResponse),
        [0x01, 0x11] => Some(MessageKind::ErrorResponse),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("STUN datagram size is invalid")]
    Size,
    #[error("unsupported STUN header")]
    Header,
    #[error("unexpected STUN message class")]
    MessageKind,
    #[error("STUN response source mismatch")]
    Source,
    #[error("STUN transaction mismatch")]
    Transaction,
    #[error("malformed STUN attribute")]
    Attribute,
    #[error("duplicate authoritative STUN attribute")]
    Duplicate,
    #[error("invalid STUN fingerprint")]
    Fingerprint,
    #[error("unsupported STUN authentication semantics")]
    Authentication,
    #[error("unsupported mandatory STUN attribute")]
    UnknownAttribute,
    #[error("too many unknown mandatory STUN attributes")]
    TooManyUnknownAttributes,
    #[error("missing STUN mapped address")]
    MissingMappedAddress,
    #[error("STUN error response ({code})")]
    ErrorResponse { code: u16 },
    #[error("STUN transaction randomness unavailable")]
    Random,
}

/// An unauthenticated observation, never a connectivity or identity grant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MappedAddress(SocketAddr);

impl MappedAddress {
    pub const fn socket_addr(self) -> SocketAddr {
        self.0
    }
}

/// Validated request with bounded rejection metadata. The response encoder
/// selects success, 400, or 420; unsupported authentication is rejected earlier.
#[derive(Clone, Debug)]
pub struct BindingRequest {
    transaction: TransactionId,
    fingerprint: Fingerprint,
    unknown: Vec<u16>,
    bad_request: bool,
}

impl BindingRequest {
    pub const fn transaction_id(&self) -> TransactionId {
        self.transaction
    }
}

struct Parsed {
    kind: MessageKind,
    transaction: TransactionId,
    mapped: Option<SocketAddr>,
    error: Option<u16>,
    fingerprint: bool,
    integrity: bool,
    unknown: Vec<u16>,
    response_attribute: bool,
}

pub fn encode_request(transaction: TransactionId, fingerprint: Fingerprint) -> Vec<u8> {
    let mut bytes = header(MessageKind::Request, transaction);
    finish(&mut bytes, fingerprint);
    bytes
}

/// A Binding request of exactly `datagram` UDP payload bytes, filled with
/// one comprehension-optional attribute: sent with DF, an answer proves the
/// path carried that size. `None` below the smallest such request or above
/// [`MAX_DATAGRAM_SIZE`]; `datagram` must be a multiple of 4.
pub fn encode_size_probe(transaction: TransactionId, datagram: usize) -> Option<Vec<u8>> {
    let filler = datagram.checked_sub(HEADER + 4 + 8)?;
    if !datagram.is_multiple_of(4) || datagram > MAX_DATAGRAM_SIZE {
        return None;
    }
    let mut bytes = header(MessageKind::Request, transaction);
    bytes.extend_from_slice(&SIZE_FILLER.to_be_bytes());
    bytes.extend_from_slice(&u16::try_from(filler).ok()?.to_be_bytes());
    bytes.resize(bytes.len() + filler, 0);
    finish(&mut bytes, Fingerprint::Include);
    (bytes.len() == datagram).then_some(bytes)
}

pub fn parse_request(bytes: &[u8]) -> Result<BindingRequest, Error> {
    let parsed = parse(bytes)?;
    if parsed.kind != MessageKind::Request {
        return Err(Error::MessageKind);
    }
    Ok(BindingRequest {
        transaction: parsed.transaction,
        fingerprint: if parsed.fingerprint {
            Fingerprint::Include
        } else {
            Fingerprint::Omit
        },
        unknown: parsed.unknown,
        bad_request: parsed.response_attribute,
    })
}

/// Source must be the real `recv_from` result, not an address in request data.
/// Does not echo arbitrary attributes. The service additionally enforces the
/// 2.2x per-datagram output budget (including error replies).
pub fn encode_response(request: &BindingRequest, source: SocketAddr) -> Vec<u8> {
    let kind = if request.bad_request || !request.unknown.is_empty() {
        MessageKind::ErrorResponse
    } else {
        MessageKind::SuccessResponse
    };
    let mut bytes = header(kind, request.transaction);
    if !request.unknown.is_empty() {
        attribute(&mut bytes, ERROR_CODE, b"\0\0\x04\x14Unknown");
        let values: Vec<u8> = request
            .unknown
            .iter()
            .flat_map(|n| n.to_be_bytes())
            .collect();
        attribute(&mut bytes, UNKNOWN_ATTRIBUTES, &values);
    } else if request.bad_request {
        attribute(&mut bytes, ERROR_CODE, b"\0\0\x04\x00Bad Request");
    } else {
        let mut value = vec![0, if source.is_ipv4() { 1 } else { 2 }];
        value.extend_from_slice(&(source.port() ^ 0x2112).to_be_bytes());
        let mask = address_mask(request.transaction);
        match source.ip() {
            IpAddr::V4(ip) => value.extend(ip.octets().iter().zip(mask).map(|(a, b)| a ^ b)),
            IpAddr::V6(ip) => value.extend(ip.octets().iter().zip(mask).map(|(a, b)| a ^ b)),
        }
        attribute(&mut bytes, XOR_MAPPED, &value);
    }
    finish(&mut bytes, request.fingerprint);
    bytes
}

/// Validates the actual UDP sender and transaction before returning an address.
/// MESSAGE-INTEGRITY (SHA-1 or SHA-256) may occur in public-server responses:
/// it is structurally checked but **never verified or trusted**. Such responses
/// require a valid final FINGERPRINT and a mapped address preceding integrity.
/// Credentials/authentication negotiation are unsupported. CRC is not security.
/// Unknown optional attributes before integrity are ignored. This strict subset
/// rejects semantic attributes after integrity instead of exposing their values.
pub fn parse_response(
    expected_transaction: TransactionId,
    expected_source: SocketAddr,
    actual_source: SocketAddr,
    bytes: &[u8],
) -> Result<MappedAddress, Error> {
    if expected_source != actual_source {
        return Err(Error::Source);
    }
    let (kind, transaction) = parse_header(bytes)?;
    if transaction != expected_transaction {
        return Err(Error::Transaction);
    }
    if kind == MessageKind::Request {
        return Err(Error::MessageKind);
    }
    let parsed = parse(bytes)?;
    if !parsed.unknown.is_empty() {
        return Err(Error::UnknownAttribute);
    }
    if parsed.integrity && !parsed.fingerprint {
        return Err(Error::Authentication);
    }
    match kind {
        MessageKind::SuccessResponse if parsed.error.is_none() => parsed
            .mapped
            .map(MappedAddress)
            .ok_or(Error::MissingMappedAddress),
        MessageKind::ErrorResponse if parsed.mapped.is_none() => Err(Error::ErrorResponse {
            code: parsed.error.ok_or(Error::Attribute)?,
        }),
        _ => Err(Error::Attribute),
    }
}

fn parse_header(bytes: &[u8]) -> Result<(MessageKind, TransactionId), Error> {
    if !(HEADER..=MAX_DATAGRAM_SIZE).contains(&bytes.len()) {
        return Err(Error::Size);
    }
    let kind = classify(bytes).ok_or(Error::Header)?;
    let length = usize::from(u16::from_be_bytes([bytes[2], bytes[3]]));
    if !length.is_multiple_of(4) || length + HEADER != bytes.len() {
        return Err(Error::Size);
    }
    let mut transaction = [0; 12];
    transaction.copy_from_slice(&bytes[8..20]);
    Ok((kind, TransactionId(transaction)))
}

fn parse(bytes: &[u8]) -> Result<Parsed, Error> {
    let (kind, transaction) = parse_header(bytes)?;
    let mut parsed = Parsed {
        kind,
        transaction,
        mapped: None,
        error: None,
        fingerprint: false,
        integrity: false,
        unknown: Vec::new(),
        response_attribute: false,
    };
    let mut seen_mapped = false;
    let mut seen_unknown = false;
    let mut sha1 = false;
    let mut sha256 = false;
    let mut offset = HEADER;
    while offset < bytes.len() {
        let attr_header = bytes.get(offset..offset + 4).ok_or(Error::Attribute)?;
        let ty = u16::from_be_bytes([attr_header[0], attr_header[1]]);
        let length = usize::from(u16::from_be_bytes([attr_header[2], attr_header[3]]));
        let start = offset.checked_add(4).ok_or(Error::Attribute)?;
        let end = start.checked_add(length).ok_or(Error::Attribute)?;
        let next = end
            .checked_add((4 - length % 4) % 4)
            .ok_or(Error::Attribute)?;
        let value = bytes.get(start..end).ok_or(Error::Attribute)?;
        if next > bytes.len() {
            return Err(Error::Attribute);
        }
        // RFC 8489 integrity ends the semantic body. Only integrity/fingerprint
        // may follow here; never return a mapped address outside that body.
        if parsed.integrity && !matches!(ty, 0x0008 | 0x001c | FINGERPRINT) {
            return Err(Error::Authentication);
        }
        match ty {
            XOR_MAPPED => {
                if parsed.mapped.is_some() {
                    return Err(Error::Duplicate);
                }
                parsed.mapped = Some(mapped(value, transaction, true)?);
                parsed.response_attribute = true;
            }
            0x0001 => {
                if seen_mapped {
                    return Err(Error::Duplicate);
                }
                seen_mapped = true;
                mapped(value, transaction, false)?;
                parsed.response_attribute = true;
            }
            ERROR_CODE => {
                if parsed.error.is_some() {
                    return Err(Error::Duplicate);
                }
                if value.len() < 4
                    || value.len() > 767
                    || value[0..2] != [0, 0]
                    || !(3..=6).contains(&value[2])
                    || value[3] > 99
                    || std::str::from_utf8(&value[4..]).is_err()
                {
                    return Err(Error::Attribute);
                }
                parsed.error = Some(u16::from(value[2]) * 100 + u16::from(value[3]));
                parsed.response_attribute = true;
            }
            UNKNOWN_ATTRIBUTES => {
                if seen_unknown {
                    return Err(Error::Duplicate);
                }
                if value.is_empty() || !value.len().is_multiple_of(2) {
                    return Err(Error::Attribute);
                }
                seen_unknown = true;
                parsed.response_attribute = true;
            }
            FINGERPRINT => {
                if parsed.fingerprint {
                    return Err(Error::Duplicate);
                }
                if value.len() != 4 || next != bytes.len() {
                    return Err(Error::Fingerprint);
                }
                let expected = (crc32fast::hash(&bytes[..offset]) ^ 0x5354_554e).to_be_bytes();
                if value != expected {
                    return Err(Error::Fingerprint);
                }
                parsed.fingerprint = true;
            }
            0x0008 | 0x001c => {
                if kind == MessageKind::Request {
                    return Err(Error::Authentication);
                }
                let seen = if ty == 0x0008 { &mut sha1 } else { &mut sha256 };
                if *seen {
                    return Err(Error::Duplicate);
                }
                *seen = true;
                if (ty == 0x0008 && value.len() != 20)
                    || (ty == 0x001c
                        && (!(16..=32).contains(&value.len()) || !value.len().is_multiple_of(4)))
                    || (ty == 0x0008 && sha256)
                {
                    return Err(Error::Authentication);
                }
                parsed.integrity = true;
            }
            // USERNAME, REALM, NONCE, PASSWORD-ALGORITHM(S), USERHASH.
            0x0006 | 0x0014 | 0x0015 | 0x001d | 0x001e | 0x8002 => {
                return Err(Error::Authentication);
            }
            // RFC 5780 PADDING is safe to discard, never echo.
            0x0026 => {}
            // SOFTWARE (and all other optional extensions) are
            // bounded by the packet and ignored, never echoed.
            ty if ty >= 0x8000 => {}
            ty => {
                if !parsed.unknown.contains(&ty) {
                    if parsed.unknown.len() == MAX_UNKNOWN {
                        return Err(Error::TooManyUnknownAttributes);
                    }
                    parsed.unknown.push(ty);
                }
            }
        }
        offset = next;
    }
    Ok(parsed)
}

fn mapped(value: &[u8], transaction: TransactionId, xor: bool) -> Result<SocketAddr, Error> {
    if value.len() < 4 || value[0] != 0 {
        return Err(Error::Attribute);
    }
    let port = u16::from_be_bytes([value[2], value[3]]) ^ if xor { 0x2112 } else { 0 };
    if port == 0 {
        return Err(Error::Attribute);
    }
    let mask = if xor {
        address_mask(transaction)
    } else {
        [0; 16]
    };
    let ip = match (value[1], value.len()) {
        (1, 8) => {
            let mut octets = [0; 4];
            for (i, byte) in octets.iter_mut().enumerate() {
                *byte = value[4 + i] ^ mask[i];
            }
            IpAddr::V4(Ipv4Addr::from(octets))
        }
        (2, 20) => {
            let mut octets = [0; 16];
            for (i, byte) in octets.iter_mut().enumerate() {
                *byte = value[4 + i] ^ mask[i];
            }
            IpAddr::V6(Ipv6Addr::from(octets))
        }
        _ => return Err(Error::Attribute),
    };
    Ok(SocketAddr::new(ip, port))
}

fn address_mask(transaction: TransactionId) -> [u8; 16] {
    let mut mask = [0; 16];
    mask[..4].copy_from_slice(&COOKIE);
    mask[4..].copy_from_slice(transaction.as_bytes());
    mask
}

fn header(kind: MessageKind, transaction: TransactionId) -> Vec<u8> {
    let ty: u16 = match kind {
        MessageKind::Request => 0x0001,
        MessageKind::SuccessResponse => 0x0101,
        MessageKind::ErrorResponse => 0x0111,
    };
    let mut bytes = Vec::with_capacity(128);
    bytes.extend_from_slice(&ty.to_be_bytes());
    bytes.extend_from_slice(&[0, 0]);
    bytes.extend_from_slice(&COOKIE);
    bytes.extend_from_slice(transaction.as_bytes());
    bytes
}

fn attribute(bytes: &mut Vec<u8>, ty: u16, value: &[u8]) {
    bytes.extend_from_slice(&ty.to_be_bytes());
    // All callers use statically bounded values (at most 32 bytes).
    bytes.extend_from_slice(&(value.len() as u16).to_be_bytes());
    bytes.extend_from_slice(value);
    bytes.resize(bytes.len().next_multiple_of(4), 0);
}

fn finish(bytes: &mut Vec<u8>, fingerprint: Fingerprint) {
    let length = bytes.len() - HEADER
        + if fingerprint == Fingerprint::Include {
            8
        } else {
            0
        };
    bytes[2..4].copy_from_slice(&(length as u16).to_be_bytes());
    if fingerprint == Fingerprint::Include {
        let crc = crc32fast::hash(bytes) ^ 0x5354_554e;
        attribute(bytes, FINGERPRINT, &crc.to_be_bytes());
    }
}
