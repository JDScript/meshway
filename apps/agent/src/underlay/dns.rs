//! Bootstrap only. No getaddrinfo, daemon, unmarked or business-DNS fallback.
use std::{
    collections::HashMap,
    io,
    net::{IpAddr, SocketAddr},
    sync::Mutex,
    time::{Duration, Instant},
};

use hickory_proto::{
    op::{Message, MessageType, OpCode, Query, ResponseCode},
    rr::{DNSClass, Name, RData, RecordType},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{Purpose, SocketFactory, closed, invalid};

/// Ordered plain DNS servers from a trusted system/VPN routing snapshot. Split
/// DNS, local resolver daemons, proxy and encrypted upstreams need an adapter;
/// they must not be converted to an unmarked system lookup.
pub(crate) struct SystemDnsSnapshot {
    servers: Vec<SocketAddr>,
}

impl SystemDnsSnapshot {
    pub(crate) fn plain_servers(servers: Vec<SocketAddr>) -> io::Result<Self> {
        if servers.is_empty()
            || servers.iter().any(|s| {
                s.ip().is_loopback()
                    || s.ip().is_unspecified()
                    || s.ip().is_multicast()
                    || s.port() == 0
            })
        {
            return Err(invalid(
                "unsupported empty, stub or invalid bootstrap DNS upstream",
            ));
        }
        Ok(Self { servers })
    }

    /// Only a plain resolv.conf topology. Preserve ordered system/VPN servers;
    /// never bind a physical interface or infer a public resolver.
    pub(crate) fn from_plain_system() -> io::Result<Self> {
        if std::path::Path::new("/run/systemd/resolve").exists() {
            return Err(invalid(
                "systemd-resolved split/stub DNS requires a protected per-link adapter",
            ));
        }
        let text = std::fs::read_to_string("/etc/resolv.conf")?;
        let mut servers = Vec::new();
        for line in text.lines() {
            let mut words = line
                .split(['#', ';'])
                .next()
                .unwrap_or_default()
                .split_whitespace();
            if words.next() == Some("nameserver") {
                let ip: IpAddr = words
                    .next()
                    .ok_or_else(|| invalid("missing nameserver"))?
                    .parse()
                    .map_err(|_| invalid("non-IP nameserver"))?;
                if words.next().is_some() {
                    return Err(invalid("invalid nameserver line"));
                }
                servers.push(SocketAddr::new(ip, 53));
            }
        }
        Self::plain_servers(servers)
    }
}

struct Cached {
    addresses: Vec<IpAddr>,
    expires: Instant,
}

pub(crate) struct BootstrapResolver {
    factory: SocketFactory,
    upstreams: SystemDnsSnapshot,
    cache: Mutex<HashMap<(Purpose, Name, RecordType), Cached>>,
}

impl BootstrapResolver {
    pub(crate) fn new(factory: SocketFactory, upstreams: SystemDnsSnapshot) -> Self {
        Self {
            factory,
            upstreams,
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) async fn resolve(
        &self,
        purpose: Purpose,
        hostname: &str,
    ) -> io::Result<Vec<IpAddr>> {
        let _ = purpose.bootstrap()?;
        if self.factory.0.cancelled.is_cancelled() {
            return Err(closed());
        }
        if let Ok(ip) = hostname.parse::<IpAddr>() {
            return Ok(vec![ip]);
        }
        // Absolute names only: no ambient search suffix or business resolver.
        let name = Name::from_ascii(format!("{}.", hostname.trim_end_matches('.')))
            .map_err(|error| invalid(error.to_string()))?;
        let (v6, v4) = tokio::join!(
            self.query(purpose, &name, RecordType::AAAA),
            self.query(purpose, &name, RecordType::A)
        );
        // A transport/upstream failure in either family is not silently NODATA.
        let mut addresses = v6?;
        addresses.extend(v4?);
        if self.factory.0.cancelled.is_cancelled() {
            return Err(closed());
        }
        if addresses.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "bootstrap DNS returned no addresses",
            ));
        }
        Ok(addresses)
    }

    async fn query(
        &self,
        purpose: Purpose,
        name: &Name,
        kind: RecordType,
    ) -> io::Result<Vec<IpAddr>> {
        let key = (purpose, name.clone(), kind);
        {
            let mut cache = self.cache.lock().map_err(|_| closed())?;
            cache.retain(|_, value| value.expires > Instant::now());
            if let Some(value) = cache.get(&key) {
                return Ok(value.addresses.clone());
            }
        }
        let mut random = [0; 2];
        getrandom::fill(&mut random).map_err(|error| io::Error::other(error.to_string()))?;
        let mut request = Message::new(
            u16::from_ne_bytes(random),
            MessageType::Query,
            OpCode::Query,
        );
        request.metadata.recursion_desired = true;
        request.add_query(Query::query(name.clone(), kind));
        let mut last = io::Error::other("no bootstrap DNS upstream");
        for server in &self.upstreams.servers {
            let response = tokio::select! {
                biased;
                _ = self.factory.0.cancelled.cancelled() => return Err(closed()),
                result = tokio::time::timeout(Duration::from_secs(3), self.exchange(purpose, *server, &request)) => {
                    match result {
                        Ok(Ok(response)) => response,
                        Ok(Err(error)) => { last = error; continue; }
                        Err(_) => { last = io::Error::new(io::ErrorKind::TimedOut, "bootstrap DNS timeout"); continue; }
                    }
                }
            };
            if response.metadata.response_code != ResponseCode::NoError {
                return Err(io::Error::other(format!(
                    "bootstrap DNS response: {}",
                    response.metadata.response_code
                )));
            }
            let (addresses, ttl) = addresses(&response, name, kind)?;
            if !addresses.is_empty() && ttl > 0 {
                let mut cache = self.cache.lock().map_err(|_| closed())?;
                if self.factory.0.cancelled.is_cancelled() {
                    return Err(closed());
                }
                // Bounded cache; resolver instance belongs to one generation.
                if cache.len() >= 256 {
                    cache.clear();
                }
                cache.insert(
                    key,
                    Cached {
                        addresses: addresses.clone(),
                        expires: Instant::now() + Duration::from_secs(u64::from(ttl)),
                    },
                );
            }
            return Ok(addresses);
        }
        Err(last)
    }

    async fn exchange(
        &self,
        purpose: Purpose,
        server: SocketAddr,
        request: &Message,
    ) -> io::Result<Message> {
        let purpose = purpose.bootstrap()?;
        let wire = request
            .to_vec()
            .map_err(|error| invalid(error.to_string()))?;
        let bind = SocketAddr::new(
            if server.is_ipv4() {
                IpAddr::from([0, 0, 0, 0])
            } else {
                IpAddr::from([0u16; 8])
            },
            0,
        );
        let socket = self.factory.udp(purpose, bind)?;
        socket.connect(server).await?;
        socket.send_to(&wire, server).await?;
        let mut buffer = vec![0; 65535];
        let (len, source) = socket.recv_from(&mut buffer).await?;
        if source != server {
            return Err(invalid("bootstrap DNS source mismatch"));
        }
        let response = decode(&buffer[..len], request)?;
        if !response.metadata.truncation {
            return Ok(response);
        }
        // TCP fallback has a fresh protected socket, including its first SYN.
        let mut tcp = self.factory.tcp(purpose, server).await?;
        tcp.write_u16(u16::try_from(wire.len()).map_err(|_| invalid("DNS query too large"))?)
            .await?;
        tcp.write_all(&wire).await?;
        let length = tcp.read_u16().await?;
        buffer.resize(usize::from(length), 0);
        tcp.read_exact(&mut buffer).await?;
        let response = decode(&buffer, request)?;
        if response.metadata.truncation {
            return Err(invalid("truncated TCP DNS response"));
        }
        Ok(response)
    }
}

