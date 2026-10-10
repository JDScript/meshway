//! Run-private rendezvous contains public identities/status, never credentials.
use crate::{Result, STARTUP};
use meshway_ipc::Status;
use meshway_model::Revision;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    time::Duration,
};

pub use meshway_packet_fixture::rendezvous::Role;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Underlay {
    pub controller: Ipv4Addr,
    pub actors: BTreeMap<Role, Ipv4Addr>,
    #[serde(default)]
    pub routers: BTreeMap<Role, NatRouter>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NatRouter {
    pub private: Ipv4Addr,
    pub wan: Ipv4Addr,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Addresses {
    pub ipv4: std::net::Ipv4Addr,
    pub ipv6: std::net::Ipv6Addr,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Signal {
    Status {
        role: Role,
        status: Box<Status>,
    },
    Configuration {
        revision: Revision,
        addresses: BTreeMap<Role, Addresses>,
        dns_domain: String,
    },
    Revision {
        revision: Revision,
    },
    Ready,
    /// A netmap scenario step: the configuration revision it produced and
    /// when the Controller committed it.
    Step {
        revision: Revision,
        at_ms: u64,
    },
    /// When an actor observed a scenario step's effect.
    Observed {
        role: Role,
        at_ms: u64,
    },
    PingRun {
        run: Box<crate::ping::Run>,
    },
    Installations {
        evidence: Box<crate::traversal::Installations>,
    },
    HttpAvailability {
        available: bool,
        observed_at_ms: u64,
    },
    Expired {
        role: Role,
        evidence: Box<crate::recovery::Expired>,
    },
    Report {
        role: Role,
        node: meshway_model::NodeId,
        udp4: u64,
        udp6: u64,
        tcp4: u64,
        tcp6: u64,
        relay_tun_received: Option<u64>,
        tun_removed: bool,
        capture_removed: bool,
        identity_preserved: bool,
        dns: Option<crate::dns::Evidence>,
        fallback: Option<Box<crate::fallback::Evidence>>,
        recovery: Box<crate::recovery::Evidence>,
        ping: Option<Box<crate::ping::PingEvidence>>,
    },
}

#[derive(Debug)]
pub(super) struct DecodeError {
    pub signal: String,
    pub source: serde_json::Error,
}
impl DecodeError {
    pub fn detail(&self) -> &'static str {
        if self.source.to_string() == "u128 is not supported" {
            "unsupported_u128"
        } else {
            crate::diagnostics::json_category(&self.source)
        }
    }
}
impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "rendezvous decode failed: {} ({})",
            self.signal,
            self.detail()
        )?;
        if self.source.line() > 0 {
            write!(f, " at {}:{}", self.source.line(), self.source.column())?;
        }
        Ok(())
    }
}
impl std::error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}
impl Signal {
    /// The same decoder serves live rendezvous and retained artifact inspection.
    /// Its public diagnostic contains no serde payload/value text.
    pub(super) fn decode(name: &str, bytes: &[u8]) -> std::result::Result<Self, DecodeError> {
        serde_json::from_slice(bytes).map_err(|source| DecodeError {
            signal: if name.len() <= 96
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            {
                name.to_owned()
            } else {
                "unrecognized-signal".into()
            },
            source,
        })
    }
}

