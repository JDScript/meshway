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

const BODY_LIMIT: usize = 256 * 1024;
const EVENT_LIMIT: usize = 4096;
const PREFIX: &str = "/agent-api/v1/connectivity/udp/";

#[derive(Clone, Default)]
pub struct Audit(Arc<Mutex<Observations>>);

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Observations {
    // Every entry came through the production nodeSession-only authenticated
    // route with a successful response; Authorization is never copied.
    events: Vec<Event>,
    failed: bool,
    bytes: usize,
}

#[derive(Serialize)]
#[serde(
    tag = "operation",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Event {
    Publication {
        instance_id: meshway_model::InstanceId,
        sequence: meshway_model::Sequence,
        applied: Option<meshway_model::ConfigurationRef>,
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
        let mut reports = BTreeMap::new();
        for role in Role::ALL {
            let path = cli
                .evidence
                .join(format!("traversal-state-first-{}.json", role.name()));
            if tokio::fs::metadata(&path).await?.len() > BODY_LIMIT as u64 {
                return Err("traversal State evidence exceeds bound".into());
            }
            let report: NodeState = serde_json::from_slice(&tokio::fs::read(path).await?)?;
            reports.insert(nodes[&role], report);
        }
        let bytes = {
            let observations = self.0.lock().map_err(|_| "traversal audit poisoned")?;
            if observations.failed {
                return Err("traversal HTTP audit incomplete".into());
            }
            let mut verified = Vec::new();
            for (left, right) in [(Role::A, Role::R), (Role::R, Role::B)] {
                let complete: Vec<_> = observations.events.iter().filter(|event| {
                let Event::Prepare { request, response: PrepareUdpTraversalResponse::Offer { offer } } = event else { return false; };
                let local = &offer.local.candidate.socket;
                let peer = &offer.peer.candidate.socket;
                if !((local.node_id == nodes[&left] && peer.node_id == nodes[&right])
                    || (local.node_id == nodes[&right] && peer.node_id == nodes[&left]))
                    || offer.compute_digest().ok() != Some(offer.digest)
                    || request.local_candidate != offer.local.candidate
                    || request.peer_candidate != offer.peer.candidate
                    || request.instance != local.instance
                    || request.request_id != offer.request_id
                    || offer.carrier.protocol != meshway_transport_spec::TransportProtocol::NativeUdp
                    || local.instance.instance_id == peer.instance.instance_id
                    || offer.expires_at_ms > request.expires_at_ms {
                    return false;
                }
                let mut active = false;
                let both = [&offer.local, &offer.peer].iter().all(|participant| {
                    let socket = &participant.candidate.socket;
                    let other = if socket.node_id == local.node_id { peer.node_id } else { local.node_id };
                    let live = reports.get(&socket.node_id).is_some_and(|report| {
                        report.sockets.get(&socket.socket_id).is_some_and(|physical| {
                            physical.generation == socket.socket_generation && physical.resource == socket.resource
                                && physical.configuration.as_ref().or(report.configuration.applied.as_ref()) == Some(&socket.configuration)
                        }) && report.links.get(&other).is_some_and(|links| links.iter().any(|link| {
                            link.phase == LinkPhase::Authenticated && link.business_dependencies > 0
                                && link.socket_id == socket.socket_id && link.transport == meshway_transport_spec::TransportProtocol::NativeUdp.as_str()
                        }))
                    });
                    let publication = observations.events.iter().any(|event| {
                        let Event::Publication { instance_id, applied, sockets, .. } = event else { return false; };
                        let Some(published) = sockets.get(&socket.socket_id) else { return false; };
                        *instance_id == socket.instance.instance_id
                            && published.configuration.as_ref().or(applied.as_ref()) == Some(&socket.configuration)
                            && published.generation == socket.socket_generation && published.resource == socket.resource
                            && published.network == SocketNetwork::Udp
                            && published.bound_address == "0.0.0.0:35000".parse::<std::net::SocketAddr>().expect("fixed address")
                            && published.endpoints.iter().any(|candidate| {
                                candidate.address == participant.address
                                    && matches!(&candidate.origin, EndpointSource::Stun { observer: actual } if actual == &observer)
                                    && candidate.expires_at.timestamp_millis() >= i64::try_from(offer.expires_at_ms).unwrap_or(i64::MAX)
                                    && CandidateReference::from_observation(socket.clone(), published.network, published.bound_address, candidate).ok().as_ref() == Some(&participant.candidate)
                            })
                    });
                    let metadata = observations.events.iter().any(|event| matches!(event,
                        Event::Query { request, response: QueryUdpTraversalResponse::Current { stun_servers, .. } }
                        if request.instance == socket.instance && stun_servers.as_slice().iter().any(|server| server.id == "fixture")));
                    let mut accepted_by_participant = false;
                    for event in &observations.events {
                        let Event::Accept { request, response, received_at_ms } = event else { continue; };
                        let (accepted, is_active) = match response {
                            AcceptUdpTraversalResponse::Waiting { offer } => (offer, false),
                            AcceptUdpTraversalResponse::Active { offer } => (offer, true),
                            _ => continue,
                        };
                        let matched = accepted == offer && request.instance == socket.instance
                            && request.configuration == socket.configuration
                            && request.state_epoch == offer.state_epoch
                            && request.version == offer.version && request.digest == offer.digest
                            && *received_at_ms < offer.expires_at_ms;
                        active |= matched && is_active;
                        accepted_by_participant |= matched;
                    }
                    live && publication && metadata && accepted_by_participant
                });
                both && active
            }).filter_map(|event| match event {
                Event::Prepare { response: PrepareUdpTraversalResponse::Offer { offer }, .. } => Some(offer.clone()),
                _ => None,
            }).collect();
                if complete.is_empty() {
                    return Err(
                        "missing fixed offer, two Node acceptances, or active traversal attempt"
                            .into(),
                    );
                }
                verified.extend(complete);
            }
            serde_json::to_vec_pretty(&verified)?
        };
        tokio::fs::write(cli.evidence.join("traversal-verified-offers.json"), bytes).await?;
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
