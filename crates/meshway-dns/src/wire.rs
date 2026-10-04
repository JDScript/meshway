use bytes::Bytes;
use hickory_proto::{
    op::{Edns, Header, Message, MessageType, Metadata, OpCode, ResponseCode},
    rr::{DNSClass, RecordType},
    serialize::binary::{BinDecodable, BinDecoder, BinEncodable, BinEncoder},
};

pub(crate) const MIN_MESSAGE_SIZE: usize = 12;
pub(crate) const MAX_MESSAGE_SIZE: usize = u16::MAX as usize;

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("DNS message must contain 12..=65535 bytes")]
    MessageLength,
    #[error("invalid DNS wire message: {0}")]
    Decode(#[from] hickory_proto::serialize::binary::DecodeError),
    #[error("bytes remain after the DNS message")]
    TrailingBytes,
    #[error("expected one complete QUERY question, with only an optional OPT record")]
    QueryShape,
    #[error("only IN-class questions are supported")]
    UnsupportedClass,
    #[error("zone transfers and meta-record questions are unsupported")]
    UnsupportedQuestionType,
    #[error("unsupported EDNS version: {0}")]
    EdnsVersion(u8),
    #[error("UDP payload limit must be at least 512 bytes")]
    PayloadLimit,
    #[error("response must match the question and omit EDNS and signatures")]
    ResponseShape,
    #[error("extended response codes require an EDNS query")]
    ExtendedResponseCode,
    #[error("DNS response exceeds the TCP message size")]
    ResponseTooLarge,
    #[error("DNS encoding failed: {0}")]
    Encode(#[from] hickory_proto::ProtoError),
}

/// A validated ordinary DNS query. The original question's label case is kept.
/// Construction is through [`decode_query`], including for TCP-framed payloads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    message: Message,
}

impl Query {
    /// Encode this validated query for a bounded upstream exchange. TCP framing
    /// belongs to the exchange adapter. The adapter may remap IDs only if it
    /// verifies its own transaction and restores this ID before returning bytes.
    pub fn encode_upstream(&self, udp_payload_limit: u16) -> Result<Bytes, WireError> {
        if udp_payload_limit < 512 {
            return Err(WireError::PayloadLimit);
        }
        let mut message = self.message.clone();
        if let Some(edns) = message.edns.as_mut() {
            edns.set_max_payload(edns.max_payload().max(512).min(udp_payload_limit));
        }
        Ok(Bytes::from(message.to_vec()?))
    }

    pub fn id(&self) -> u16 {
        self.message.metadata.id
    }

    pub fn question(&self) -> &hickory_proto::op::Query {
        &self.message.queries[0]
    }

    pub fn edns(&self) -> Option<&Edns> {
        self.message.edns.as_ref()
    }

    /// An empty NOERROR response preserving the ID, question, RD and CD bits.
    /// AA, RA and AD are clear. No authority, recursion or DNSSEC is asserted.
    /// [`encode_response`] adds the negotiated EDNS envelope.
    pub fn response(&self) -> Message {
        let mut response = Message::response(self.id(), OpCode::Query);
        response.metadata = Metadata::response_from_request(&self.message.metadata);
        response.add_query(self.question().clone());
        response
    }
}