pub struct Signals(PathBuf);
impl Signals {
    pub fn new(path: &Path) -> Self {
        Self(path.into())
    }
    pub async fn write(&self, name: &str, value: &Signal) -> Result<()> {
        meshway_packet_fixture::rendezvous::write(&self.0, name, value).await
    }
    pub async fn read(&self, name: &str) -> Result<Option<Signal>> {
        for role in ["a", "r", "b", "controller"] {
            if tokio::fs::try_exists(self.0.join(format!("abort-{role}.json"))).await? {
                return Err("another runtime actor failed; inspect its private evidence".into());
            }
        }
        let path = self.0.join(format!("{name}.json"));
        let metadata = match tokio::fs::metadata(&path).await {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        // A final report embeds all twelve bounded CLI streams (including the
        // live outage stream). Other rendezvous messages retain the small cap.
        let limit = if name.starts_with("report-") {
            16 * 1024 * 1024
        } else {
            512 * 1024
        };
        if !metadata.is_file() || metadata.len() > limit {
            return Err("invalid rendezvous file".into());
        }
        Ok(Some(Signal::decode(name, &tokio::fs::read(path).await?)?))
    }
    pub async fn wait_for(&self, name: &str, duration: Duration) -> Result<Signal> {
        tokio::time::timeout(crate::remaining(duration)?, async {
            loop {
                if let Some(signal) = self.read(name).await? {
                    return Ok(signal);
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await?
    }
    pub async fn wait(&self, name: &str) -> Result<Signal> {
        self.wait_for(name, STARTUP).await
    }
    pub async fn ready(&self, name: &str) -> Result<()> {
        if !matches!(self.wait(name).await?, Signal::Ready) {
            return Err("invalid ready signal".into());
        }
        Ok(())
    }
}

pub async fn underlay(cli: &crate::Cli) -> Result<Underlay> {
    let underlay: Underlay =
        serde_json::from_slice(&tokio::fs::read(cli.public.join("underlay.json")).await?)?;
    if underlay.actors.len() != 3
        || Role::ALL
            .iter()
            .any(|role| !underlay.actors.contains_key(role))
    {
        return Err("incomplete underlay actor addresses".into());
    }
    Ok(underlay)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        application::{Counts, PacketEvidence},
        recovery::{Evidence, Expired},
    };

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            use std::os::unix::fs::DirBuilderExt;
            let path = std::env::temp_dir()
                .join(format!("meshway-signal-test-{}", crate::request().unwrap()));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn status() -> Status {
        serde_json::from_str(
            r#"{
            "processId":42,"revision":1,"desiredUp":false,"phase":"down",
            "sync":"disconnected","lastSyncAt":null,"netmap":null,
            "server":"http://127.0.0.1:8430/","observedAt":"2026-10-05T00:00:00Z",
            "enrollmentRequestId":null,"enrollmentUrl":null,"identity":null,
            "authenticated":false,"instance":null,"saved":null,"applied":null,
            "dataPlane":"controlOnly","networkMode":"controlOnly","lastError":null,"retryAt":null
        }"#,
        )
        .unwrap()
    }

    fn report(elapsed: u64) -> Signal {
        // Envelope roundtrip data, not simulated successful runtime evidence.
        // PacketEvidence is constructed through its formal decoder so the test
        // fails specifically at Signal's tagged buffering with the old u128.
        let packet: PacketEvidence = serde_json::from_slice(
            &serde_json::to_vec(&serde_json::json!({
                "sockets":["127.0.0.1:40000","[::1]:40001","127.0.0.1:40002","[::1]:40003"],
                "counts":{"udp4":1,"udp6":1,"tcp4":1,"tcp6":1},
                "elapsedMs":elapsed,"lastEchoSequence":1,"echoedFrame":[1,2,3,4]
            }))
            .unwrap(),
        )
        .unwrap();
        Signal::Report {
            role: Role::A,
            node: "00000000-0000-4000-8000-000000000001".parse().unwrap(),
            udp4: 1,
            udp6: 1,
            tcp4: 1,
            tcp6: 1,
            relay_tun_received: None,
            tun_removed: true,
            capture_removed: true,
            identity_preserved: true,
            dns: None,
            fallback: None,
            ping: Some(Box::new(crate::ping::PingEvidence {
                runs: vec![crate::ping::roundtrip_run(elapsed)],
                business_continuity: None,
                diagnostic_only_business_packets: None,
            })),
            recovery: Box::new(Evidence {
                expired: Expired {
                    before: Box::new(status()),
                    status: Box::new(status()),
                    closed_at_ms: 1,
                    instance_expires_at_ms: 2,
                    netmap_expires_at_ms: 2,
                    observed_at_ms: 3,
                    old_traffic_blocked: None,
                },
                recovered: Box::new(status()),
                reopened_at_ms: 4,
                old_tcp_blocked_after_recovery: None,
                old_sockets: None,
                old_circuits: vec!["a".repeat(64)],
                new_circuits: vec!["b".repeat(64)],
                traffic: Some(packet),
                delivered: Counts::default(),
                relay_tun_received: None,
            }),
        }
    }

    #[tokio::test]
    async fn full_report_with_packet_evidence_roundtrips_through_tagged_signal_and_files() {
        let directory = Directory::new();
        let signals = Signals::new(&directory.0);
        for elapsed in [125_001, u64::MAX] {
            let report = report(elapsed);
            let expected = serde_json::to_vec(&report).unwrap();
            let decoded = Signal::decode("report-a", &expected).unwrap();
            assert_eq!(serde_json::to_vec(&decoded).unwrap(), expected);
            signals.write("report-a", &report).await.unwrap();
            let decoded = signals.read("report-a").await.unwrap().unwrap();
            assert_eq!(serde_json::to_vec(&decoded).unwrap(), expected);
        }
    }

    #[test]
    #[ignore = "read-only retained Linux report inspection; set MESHWAY_RETAINED_RUNTIME_REPORTS to run directories"]
    fn retained_linux_reports_use_formal_signal_decoder_and_controller_validation() {
        use std::io::Read;
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Mode {
            dns_backend: crate::DnsBackend,
            traversal_fallback: Option<BTreeMap<String, serde::de::IgnoredAny>>,
        }
        let paths = std::env::var_os("MESHWAY_RETAINED_RUNTIME_REPORTS")
            .expect("explicit retained run directories required");
        let mut count = 0;
        let mut failures = 0;
        for (index, path) in std::env::split_paths(&paths).enumerate() {
            let mode: Mode =
                serde_json::from_slice(&std::fs::read(path.join("report.json")).unwrap()).unwrap();
            for role in Role::ALL {
                let name = format!("report-{}", role.name());
                let mut bytes = Vec::new();
                std::fs::File::open(path.join("signals").join(format!("{name}.json")))
                    .unwrap()
                    .take(512 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .unwrap();
                assert!(bytes.len() <= 512 * 1024);
                // Original abort markers/run verdicts stay intact. This uses
                // the live decoder directly, without bypassing abort handling
                // for a running Signals consumer or writing into old evidence.
                match Signal::decode(&name, &bytes) {
                    Ok(report) => {
                        crate::controller::validate_report(
                            &report,
                            role,
                            mode.dns_backend,
                            mode.traversal_fallback.is_some(),
                        )
                        .unwrap();
                        eprintln!(
                            "retained-run={index} role={} formal decode and validation passed",
                            role.name()
                        );
                    }
                    Err(error) => {
                        eprintln!(
                            "retained-run={index} role={} formal decode failed: {}",
                            role.name(),
                            error.detail()
                        );
                        failures += 1;
                    }
                }
                count += 1;
            }
        }
        assert!(count > 0);
        assert_eq!(failures, 0, "retained report decoding failures");
    }
}
