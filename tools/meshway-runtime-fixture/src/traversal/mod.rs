//! Read-only observation of the real Server routes and real reported sockets.
//! No acquisition requests originate here; Node credentials stay in the daemon.
use crate::{
    Cli, Result,
    signals::{self, Role},
};
use axum::{
    body::{Body, to_bytes},
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use meshway_model::{
    NodeId,
    state::{EndpointSource, LinkPhase, NodeState, SocketNetwork},
};
use meshway_protocol::connectivity::*;
use meshway_protocol::state::{StateAccepted, StateReport};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

mod installation;
#[cfg(test)]
mod tests;
mod verification;
pub use installation::{Installations, publish_installations};

const BODY_LIMIT: usize = 256 * 1024;
const EVENT_LIMIT: usize = 4096;
const PREFIX: &str = "/agent-api/v1/connectivity/udp/";

#[derive(Clone, Default)]
pub struct Audit(Arc<Mutex<Observations>>);

#[derive(Default, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Observations {
    // Every entry came through the production nodeSession-only authenticated
    // route with a successful response; Authorization is never copied.
    events: Vec<Event>,
    failed: bool,
    bytes: usize,
}

#[derive(Serialize, serde::Deserialize)]
#[serde(
    tag = "operation",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Event {
    Publication {
        instance_id: meshway_model::InstanceId,
        sequence: meshway_model::Sequence,
        applied: Option<meshway_model::NetmapRef>,
        sockets: BTreeMap<String, meshway_model::state::SocketState>,
    },
    Query {
        request: QueryUdpTraversalRequest,
        response: QueryUdpTraversalResponse,
    },
    Prepare {
        request: Box<PrepareUdpTraversalRequest>,
        response: PrepareUdpTraversalResponse,
    },
    Accept {
        request: Box<UdpTraversalAttemptRequest>,
        response: AcceptUdpTraversalResponse,
        received_at_ms: u64,
    },
    Cancel {
        request: UdpTraversalAttemptRequest,
        response: CancelUdpTraversalResponse,
    },
}

fn now_ms() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis()
        .try_into()?)
}

pub async fn observe(State(audit): State<Audit>, request: Request, next: Next) -> Response {
    if !request.uri().path().starts_with(PREFIX) && request.uri().path() != "/agent-api/v1/state" {
        return next.run(request).await;
    }
    let path = request.uri().path().to_owned();
    let authenticated_header = request
        .headers()
        .contains_key(axum::http::header::AUTHORIZATION);
    let (parts, body) = request.into_parts();
    let Ok(input) = to_bytes(body, BODY_LIMIT).await else {
        audit.fail();
        return Response::builder()
            .status(413)
            .body(Body::empty())
            .expect("fixed status");
    };
    let response = next
        .run(Request::from_parts(parts, Body::from(input.clone())))
        .await;
    if !response.status().is_success() {
        return response;
    }
    let (parts, body) = response.into_parts();
    let Ok(output) = to_bytes(body, BODY_LIMIT).await else {
        audit.fail();
        return Response::builder()
            .status(502)
            .body(Body::empty())
            .expect("fixed status");
    };
    if !authenticated_header || audit.record(&path, &input, &output).is_err() {
        audit.fail();
    }
    Response::from_parts(parts, Body::from(output))
}