fn decode(wire: &[u8], query: &Message) -> io::Result<Message> {
    let response = Message::from_vec(wire).map_err(|error| invalid(error.to_string()))?;
    if response.metadata.id != query.metadata.id
        || response.metadata.message_type != MessageType::Response
        || response.metadata.op_code != OpCode::Query
        || response.queries != query.queries
    {
        return Err(invalid("bootstrap DNS transaction/question mismatch"));
    }
    Ok(response)
}

fn addresses(response: &Message, name: &Name, kind: RecordType) -> io::Result<(Vec<IpAddr>, u32)> {
    let mut current = name.clone();
    let mut ttl = u32::MAX;
    let mut visited = std::collections::HashSet::new();
    for _ in 0..16 {
        if !visited.insert(current.clone()) {
            return Err(invalid("DNS CNAME cycle"));
        }
        let mut addresses = Vec::new();
        let mut alias = None;
        for record in &response.answers {
            if record.name != current || record.dns_class != DNSClass::IN {
                continue;
            }
            match &record.data {
                RData::A(ip) if kind == RecordType::A => {
                    addresses.push(IpAddr::V4(ip.0));
                    ttl = ttl.min(record.ttl);
                }
                RData::AAAA(ip) if kind == RecordType::AAAA => {
                    addresses.push(IpAddr::V6(ip.0));
                    ttl = ttl.min(record.ttl);
                }
                RData::CNAME(target) => {
                    if alias.as_ref().is_some_and(|name| name != &target.0) {
                        return Err(invalid("conflicting DNS CNAME targets"));
                    }
                    alias = Some(target.0.clone());
                    ttl = ttl.min(record.ttl);
                }
                _ => {}
            }
        }
        if !addresses.is_empty() {
            if alias.is_some() {
                return Err(invalid("DNS CNAME and address share owner"));
            }
            return Ok((addresses, ttl));
        }
        match alias {
            Some(name) => current = name,
            None => return Ok((Vec::new(), 0)),
        }
    }
    Err(invalid("DNS CNAME chain too long"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::rr::{Record, rdata::A};

    #[test]
    fn validates_question_and_ignores_unrelated_answers() {
        let name = Name::from_ascii("controller.test.").unwrap();
        let mut query = Message::new(7, MessageType::Query, OpCode::Query);
        query.add_query(Query::query(name.clone(), RecordType::A));
        let mut response = Message::response(7, OpCode::Query);
        response.queries = query.queries.clone();
        response.add_answer(Record::from_rdata(
            Name::from_ascii("unrelated.test.").unwrap(),
            100,
            RData::A(A(std::net::Ipv4Addr::LOCALHOST)),
        ));
        assert!(decode(&response.to_vec().unwrap(), &query).is_ok());
        assert!(
            addresses(&response, &name, RecordType::A)
                .unwrap()
                .0
                .is_empty()
        );
        response.metadata.id = 8;
        assert!(decode(&response.to_vec().unwrap(), &query).is_err());
    }

    #[test]
    fn daemon_and_empty_dns_are_explicitly_unsupported() {
        assert!(SystemDnsSnapshot::plain_servers(vec![]).is_err());
        assert!(SystemDnsSnapshot::plain_servers(vec!["127.0.0.53:53".parse().unwrap()]).is_err());
        assert!(SystemDnsSnapshot::plain_servers(vec!["[::1]:53".parse().unwrap()]).is_err());
    }
}