/// Decode one complete UDP datagram or one already-delimited TCP payload.
///
/// Accepts one IN question, standard QUERY, no answer/authority records and at
/// most one additional record (OPT, EDNS version 0). Transfer, signature and
/// meta-record questions are unsupported. Unknown EDNS options are retained for
/// inspection; local response encoding does not echo these request options.
pub fn decode_query(bytes: &[u8]) -> Result<Query, WireError> {
    if !(MIN_MESSAGE_SIZE..=MAX_MESSAGE_SIZE).contains(&bytes.len()) {
        return Err(WireError::MessageLength);
    }
    // Check counts before the library allocates vectors from untrusted counts.
    let header = Header::read(&mut BinDecoder::new(bytes))?;
    if header.metadata.message_type != MessageType::Query
        || header.metadata.op_code != OpCode::Query
        || header.metadata.authoritative
        || header.metadata.truncation
        || header.metadata.recursion_available
        || header.metadata.response_code != ResponseCode::NoError
        || bytes[3] & 0x40 != 0 // reserved header bit, not exposed by Metadata
        || header.counts.queries != 1
        || header.counts.answers != 0
        || header.counts.authorities != 0
        || header.counts.additionals > 1
    {
        return Err(WireError::QueryShape);
    }
    let mut decoder = BinDecoder::new(bytes);
    let message = Message::read(&mut decoder)?;
    if !decoder.is_empty() {
        return Err(WireError::TrailingBytes);
    }
    if !message.additionals.is_empty()
        || message.signature.is_some()
        || message.metadata.response_code != ResponseCode::NoError
    {
        return Err(WireError::QueryShape);
    }
    let question = &message.queries[0];
    if question.query_class() != DNSClass::IN {
        return Err(WireError::UnsupportedClass);
    }
    if matches!(
        question.query_type(),
        RecordType::AXFR | RecordType::IXFR | RecordType::OPT | RecordType::TSIG | RecordType::ZERO
    ) {
        return Err(WireError::UnsupportedQuestionType);
    }
    if let Some(edns) = &message.edns
        && edns.version() != 0
    {
        return Err(WireError::EdnsVersion(edns.version()));
    }
    if message
        .edns
        .as_ref()
        .is_some_and(|edns| edns.flags().z != 0)
    {
        return Err(WireError::QueryShape);
    }
    Ok(Query { message })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseTransport {
    Udp,
    Tcp,
}

/// Validated upstream data, not a trusted domain association or an I/O grant.
/// Source endpoint validation is the exchange adapter's responsibility.
#[derive(Debug)]
pub struct TrustedUpstreamResponse {
    pub(crate) message: Message,
    pub(crate) query: Query,
}

impl TrustedUpstreamResponse {
    /// Pure serialization. A live service must retain the owner's Answer guard
    /// through actual I/O, even after successful encoding.
    pub fn encode(
        &self,
        query: &Query,
        transport: ResponseTransport,
        udp_payload_limit: u16,
    ) -> Result<Bytes, WireError> {
        if query != &self.query {
            return Err(WireError::ResponseShape);
        }
        encode_response(query, &self.message, transport, udp_payload_limit)
    }

    pub fn decode(
        query: &Query,
        bytes: &[u8],
        transport: ResponseTransport,
        udp_payload_limit: u16,
    ) -> Result<Self, WireError> {
        if udp_payload_limit < 512 {
            return Err(WireError::PayloadLimit);
        }
        let limit = match transport {
            ResponseTransport::Tcp => MAX_MESSAGE_SIZE,
            ResponseTransport::Udp => usize::from(
                query
                    .edns()
                    .map_or(512, |e| e.max_payload().max(512))
                    .min(udp_payload_limit),
            ),
        };
        if !(MIN_MESSAGE_SIZE..=limit).contains(&bytes.len()) {
            return Err(WireError::MessageLength);
        }
        let header = Header::read(&mut BinDecoder::new(bytes))?;
        let records = usize::from(header.counts.answers)
            + usize::from(header.counts.authorities)
            + usize::from(header.counts.additionals);
        if header.metadata.message_type != MessageType::Response
            || header.metadata.op_code != OpCode::Query
            || header.metadata.id != query.id()
            || header.counts.queries != 1
            || bytes[3] & 0x40 != 0
            || records > 4096
            || records > bytes.len()
        {
            return Err(WireError::ResponseShape);
        }
        let mut decoder = BinDecoder::new(bytes);
        let mut message = Message::read(&mut decoder)?;
        if !decoder.is_empty() {
            return Err(WireError::TrailingBytes);
        }
        if message.queries.as_slice() != std::slice::from_ref(query.question())
            || message.signature.is_some()
            || message
                .answers
                .iter()
                .chain(&message.authorities)
                .chain(&message.additionals)
                .any(|r| {
                    matches!(
                        r.record_type(),
                        RecordType::OPT
                            | RecordType::TSIG
                            | RecordType::AXFR
                            | RecordType::IXFR
                            | RecordType::ZERO
                    )
                })
        {
            return Err(WireError::ResponseShape);
        }
        if let Some(edns) = &message.edns {
            if query.edns().is_none() || edns.version() != 0 || edns.flags().z != 0 {
                return Err(WireError::ResponseShape);
            }
        } else if message.metadata.response_code.high() != 0 {
            return Err(WireError::ResponseShape);
        }
        // Rebuild the downstream EDNS envelope through the one canonical encoder.
        // We do not claim DNSSEC validation, nor reflect upstream EDNS options.
        message.edns = None;
        message.metadata.authentic_data = false;
        Ok(Self {
            message,
            query: query.clone(),
        })
    }
}

/// Encode a canonical response, without a TCP length prefix. Upstream data must
/// first pass through `TrustedUpstreamResponse::decode` to normalize its envelope.
///
/// `udp_payload_limit` is the caller's UDP size ceiling (512..=65535), also used
/// as the advertised OPT payload size for TCP responses. UDP output is limited
/// by both that ceiling and the query's advertised payload (512 without EDNS).
/// If records do not fit, all record sections are omitted and TC is set; the
/// original question and an OPT for EDNS clients remain. TCP never silently
/// drops records. Feed TCP output to [`crate::TcpCodec`] for framing.
///
/// The question must match; its original case and client ID replace the supplied
/// response's question and ID. This is not a transparent upstream-response
/// adapter: EDNS/signatures must be absent, and a fresh EDNS envelope is built.
/// Request options are not reflected. The DO bit is copied without asserting AD.
pub fn encode_response(
    query: &Query,
    response: &Message,
    transport: ResponseTransport,
    udp_payload_limit: u16,
) -> Result<Bytes, WireError> {
    if udp_payload_limit < 512 {
        return Err(WireError::PayloadLimit);
    }
    if response.metadata.message_type != MessageType::Response
        || response.metadata.op_code != OpCode::Query
        || response.queries.as_slice() != std::slice::from_ref(query.question())
        || response.edns.is_some()
        || response.signature.is_some()
        || response
            .answers
            .iter()
            .chain(&response.authorities)
            .chain(&response.additionals)
            .any(|record| matches!(record.record_type(), RecordType::OPT | RecordType::TSIG))
    {
        return Err(WireError::ResponseShape);
    }
    if response.metadata.response_code.high() != 0 && query.edns().is_none() {
        return Err(WireError::ExtendedResponseCode);
    }
    let mut response = response.clone();
    response.metadata.id = query.id();
    response.metadata.recursion_desired = query.message.metadata.recursion_desired;
    response.metadata.checking_disabled = query.message.metadata.checking_disabled;
    response.queries[0] = query.question().clone();
    if let Some(request_edns) = query.edns() {
        let mut edns = Edns::new();
        edns.set_max_payload(udp_payload_limit)
            .set_dnssec_ok(request_edns.flags().dnssec_ok);
        response.edns = Some(edns);
    }
    let limit = match transport {
        ResponseTransport::Udp => query
            .edns()
            .map_or(512, |edns| edns.max_payload().max(512))
            .min(udp_payload_limit),
        ResponseTransport::Tcp => u16::MAX,
    };
    let (bytes, omitted) = emit_limited(&response, limit)?;
    if !omitted {
        return Ok(bytes);
    }
    if transport == ResponseTransport::Tcp {
        return Err(WireError::ResponseTooLarge);
    }
    // Do not return an incomplete RRset or lose the OPT due to record ordering.
    let (bytes, omitted) = emit_limited(&response.truncate(), limit)?;
    debug_assert!(!omitted); // one question + empty OPT always fits 512 bytes
    Ok(bytes)
}

fn emit_limited(message: &Message, limit: u16) -> Result<(Bytes, bool), WireError> {
    let mut bytes = Vec::new();
    let mut encoder = BinEncoder::new(&mut bytes);
    encoder.set_max_size(limit);
    message.emit(&mut encoder)?;
    let header = Header::read(&mut BinDecoder::new(&bytes))?;
    let omitted = usize::from(header.counts.answers) != message.answers.len()
        || usize::from(header.counts.authorities) != message.authorities.len()
        || usize::from(header.counts.additionals)
            != message.additionals.len() + usize::from(message.edns.is_some());
    Ok((Bytes::from(bytes), omitted))
}