impl Audit {
    /// Bind a CLI-observed physical channel to this run's authenticated immutable
    /// offer and both acceptances. Candidates alone are never a channel proof.
    pub fn verify_ping_channel(
        &self,
        channel: &meshway_model::observations::ProbeChannel,
        source: NodeId,
        instance: meshway_model::InstanceId,
    ) -> Result<UdpTraversalOffer> {
        use meshway_model::observations::{EndpointKind, LinkAcquisition};
        let LinkAcquisition::CoordinatedUdp {
            attempt_id,
            endpoint_source: EndpointKind::Stun,
        } = &channel.acquisition
        else {
            return Err("NAT ping channel has no actual STUN traversal provenance".into());
        };
        let observations = self.0.lock().map_err(|_| "traversal audit poisoned")?;
        if observations.failed {
            return Err("traversal HTTP audit incomplete".into());
        }
        for event in &observations.events {
            let Event::Prepare {
                response: PrepareUdpTraversalResponse::Offer { offer },
                ..
            } = event
            else {
                continue;
            };
            if offer.attempt_id != *attempt_id || offer.compute_digest()? != offer.digest {
                continue;
            }
            let (local, peer) = if offer.local.candidate.socket.node_id == source {
                (&offer.local, &offer.peer)
            } else if offer.peer.candidate.socket.node_id == source {
                (&offer.peer, &offer.local)
            } else {
                continue;
            };
            if local.candidate.socket.instance.instance_id != instance
                || local.candidate.socket.socket_id != channel.socket_id.to_string()
                || peer.candidate.socket.node_id != channel.peer_node_id
                || peer.address.parse::<std::net::SocketAddr>()? != channel.remote_address.get()
                || peer.source != EndpointKind::Stun
                || offer.carrier.protocol != channel.transport
            {
                continue;
            }
            let mut active = false;
            let both = [&offer.local, &offer.peer].into_iter().all(|participant| {
                let mut accepted_by_participant = false;
                for event in &observations.events {
                    let Event::Accept {
                        request,
                        response,
                        received_at_ms,
                    } = event
                    else {
                        continue;
                    };
                    let (accepted, is_active) = match response {
                        AcceptUdpTraversalResponse::Waiting { offer } => (offer, false),
                        AcceptUdpTraversalResponse::Active { offer } => (offer, true),
                        _ => continue,
                    };
                    let matches = accepted == offer
                        && request.instance == participant.candidate.socket.instance
                        && request.netmap == participant.candidate.socket.netmap
                        && request.version == offer.version
                        && request.digest == offer.digest
                        && request.state_epoch == offer.state_epoch
                        && *received_at_ms < offer.expires_at_ms;
                    active |= matches && is_active;
                    accepted_by_participant |= matches;
                }
                accepted_by_participant
            });
            if both && active {
                return Ok(offer.clone());
            }
        }
        Err("CLI remote endpoint/socket/attempt not bound to this run's authenticated offer".into())
    }

    fn fail(&self) {
        if let Ok(mut observations) = self.0.lock() {
            observations.failed = true;
        }
    }

    fn record(&self, path: &str, input: &[u8], output: &[u8]) -> Result<()> {
        let event = if path == "/agent-api/v1/state" {
            let request: StateReport = serde_json::from_slice(input)?;
            let response: StateAccepted = serde_json::from_slice(output)?;
            if response.sequence != request.sequence {
                return Err("State publication acknowledgement sequence mismatch".into());
            }
            if response.duplicate || request.state.sockets.is_empty() {
                return Ok(());
            }
            Event::Publication {
                instance_id: request.instance_id,
                sequence: request.sequence,
                applied: request.state.configuration.applied,
                sockets: request.state.sockets,
            }
        } else if path == format!("{PREFIX}query") {
            Event::Query {
                request: serde_json::from_slice(input)?,
                response: serde_json::from_slice(output)?,
            }
        } else if path == format!("{PREFIX}attempts") {
            Event::Prepare {
                request: serde_json::from_slice(input)?,
                response: serde_json::from_slice(output)?,
            }
        } else if path.ends_with("/accept") {
            Event::Accept {
                request: serde_json::from_slice(input)?,
                response: serde_json::from_slice(output)?,
                received_at_ms: now_ms()?,
            }
        } else if path.ends_with("/cancel") {
            Event::Cancel {
                request: serde_json::from_slice(input)?,
                response: serde_json::from_slice(output)?,
            }
        } else {
            return Err("unknown traversal HTTP operation".into());
        };
        let mut observations = self.0.lock().map_err(|_| "traversal audit poisoned")?;
        let duplicate = match &event {
            Event::Publication { instance_id, applied, sockets, .. } => observations.events.iter().any(|event| matches!(event,
                Event::Publication { instance_id: previous, applied: configuration, sockets: previous_sockets, .. }
                if previous == instance_id && configuration == applied && previous_sockets == sockets)),
        // Keep the first actual metadata response for each instance. Subsequent
        // queries do not create additional proof or extend an observation.
            Event::Query { request, response: QueryUdpTraversalResponse::Current { .. } } => observations.events.iter().any(|event| matches!(event,
                Event::Query { request: previous, response: QueryUdpTraversalResponse::Current { .. } }
                if previous.instance == request.instance)),
            Event::Query { .. } => true,
            _ => false,
        };
        if duplicate {
            return Ok(());
        }
        let size = serde_json::to_vec(&event)?.len();
        if observations.events.len() >= EVENT_LIMIT || observations.bytes + size > 32 * 1024 * 1024
        {
            return Err("traversal audit bound exceeded".into());
        }
        observations.bytes += size;
        observations.events.push(event);
        Ok(())
    }

