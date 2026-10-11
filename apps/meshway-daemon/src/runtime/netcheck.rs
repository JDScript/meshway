//! `meshway netcheck`: one bounded round of the existing observations. The
//! Manager's traversal sockets observe their mappings on request and the
//! Control client makes one read-only Server request; this module only
//! classifies what they returned.
use meshway_connectivity::{StunProbe, StunReport};
use meshway_model::{Sequence, SocketAddress};
use meshway_protocol::ipc::{
    EndpointMapping, NetcheckObserver, NetcheckReport, NetcheckServer, NetcheckSocket,
    NetcheckUnavailable, PortMapping, StunMapping,
};
use std::{collections::BTreeMap, time::Duration};

/// Each traversal socket gets one ordinary STUN round (its 5 s transaction).
const STUN_WAIT: Duration = Duration::from_secs(5);
const SERVER_WAIT: Duration = Duration::from_secs(5);

pub(super) async fn run(
    origin: String,
    local: Option<super::network::Local>,
    server: Option<meshway_control::ServerProbe>,
) -> NetcheckReport {
    let probe = async {
        match &local {
            Some(local) => local.netcheck().await,
            None => None,
        }
    };
    let stun = async {
        let probe: Option<StunProbe> = probe.await;
        match probe {
            Some(probe) => Ok(probe.wait(STUN_WAIT).await),
            None => Err(NetcheckUnavailable::NotRunning),
        }
    };
    let rtt = async {
        let server = server.as_ref()?;
        Some(
            tokio::time::timeout(SERVER_WAIT, server.round_trip())
                .await
                .map_err(|_| ())
                .and_then(|result| result.map_err(|_| ())),
        )
    };
    let (stun, rtt) = tokio::join!(stun, rtt);
    report(origin, stun, rtt)
}

fn micros(duration: Duration) -> Option<Sequence> {
    Sequence::new(u64::try_from(duration.as_micros()).ok()?).ok()
}

pub(super) fn report(
    origin: String,
    stun: Result<StunReport, NetcheckUnavailable>,
    rtt: Option<Result<Duration, ()>>,
) -> NetcheckReport {
    let server = NetcheckServer {
        origin,
        reachable: rtt.as_ref().map(Result::is_ok),
        rtt_us: rtt.and_then(Result::ok).and_then(micros),
    };
    let stun = match stun {
        Ok(stun) => stun,
        Err(reason) => {
            return NetcheckReport {
                observed_at: chrono::Utc::now(),
                server,
                unavailable: Some(reason),
                ipv4: false,
                ipv6: false,
                udp: false,
                sockets: Vec::new(),
                observers: Vec::new(),
            };
        }
    };
    let mut observers: BTreeMap<SocketAddress, Option<Sequence>> = BTreeMap::new();
    let (mut ipv4, mut ipv6) = (false, false);
    let sockets: Vec<NetcheckSocket> = stun
        .sockets
        .iter()
        .filter_map(|socket| {
            let bound = SocketAddress::try_from(socket.socket.bound_address).ok()?;
            let mappings: Vec<StunMapping> = socket
                .samples
                .iter()
                .filter_map(|(observer, sample)| {
                    Some(StunMapping {
                        observer: SocketAddress::try_from(*observer).ok()?,
                        mapped: SocketAddress::try_from(sample.mapped).ok()?,
                        rtt_us: sample.rtt.and_then(micros),
                        fresh: sample.at >= stun.requested_at,
                    })
                })
                .collect();
            let fresh: Vec<&StunMapping> = mappings.iter().filter(|m| m.fresh).collect();
            for mapping in &fresh {
                let best = observers.entry(mapping.observer).or_default();
                if let Some(rtt) = mapping.rtt_us
                    && best.is_none_or(|known| rtt < known)
                {
                    *best = Some(rtt);
                }
                if mapping.mapped.get().is_ipv4() {
                    ipv4 = true;
                } else {
                    ipv6 = true;
                }
            }
            let port_mapping = if fresh.is_empty() {
                PortMapping::Unknown
            } else if fresh
                .iter()
                .all(|m| m.mapped.get().port() == socket.socket.bound_address.port())
            {
                PortMapping::Preserved
            } else {
                PortMapping::Changed
            };
            let endpoint_mapping = if fresh.len() < 2 {
                EndpointMapping::Unknown
            } else if fresh.iter().all(|m| m.mapped == fresh[0].mapped) {
                EndpointMapping::Independent
            } else {
                EndpointMapping::Dependent
            };
            Some(NetcheckSocket {
                resource: socket.socket.reference.resource.clone(),
                bound_address: bound,
                mappings,
                port_mapping,
                endpoint_mapping,
            })
        })
        .collect();
    NetcheckReport {
        observed_at: chrono::Utc::now(),
        server,
        unavailable: None,
        ipv4,
        ipv6,
        udp: ipv4 || ipv6 || stun.udp_links > 0,
        sockets,
        observers: observers
            .into_iter()
            .map(|(observer, rtt_us)| NetcheckObserver { observer, rtt_us })
            .collect(),
    }
}
