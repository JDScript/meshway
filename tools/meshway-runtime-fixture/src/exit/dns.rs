//! Isolated DNS authorities of the exit topology. External is E's only OS
//! upstream; Resolver is A's own OS upstream and its `useWithExit` split
//! resolver. Both log every question with its real peer; no Meshway owner,
//! permit or resolver selection exists here.
use super::*;
use hickory_proto::{
    op::{Message, ResponseCode},
    rr::{RData, Record, RecordType, rdata},
};
use std::net::{Ipv4Addr, Ipv6Addr};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UdpSocket},
    task::JoinSet,
};

/// Ordinary names A resolves through the exit.
pub const EXIT_ZONE: &str = "exit-dns.test.";
/// A's `useWithExit` split suffix.
pub const SPLIT_ZONE: &str = "split.test.";
const EXIT_A: Ipv4Addr = Ipv4Addr::new(198, 51, 100, 10);
const EXIT_AAAA: Ipv6Addr = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x10);
/// More A records than one 1232-byte UDP response holds.
const BIG_RECORDS: u8 = 90;
const SPLIT_A: Ipv4Addr = Ipv4Addr::new(198, 51, 100, 53);
/// Resolver's answer to anything outside its zone: a source fallback would
/// return this instead of the exit's answer.
const FALLBACK_A: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 66);
const MAX_LOG: usize = 2048;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Question {
    name: String,
    record_type: String,
    protocol: &'static str,
    peer: std::net::SocketAddr,
    truncated: bool,
}

fn answer(role: Role, bytes: &[u8], limit: Option<usize>) -> Result<(Message, Vec<u8>)> {
    let query = Message::from_vec(bytes)?;
    let question = query.queries.first().ok_or("DNS query without question")?;
    let name = question.name().to_ascii().to_ascii_lowercase();
    let kind = question.query_type();
    let mut response = Message::new(
        query.metadata.id,
        hickory_proto::op::MessageType::Response,
        hickory_proto::op::OpCode::Query,
    );
    response.metadata.recursion_desired = query.metadata.recursion_desired;
    response.metadata.recursion_available = true;
    response.metadata.authoritative = true;
    response.add_query(question.clone());
    let owner = question.name().clone();
    let records: Vec<RData> = match (role, kind) {
        (Role::External, _) if !name.ends_with(EXIT_ZONE) => {
            response.metadata.response_code = ResponseCode::Refused;
            Vec::new()
        }
        (Role::External, RecordType::A) if name == format!("big.{EXIT_ZONE}") => (1..=BIG_RECORDS)
            .map(|i| RData::A(rdata::A(Ipv4Addr::new(198, 51, 100, i))))
            .collect(),
        (Role::External, RecordType::A) => vec![RData::A(rdata::A(EXIT_A))],
        (Role::External, RecordType::AAAA) if !name.starts_with("big.") => {
            vec![RData::AAAA(rdata::AAAA(EXIT_AAAA))]
        }
        (Role::Resolver, RecordType::A) if name.ends_with(SPLIT_ZONE) => {
            vec![RData::A(rdata::A(SPLIT_A))]
        }
        (Role::Resolver, RecordType::A) if !name.ends_with(SPLIT_ZONE) => {
            vec![RData::A(rdata::A(FALLBACK_A))]
        }
        _ => Vec::new(),
    };
    for data in records {
        response.add_answer(Record::from_rdata(owner.clone(), 0, data));
    }
    let mut encoded = response.to_vec()?;
    if let Some(limit) = limit {
        let limit = query
            .edns
            .as_ref()
            .map_or(512, |edns| usize::from(edns.max_payload()).max(512))
            .min(limit);
        if encoded.len() > limit {
            response.answers.clear();
            response.metadata.truncation = true;
            encoded = response.to_vec()?;
        }
    }
    Ok((response, encoded))
}

fn record(
    log: &std::sync::Mutex<Vec<Question>>,
    bytes: &[u8],
    response: &Message,
    protocol: &'static str,
    peer: std::net::SocketAddr,
) -> Result<()> {
    let question = Message::from_vec(bytes)
        .ok()
        .and_then(|m| m.queries.first().cloned());
    let mut log = log.lock().map_err(|_| "DNS log poisoned")?;
    if log.len() >= MAX_LOG {
        return Err("exit DNS log bound exceeded".into());
    }
    log.push(Question {
        name: question
            .as_ref()
            .map(|q| q.name().to_ascii().to_ascii_lowercase())
            .unwrap_or_default(),
        record_type: question
            .map(|q| q.query_type().to_string())
            .unwrap_or_default(),
        protocol,
        peer,
        truncated: response.metadata.truncation,
    });
    Ok(())
}

pub(super) async fn run(cli: &Cli, role: Role) -> Result<()> {
    let udp = UdpSocket::bind("0.0.0.0:53").await?;
    let tcp = TcpListener::bind("0.0.0.0:53").await?;
    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Question>::new()));
    let mut connections = JoinSet::new();
    let mut buffer = [0; 4096];
    let mut written = 0;
    let result = async {
        loop {
            if tokio::fs::try_exists(cli.signals.join("exit-stop")).await? {
                break;
            }
            tokio::select! {
                received = udp.recv_from(&mut buffer) => {
                    let (size, peer) = received?;
                    let Ok((response, bytes)) = answer(role, &buffer[..size], Some(4096)) else { continue };
                    record(&log, &buffer[..size], &response, "udp", peer)?;
                    let _ = udp.send_to(&bytes, peer).await;
                }
                accepted = tcp.accept(), if connections.len() < 32 => {
                    let (mut stream, peer) = accepted?;
                    let log = log.clone();
                    connections.spawn(async move {
                        let exchange = async {
                            let size = usize::from(stream.read_u16().await?);
                            if !(12..=4096).contains(&size) {
                                return Err::<_, crate::Error>("TCP DNS length out of bounds".into());
                            }
                            let mut bytes = vec![0; size];
                            stream.read_exact(&mut bytes).await?;
                            let (response, encoded) = answer(role, &bytes, None)?;
                            record(&log, &bytes, &response, "tcp", peer)?;
                            stream.write_u16(encoded.len().try_into()?).await?;
                            stream.write_all(&encoded).await?;
                            Ok(())
                        };
                        // A peer that stops mid-exchange only ends its connection.
                        let _ = tokio::time::timeout(IO, exchange).await;
                        Ok::<(), crate::Error>(())
                    });
                }
                Some(done) = connections.join_next(), if !connections.is_empty() => { done??; }
                _ = tokio::time::sleep(Duration::from_millis(100)) => {}
            }
            let current = log.lock().map_err(|_| "DNS log poisoned")?.len();
            if current != written {
                let snapshot = serde_json::to_vec_pretty(
                    &*log.lock().map_err(|_| "DNS log poisoned")?,
                )?;
                let path = cli.evidence.join("dns-queries.json");
                tokio::fs::write(path.with_extension("tmp"), snapshot).await?;
                tokio::fs::rename(path.with_extension("tmp"), path).await?;
                written = current;
            }
        }
        Ok::<(), crate::Error>(())
    }
    .await;
    connections.abort_all();
    while let Some(done) = connections.join_next().await {
        if let Err(error) = done
            && !error.is_cancelled()
        {
            return Err(error.into());
        }
    }
    result
}