    pub async fn verify(&self, cli: &Cli, nodes: &BTreeMap<Role, NodeId>) -> Result<()> {
        let observer = format!("{}:3478", signals::underlay(cli).await?.controller);
        let signals = signals::Signals::new(&cli.signals);
        let mut reports = BTreeMap::new();
        for role in Role::ALL {
            let path = cli
                .evidence
                .join(format!("state-initial-carrier-{}.json", role.name()));
            if tokio::fs::metadata(&path).await?.len() > BODY_LIMIT as u64 {
                return Err("traversal State evidence exceeds bound".into());
            }
            let view = serde_json::from_slice(&tokio::fs::read(path).await?)?;
            let signals::Signal::Installations { evidence } = signals
                .wait(&format!("traversal-installations-{}", role.name()))
                .await?
            else {
                return Err("missing typed actor installation evidence".into());
            };
            if evidence.role != role {
                return Err("installation evidence actor mismatch".into());
            }
            reports.insert(
                nodes[&role],
                verification::Current {
                    view,
                    installations: *evidence,
                },
            );
        }
        let (bytes, continuity) = {
            let observations = self.0.lock().map_err(|_| "traversal audit poisoned")?;
            let (offers, continuity) =
                verification::verify(&observations, &reports, &observer, nodes)?;
            (
                serde_json::to_vec_pretty(&offers)?,
                serde_json::to_vec_pretty(&continuity)?,
            )
        };
        tokio::fs::write(cli.evidence.join("traversal-verified-offers.json"), bytes).await?;
        tokio::fs::write(
            cli.evidence.join("traversal-channel-continuity.json"),
            continuity,
        )
        .await?;
        Ok(())
    }

    pub async fn save(&self, cli: &Cli) -> Result<()> {
        if cli.traversal != crate::TraversalMode::Nat {
            return Ok(());
        }
        let bytes =
            serde_json::to_vec_pretty(&*self.0.lock().map_err(|_| "traversal audit poisoned")?)?;
        tokio::fs::write(cli.evidence.join("traversal-http.json"), bytes).await?;
        Ok(())
    }
}

pub async fn verify_state(cli: &Cli, role: Role, report: &NodeState) -> Result<()> {
    let underlay = signals::underlay(cli).await?;
    let observer = format!("{}:3478", underlay.controller);
    let expected_ip = underlay
        .routers
        .get(&role)
        .map_or(underlay.actors[&role], |router| router.wan);
    let expected = std::net::SocketAddr::new(expected_ip.into(), 35000);
    let now = i64::try_from(now_ms()?)?;
    for links in report.links.values() {
        for link in links.iter().filter(|link| {
            link.phase == LinkPhase::Authenticated
                && link.business_dependencies > 0
                && link.transport == meshway_transport_spec::TransportProtocol::NativeUdp.as_str()
        }) {
            let socket = report
                .sockets
                .get(&link.socket_id)
                .ok_or("authenticated link has no physical socket State")?;
            if socket.network != SocketNetwork::Udp
                || socket.bound_address != "0.0.0.0:35000".parse::<std::net::SocketAddr>()?
                || socket.endpoints.iter().any(|candidate| !matches!(candidate.origin, EndpointSource::Stun { .. }))
                || !socket.endpoints.iter().any(|candidate| {
                    matches!(&candidate.origin, EndpointSource::Stun { observer: actual } if actual == &observer)
                        && candidate.address.parse::<std::net::SocketAddr>().ok() == Some(expected)
                        && candidate.observed_at.timestamp_millis() <= now
                        && candidate.expires_at.timestamp_millis() > now
                        && candidate.observed_at < candidate.expires_at
                }) {
                return Err("link socket lacks original live IPv4 STUN mapping or has seeded candidates".into());
            }
        }
    }
    let path = cli
        .evidence
        .join(format!("traversal-state-first-{}.json", role.name()));
    if !tokio::fs::try_exists(&path).await? {
        tokio::fs::write(path, serde_json::to_vec_pretty(report)?).await?;
    }
    Ok(())
}
