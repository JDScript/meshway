//! OS resolver conformance and a bounded, isolated external DNS authority.
//! No control, policy, DNS installation or dataplane owners are constructed here.
use crate::{
    Cli, DnsBackend, Error, Result,
    signals::{Addresses, Role},
};
use hickory_proto::{
    op::{Message, Query as Question, ResponseCode},
    rr::{Name, RData, Record, RecordType, rdata},
};
use meshway_configuration::{DnsServer, DnsTransport, NetworkConfigurationInput};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UdpSocket},
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;

const LIMIT: Duration = Duration::from_secs(5);
// The pinned musl resolver ignores REFUSED as a terminal UDP result and uses
// its configured five-second total retry window before returning EAI_AGAIN.
// Keep headroom around the completed libc result, separate from wire I/O.
const SYSTEM_LOOKUP_LIMIT: Duration = Duration::from_secs(15);
const PUBLIC4: Ipv4Addr = Ipv4Addr::new(203, 0, 113, 7);
const PUBLIC6: Ipv6Addr = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 7);

pub fn configure(input: &mut NetworkConfigurationInput, server: Ipv4Addr) -> Result<()> {
    for (key, transport, suffix) in [
        ("office", DnsTransport::Udp, "office.example"),
        ("tcp-office", DnsTransport::Tcp, "tcp.example"),
    ] {
        input.dns.servers.insert(
            key.into(),
            DnsServer {
                transport,
                server: server.into(),
                server_port: 53.try_into()?,
                use_with_exit: false,
            },
        );
        input.dns.routes.insert(suffix.into(), vec![key.into()]);
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Evidence {
    pub backend: DnsBackend,
    pub samples: Vec<Sample>,
    pub original_resolv_conf_restored: Option<bool>,
    pub original_links_restored: Option<bool>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Sample {
    stage: String,
    name: String,
    addresses: Vec<IpAddr>,
    failed: bool,
    service_rcode: Option<u16>,
}

pub struct Probe {
    evidence: Evidence,
    original: Vec<u8>,
    links: BTreeSet<String>,
}

impl Probe {
    pub async fn start(cli: &Cli, role: Role) -> Result<Option<Self>> {
        if !cli.dns_backend.enabled() {
            return Ok(None);
        }
        let mut probe = Self {
            evidence: Evidence {
                backend: cli.dns_backend,
                samples: Vec::new(),
                original_resolv_conf_restored: None,
                original_links_restored: None,
            },
            original: tokio::fs::read("/etc/resolv.conf").await?,
            links: links().await?,
        };
        probe.public(role, "before", true).await?;
        probe.save(cli).await?;
        Ok(Some(probe))
    }

    async fn lookup(
        &mut self,
        stage: &str,
        name: &str,
        expected: Option<BTreeSet<IpAddr>>,
    ) -> Result<()> {
        // Tokio's system lookup delegates to libc getaddrinfo; no resolver library,
        // raw DNS request or /etc/hosts entry substitutes for this observation.
        let result =
            tokio::time::timeout(SYSTEM_LOOKUP_LIMIT, tokio::net::lookup_host((name, 0))).await?;
        let failed = result.is_err();
        let actual: BTreeSet<_> = result
            .map(|addresses| addresses.map(|address| address.ip()).collect())
            .unwrap_or_default();
        let valid = match expected {
            Some(expected) => !failed && actual == expected,
            None => failed,
        };
        self.evidence.samples.push(Sample {
            stage: stage.into(),
            name: name.into(),
            addresses: actual.into_iter().collect(),
            failed,
            service_rcode: None,
        });
        if !valid {
            return Err(format!("system getaddrinfo assertion failed: {stage} {name}").into());
        }
        Ok(())
    }

    async fn public(&mut self, role: Role, stage: &str, allowed: bool) -> Result<()> {
        let name = format!("{}-{stage}.public.test.", role.name());
        self.lookup(
            stage,
            &name,
            allowed.then(|| BTreeSet::from([PUBLIC4.into(), PUBLIC6.into()])),
        )
        .await
    }

    pub async fn active(
        &mut self,
        cli: &Cli,
        role: Role,
        domain: &str,
        peer: &Addresses,
        stage: &str,
        allowed: bool,
    ) -> Result<()> {
        if role == Role::A {
            let name = format!("b.{}.", domain.trim_end_matches('.'));
            self.lookup(
                stage,
                &name,
                allowed.then(|| BTreeSet::from([peer.ipv4.into(), peer.ipv6.into()])),
            )
            .await?;
            if allowed {
                self.lookup(
                    stage,
                    "b",
                    Some(BTreeSet::from([peer.ipv4.into(), peer.ipv6.into()])),
                )
                .await?;
            }
            for suffix in ["office.example", "tcp.example", "fallback.public.test"] {
                self.lookup(
                    stage,
                    &format!("{stage}.{suffix}."),
                    Some(BTreeSet::from([PUBLIC4.into(), PUBLIC6.into()])),
                )
                .await?;
            }
        }
        // A→R is now an explicit ICMP target for direct diagnostics. Neither
        // R nor B has reverse business access to A: a physical adjacency or
        // original-path reply permission must not make that known name visible.
        if role != Role::A {
            self.refused(cli, stage, &format!("a.{}.", domain.trim_end_matches('.')))
                .await?;
        }
        self.refused(
            cli,
            stage,
            &format!("absent-fixture-node.{}.", domain.trim_end_matches('.')),
        )
        .await?;
        self.public(role, stage, true).await?;
        self.save(cli).await
    }

    async fn refused(&mut self, cli: &Cli, stage: &str, name: &str) -> Result<()> {
        // Exercise libc's refusal window after all actors are ready; every
        // phase also requires the exact service-level REFUSED response.
        if stage == "initial" {
            self.lookup(stage, name, None).await?;
        }
        let deadline = tokio::time::Instant::now() + LIMIT;
        loop {
            // A fenced in-flight transaction may return SERVFAIL. Preserve it
            // as an observation, then require REFUSED from a fresh transaction.
            let code = tokio::time::timeout_at(deadline, service_rcode(cli, name)).await??;
            self.evidence.samples.push(Sample {
                stage: stage.into(),
                name: name.into(),
                addresses: Vec::new(),
                failed: code != ResponseCode::NoError,
                service_rcode: Some(code.into()),
            });
            if code == ResponseCode::Refused {
                return Ok(());
            }
            if code != ResponseCode::ServFail || tokio::time::Instant::now() >= deadline {
                return Err("source-invisible/unknown Mesh DNS did not return REFUSED".into());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub async fn expired(&mut self, cli: &Cli, role: Role, domain: &str) -> Result<()> {
        self.unavailable(cli, role, domain, "lease-expired").await
    }

    pub async fn unavailable(
        &mut self,
        cli: &Cli,
        role: Role,
        domain: &str,
        stage: &str,
    ) -> Result<()> {
        self.lookup(stage, &format!("b.{}.", domain.trim_end_matches('.')), None)
            .await?;
        self.public(role, stage, cli.dns_backend == DnsBackend::Resolved)
            .await?;
        self.save(cli).await
    }

    pub async fn down(mut self, cli: &Cli, role: Role) -> Result<Evidence> {
        self.evidence.original_resolv_conf_restored =
            Some(self.original == tokio::fs::read("/etc/resolv.conf").await?);
        self.evidence.original_links_restored = Some(self.links == links().await?);
        self.save(cli).await?;
        if self.evidence.original_resolv_conf_restored != Some(true)
            || self.evidence.original_links_restored != Some(true)
        {
            return Err("DNS down did not restore resolver bytes/interface inventory".into());
        }
        self.public(role, "down", true).await?;
        self.save(cli).await?;
        Ok(self.evidence)
    }

    pub async fn save(&self, cli: &Cli) -> Result<()> {
        tokio::fs::write(
            cli.evidence.join("dns.json"),
            serde_json::to_vec_pretty(&self.evidence)?,
        )
        .await?;
        Ok(())
    }
}

async fn links() -> Result<BTreeSet<String>> {
    let mut entries = tokio::fs::read_dir("/sys/class/net").await?;
    let mut names = BTreeSet::new();
    while let Some(entry) = entries.next_entry().await? {
        names.insert(
            entry
                .file_name()
                .into_string()
                .map_err(|_| "invalid interface name")?,
        );
    }
    Ok(names)
}

async fn service_rcode(cli: &Cli, name: &str) -> Result<ResponseCode> {
    tokio::time::timeout(LIMIT, async {
        let mut entropy = [0; 2];
        getrandom::fill(&mut entropy).map_err(|_| "DNS query entropy unavailable")?;
        let mut query = Message::new(
            u16::from_ne_bytes(entropy),
            hickory_proto::op::MessageType::Query,
            hickory_proto::op::OpCode::Query,
        );
        query.add_query(Question::query(Name::from_ascii(name)?, RecordType::A));
        let socket = UdpSocket::bind("127.0.0.1:0").await?;
        socket.connect(cli.dns_backend.bind()).await?;
        socket.send(&query.to_vec()?).await?;
        let mut bytes = [0; 4096];
        let size = socket.recv(&mut bytes).await?;
        let response = Message::from_vec(&bytes[..size])?;
        if response.metadata.id != query.metadata.id
            || response.queries != query.queries
            || response.metadata.message_type != hickory_proto::op::MessageType::Response
        {
            return Err("DNS refusal response correlation failed".into());
        }
        Ok::<_, Error>(response.metadata.response_code)
    })
    .await?
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpstreamEvidence {
    queries: BTreeMap<String, BTreeMap<String, u64>>,
    unexpected: u64,
}

pub struct Upstream {
    stop: CancellationToken,
    task: Option<JoinHandle<Result<()>>>,
    evidence: Arc<Mutex<UpstreamEvidence>>,
}

impl Upstream {
    pub async fn start(cli: &Cli) -> Result<Self> {
        let stop = CancellationToken::new();
        let evidence = Arc::new(Mutex::new(UpstreamEvidence::default()));
        let task = if cli.dns_backend.enabled() {
            let udp = UdpSocket::bind("0.0.0.0:53").await?;
            let tcp = TcpListener::bind("0.0.0.0:53").await?;
            let stopped = stop.clone();
            let recorded = evidence.clone();
            Some(tokio::spawn(async move {
                let mut connections = JoinSet::new();
                let result = async {
                    let mut bytes = [0; 4096];
                    loop {
                        tokio::select! {
                            _ = stopped.cancelled() => return Ok::<(), Error>(()),
                            Some(result) = connections.join_next(), if !connections.is_empty() => { result??; }
                            received = udp.recv_from(&mut bytes) => {
                                let (size, peer) = received?;
                                let response = answer(&bytes[..size], false, &recorded)?;
                                tokio::time::timeout(LIMIT, udp.send_to(&response, peer)).await??;
                            }
                            accepted = tcp.accept(), if connections.len() < 32 => {
                                let (mut stream, _) = accepted?;
                                let recorded = recorded.clone();
                                connections.spawn(async move {
                                    tokio::time::timeout(LIMIT, async {
                                        let size = usize::from(stream.read_u16().await?);
                                        if !(12..=4096).contains(&size) { return Err("upstream TCP DNS length out of bounds".into()); }
                                        let mut bytes = vec![0; size];
                                        stream.read_exact(&mut bytes).await?;
                                        let response = answer(&bytes, true, &recorded)?;
                                        stream.write_u16(response.len().try_into()?).await?;
                                        stream.write_all(&response).await?;
                                        Ok::<(), Error>(())
                                    }).await?
                                });
                            }
                        }
                    }
                }.await;
                connections.abort_all();
                while let Some(joined) = connections.join_next().await {
                    match joined {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => return Err(error),
                        Err(error) if error.is_cancelled() => {}
                        Err(error) => return Err(error.into()),
                    }
                }
                result
            }))
        } else {
            None
        };
        Ok(Self {
            stop,
            task,
            evidence,
        })
    }

    pub async fn shutdown(self, cli: &Cli) -> Result<()> {
        self.stop.cancel();
        if let Some(task) = self.task {
            task.await??;
        }
        if !cli.dns_backend.enabled() {
            return Ok(());
        }
        let evidence = self.evidence.lock().map_err(|_| "DNS evidence poisoned")?;
        std::fs::write(
            cli.evidence.join("dns-upstream.json"),
            serde_json::to_vec_pretty(&*evidence)?,
        )?;
        if evidence.unexpected != 0 {
            return Err("non-public/Mesh query leaked to upstream".into());
        }
        for stage in [
            "before-restart",
            "after-restart",
            "initial",
            "revoked",
            "restored",
            "recovered",
        ] {
            for (suffix, protocol) in [
                ("office.example.", "udp"),
                ("tcp.example.", "tcp"),
                ("fallback.public.test.", "udp"),
                ("fallback.public.test.", "tcp"),
            ] {
                if !evidence
                    .queries
                    .get(&format!("{stage}.{suffix}"))
                    .and_then(|counts| counts.get(protocol))
                    .is_some_and(|count| *count > 0)
                {
                    return Err("missing actual DNS split/UDP-to-TCP fallback evidence".into());
                }
            }
        }
        if evidence
            .queries
            .iter()
            .any(|(name, counts)| name.ends_with(".tcp.example.") && counts.contains_key("udp"))
        {
            return Err("TCP-configured split DNS used UDP".into());
        }
        for role in Role::ALL {
            for stage in [
                "before",
                "before-restart",
                "after-restart",
                "initial",
                "revoked",
                "restored",
                "recovered",
                "down",
            ] {
                if !evidence
                    .queries
                    .contains_key(&format!("{}-{stage}.public.test.", role.name()))
                {
                    return Err("missing ordinary OS DNS upstream observation".into());
                }
            }
            let expired = evidence
                .queries
                .contains_key(&format!("{}-lease-expired.public.test.", role.name()));
            if expired != (cli.dns_backend == DnsBackend::Resolved) {
                return Err("expired ordinary DNS did not follow backend scope boundary".into());
            }
        }
        Ok(())
    }
}

fn answer(bytes: &[u8], tcp: bool, evidence: &Mutex<UpstreamEvidence>) -> Result<Vec<u8>> {
    let query = meshway_dns::decode_query(bytes)?;
    let name = query.question().name().to_ascii().to_ascii_lowercase();
    let known = [".public.test.", ".office.example.", ".tcp.example."]
        .iter()
        .any(|suffix| name.ends_with(suffix));
    {
        let mut evidence = evidence.lock().map_err(|_| "DNS evidence poisoned")?;
        if evidence.queries.len() >= 256 && !evidence.queries.contains_key(&name) {
            return Err("DNS evidence bound exceeded".into());
        }
        *evidence
            .queries
            .entry(name.clone())
            .or_default()
            .entry(if tcp { "tcp" } else { "udp" }.into())
            .or_default() += 1;
        if !known {
            evidence.unexpected += 1;
        }
    }
    let mut response = query.response();
    response.metadata.recursion_available = true;
    if !known {
        response.metadata.response_code = ResponseCode::Refused;
    } else if !tcp && name.ends_with(".fallback.public.test.") {
        response.metadata.truncation = true;
    } else {
        let data = match query.question().query_type() {
            RecordType::A => Some(RData::A(rdata::A(PUBLIC4))),
            RecordType::AAAA => Some(RData::AAAA(rdata::AAAA(PUBLIC6))),
            _ => None,
        };
        if let Some(data) = data {
            response.add_answer(Record::from_rdata(query.question().name().clone(), 0, data));
        }
    }
    Ok(response.to_vec()?)
}
