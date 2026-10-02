//! Strict public DNS diagnostics DTOs and a bounded real wire query client.
use std::{
    net::{IpAddr, SocketAddr},
    path::Path,
    time::Duration,
};

use hickory_proto::{
    op::{Message, MessageType, OpCode, Query, ResponseCode},
    rr::{DNSClass, Name, RData, RecordType},
    serialize::binary::{BinDecodable, BinDecoder},
};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
    time::{Instant, timeout},
};

use super::{LiveResponse, MAX_RESPONSE, Operation, ProbeError, Request, SCHEMA_VERSION, exchange};

const QUERY_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_WIRE: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Active,
    Disabled,
    Expired,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub schema_version: u32,
    pub state: State,
    pub generation: u64,
    pub revision_floor: u32,
    pub active_revision: Option<u32>,
    pub zone: Option<String>,
    pub ttl: Option<u32>,
    pub lease_remaining_ms: u64,
    pub listener: Option<SocketAddr>,
    pub listener_bound: bool,
    pub network_id: Option<String>,
    pub node_id: Option<String>,
    pub evidence: StatusEvidence,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusEvidence {
    CurrentDnsGuard,
}

impl Status {
    pub(crate) fn unavailable() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            state: State::Unavailable,
            generation: 0,
            revision_floor: 0,
            active_revision: None,
            zone: None,
            ttl: None,
            lease_remaining_ms: 0,
            listener: None,
            listener_bound: false,
            network_id: None,
            node_id: None,
            evidence: StatusEvidence::CurrentDnsGuard,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryType {
    A,
    AAAA,
    PTR,
}

impl QueryType {
    fn record_type(self) -> RecordType {
        match self {
            Self::A => RecordType::A,
            Self::AAAA => RecordType::AAAA,
            Self::PTR => RecordType::PTR,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryState {
    Positive,
    Nxdomain,
    Nodata,
    Refused,
    Servfail,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    pub name: String,
    pub query_type: QueryType,
    pub ttl: u32,
    pub data: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryEvidence {
    LoopbackUdpRoundTrip,
    LoopbackTcpRoundTrip,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub schema_version: u32,
    /// Separate point-in-time metadata; a successful status is not query evidence.
    pub metadata: Status,
    pub name: String,
    pub query_type: QueryType,
    pub endpoint: SocketAddr,
    pub state: QueryState,
    pub rcode: u16,
    pub authoritative: bool,
    pub answers: Vec<Answer>,
    pub elapsed_ms: u64,
    pub evidence: QueryEvidence,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum Error {
    #[error("DNS daemon unavailable; no cached DNS fallback")]
    Unavailable,
    #[error("DNS query timed out")]
    Timeout,
    #[error("invalid DNS name, IP or query type")]
    InvalidInput,
    #[error("DNS response failed transaction, question, type, size or framing validation")]
    InvalidResponse,
    #[error("DNS listener returned REFUSED")]
    Refused,
    #[error("DNS listener returned SERVFAIL")]
    Servfail,
    #[error("DNS local IPC failed: {0}")]
    Ipc(ProbeError),
}

fn ipc_error(error: ProbeError) -> Error {
    match error {
        ProbeError::Unavailable => Error::Unavailable,
        ProbeError::Timeout => Error::Timeout,
        ProbeError::InvalidRequest => Error::Ipc(ProbeError::UnsupportedVersion),
        error => Error::Ipc(error),
    }
}

pub async fn status(state: &Path) -> Result<Status, Error> {
    match exchange::<LiveResponse>(
        state,
        &Request {
            schema_version: SCHEMA_VERSION,
            operation: Operation::DnsStatus {},
        },
        MAX_RESPONSE,
    )
    .await
    .map_err(ipc_error)?
    {
        LiveResponse::DnsStatus(value) if value.schema_version == SCHEMA_VERSION => Ok(value),
        LiveResponse::Error(error) => Err(ipc_error(error)),
        _ => Err(Error::Ipc(ProbeError::UnsupportedVersion)),
    }
}

pub fn question(input: &str, kind: Option<QueryType>) -> Result<(Name, QueryType), Error> {
    if let Ok(ip) = input.parse::<IpAddr>() {
        if kind.is_some_and(|kind| kind != QueryType::PTR) {
            return Err(Error::InvalidInput);
        }
        return Ok((Name::from(ip), QueryType::PTR));
    }
    if input.is_empty()
        || input.len() > 253
        || !input.is_ascii()
        || input
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || c == '-' || c == '.'))
    {
        return Err(Error::InvalidInput);
    }
    let mut name = Name::from_ascii(input)
        .map_err(|_| Error::InvalidInput)?
        .to_lowercase();
    name.set_fqdn(true);
    if name.is_root() {
        return Err(Error::InvalidInput);
    }
    Ok((name, kind.unwrap_or(QueryType::A)))
}

pub async fn query(
    state: &Path,
    input: &str,
    kind: Option<QueryType>,
    tcp: bool,
) -> Result<Observation, Error> {
    let (name, kind) = question(input, kind)?;
    let metadata = status(state).await?;
    query_listener(metadata, name, kind, tcp).await
}

pub(crate) async fn query_listener(
    metadata: Status,
    name: Name,
    kind: QueryType,
    tcp: bool,
) -> Result<Observation, Error> {
    let endpoint = metadata
        .listener
        .filter(|address| address.ip().is_loopback() && address.port() >= 1024)
        .filter(|_| metadata.listener_bound)
        .ok_or(Error::Unavailable)?;
    let mut random = [0; 2];
    getrandom::fill(&mut random).map_err(|_| Error::Unavailable)?;
    let mut request = Message::new(
        u16::from_ne_bytes(random),
        MessageType::Query,
        OpCode::Query,
    );
    request.add_query(Query::query(name.clone(), kind.record_type()));
    let bytes = request.to_vec().map_err(|_| Error::InvalidInput)?;
    let start = Instant::now();
    let response = timeout(QUERY_TIMEOUT, async {
        if tcp {
            let mut socket = TcpStream::connect(endpoint)
                .await
                .map_err(|_| Error::Unavailable)?;
            if socket.peer_addr().map_err(|_| Error::Unavailable)? != endpoint {
                return Err(Error::InvalidResponse);
            }
            socket
                .write_u16(bytes.len() as u16)
                .await
                .map_err(|_| Error::Unavailable)?;
            socket
                .write_all(&bytes)
                .await
                .map_err(|_| Error::Unavailable)?;
            let length = socket
                .read_u16()
                .await
                .map_err(|_| Error::InvalidResponse)? as usize;
            if !(12..=MAX_WIRE).contains(&length) {
                return Err(Error::InvalidResponse);
            }
            let mut reply = vec![0; length];
            socket
                .read_exact(&mut reply)
                .await
                .map_err(|_| Error::InvalidResponse)?;
            // The daemon's protocol is exactly one transaction then EOF.
            let mut extra = [0];
            if socket
                .read(&mut extra)
                .await
                .map_err(|_| Error::InvalidResponse)?
                != 0
            {
                return Err(Error::InvalidResponse);
            }
            Ok(reply)
        } else {
            let bind = if endpoint.is_ipv4() {
                "127.0.0.1:0"
            } else {
                "[::1]:0"
            };
            let socket = UdpSocket::bind(bind)
                .await
                .map_err(|_| Error::Unavailable)?;
            socket
                .send_to(&bytes, endpoint)
                .await
                .map_err(|_| Error::Unavailable)?;
            let mut reply = vec![0; 513]; // No EDNS advertised: RFC UDP limit is 512.
            let (size, from) = socket
                .recv_from(&mut reply)
                .await
                .map_err(|_| Error::Unavailable)?;
            if from != endpoint || !(12..=512).contains(&size) {
                return Err(Error::InvalidResponse);
            }
            reply.truncate(size);
            Ok(reply)
        }
    })
    .await
    .map_err(|_| Error::Timeout)??;
    let reply = validate_response(&request, &response)?;
    let state = match reply.metadata.response_code {
        ResponseCode::NoError if reply.answers.is_empty() => QueryState::Nodata,
        ResponseCode::NoError => QueryState::Positive,
        ResponseCode::NXDomain if reply.answers.is_empty() => QueryState::Nxdomain,
        ResponseCode::Refused if reply.answers.is_empty() => QueryState::Refused,
        ResponseCode::ServFail if reply.answers.is_empty() => QueryState::Servfail,
        _ => return Err(Error::InvalidResponse),
    };
    let mut answers = Vec::new();
    for record in &reply.answers {
        let data = match &record.data {
            RData::A(value) if kind == QueryType::A => value.0.to_string(),
            RData::AAAA(value) if kind == QueryType::AAAA => value.0.to_string(),
            RData::PTR(value) if kind == QueryType::PTR => value.0.to_string(),
            _ => return Err(Error::InvalidResponse),
        };
        if record.name != name || record.dns_class != DNSClass::IN {
            return Err(Error::InvalidResponse);
        }
        answers.push(Answer {
            name: record.name.to_string(),
            query_type: kind,
            ttl: record.ttl,
            data,
        });
    }
    Ok(Observation {
        schema_version: SCHEMA_VERSION,
        metadata,
        name: name.to_string(),
        query_type: kind,
        endpoint,
        state,
        rcode: reply.metadata.response_code.into(),
        authoritative: reply.metadata.authoritative,
        answers,
        elapsed_ms: start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        evidence: if tcp {
            QueryEvidence::LoopbackTcpRoundTrip
        } else {
            QueryEvidence::LoopbackUdpRoundTrip
        },
    })
}

fn validate_response(request: &Message, bytes: &[u8]) -> Result<Message, Error> {
    let mut decoder = BinDecoder::new(bytes);
    let reply = Message::read(&mut decoder).map_err(|_| Error::InvalidResponse)?;
    if decoder.index() != bytes.len()
        || reply.metadata.id != request.metadata.id
        || reply.metadata.message_type != MessageType::Response
        || reply.metadata.op_code != OpCode::Query
        || reply.metadata.truncation
        || reply.queries != request.queries
        || reply.queries.len() != 1
        || !reply.authorities.is_empty()
        || !reply.additionals.is_empty()
        || reply.signature.is_some()
        || reply.edns.is_some()
    {
        return Err(Error::InvalidResponse);
    }
    Ok(reply)
}

#[cfg(test)]
mod tests;
