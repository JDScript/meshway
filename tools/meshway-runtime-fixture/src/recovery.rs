//! Fixture-only HTTP availability and typed recovery observations. The real
//! Server and daemon remain the only credential, lease and packet owners.
use crate::{
    Result,
    application::{Counts, PacketEvidence},
};
use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use meshway_ipc::{Phase, Status};
use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Notify;

struct GateState {
    available: bool,
    admitted: usize,
    rejected: u64,
}
struct Inner {
    state: Mutex<GateState>,
    drained: Notify,
}
#[derive(Clone)]
pub struct Availability(Arc<Inner>);
impl Default for Availability {
    fn default() -> Self {
        Self::new()
    }
}
impl Availability {
    pub fn new() -> Self {
        Self(Arc::new(Inner {
            state: Mutex::new(GateState {
                available: true,
                admitted: 0,
                rejected: 0,
            }),
            drained: Notify::new(),
        }))
    }
    pub async fn close(&self) -> Result<u64> {
        self.0
            .state
            .lock()
            .map_err(|_| "HTTP gate poisoned")?
            .available = false;
        // Drain admitted HTTP handlers before publishing the outage boundary.
        // Actors still sample the last delivered lease until actual expiry;
        // cancelled, accepted database work remains with the real Server owner.
        tokio::time::timeout(crate::remaining(Duration::from_secs(75))?, async {
            loop {
                let changed = self.0.drained.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if self
                    .0
                    .state
                    .lock()
                    .map_err(|_| "HTTP gate poisoned")?
                    .admitted
                    == 0
                {
                    return now_ms();
                }
                changed.await;
            }
        })
        .await?
    }
    pub fn open(&self) -> Result<(u64, u64)> {
        let mut state = self.0.state.lock().map_err(|_| "HTTP gate poisoned")?;
        if state.available || state.admitted != 0 || state.rejected == 0 {
            return Err("HTTP outage lacks rejected Node requests or a drained boundary".into());
        }
        let at = now_ms()?;
        state.available = true;
        Ok((at, state.rejected))
    }
}
struct Admission(Arc<Inner>);
impl Drop for Admission {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.admitted -= 1;
            if state.admitted == 0 {
                self.0.drained.notify_waiters();
            }
        }
    }
}
pub async fn gate(State(gate): State<Availability>, request: Request, next: Next) -> Response {
    if !request.uri().path().starts_with("/agent-api/v1/") {
        return next.run(request).await;
    }
    let admission = {
        let Ok(mut state) = gate.0.state.lock() else {
            return unavailable();
        };
        if !state.available {
            state.rejected = state.rejected.saturating_add(1);
            return unavailable();
        }
        state.admitted += 1;
        Admission(gate.0.clone())
    };
    let response = next.run(request).await;
    drop(admission);
    response
}
fn unavailable() -> Response {
    Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        .body(axum::body::Body::empty())
        .expect("fixed availability response")
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Expired {
    pub before: Box<Status>,
    pub status: Box<Status>,
    pub closed_at_ms: u64,
    pub instance_expires_at_ms: u64,
    pub netmap_expires_at_ms: u64,
    pub observed_at_ms: u64,
    pub old_traffic_blocked: Option<bool>,
}
impl Expired {
    pub fn validate(&self) -> Result<()> {
        let lease = self
            .before
            .instance
            .as_ref()
            .ok_or("missing pre-outage instance")?;
        if !self.before.desired_up
            || self.before.phase != Phase::Running
            || self.status.identity != self.before.identity
            || self.status.process_id != self.before.process_id
            || self.status.revision != self.before.revision
            || !self.status.desired_up
            || self.status.instance.is_some()
            || self.status.applied.is_some()
            || self.status.data_plane != meshway_ipc::DataPlane::Suspended
            || self.status.phase == Phase::Blocked
            || self.instance_expires_at_ms < lease.expires_at_ms
            || self.observed_at_ms <= self.instance_expires_at_ms
            || self.observed_at_ms <= self.netmap_expires_at_ms
            || self.before.netmap.as_ref().is_none_or(|netmap| {
                u64::try_from(netmap.expires_at.timestamp_millis())
                    .ok()
                    .is_none_or(|expiry| expiry > self.netmap_expires_at_ms)
            })
            || self.observed_at_ms <= self.closed_at_ms
        {
            return Err("outage did not expire both the actual instance and netmap lease".into());
        }
        Ok(())
    }
    pub fn validate_recovered(&self, after: &Status, reopened_at_ms: u64) -> Result<()> {
        self.validate()?;
        let before = &self.before;
        let previous = before
            .instance
            .as_ref()
            .ok_or("missing pre-outage instance")?;
        if reopened_at_ms < self.observed_at_ms
            || after.identity != before.identity
            || after.process_id != before.process_id
            || after.revision != before.revision
            || !after.desired_up
            || !after.authenticated
            || after.network_mode != before.network_mode
            || after.phase != Phase::Running
            || after.data_plane != meshway_ipc::DataPlane::Ready
            || after.applied.is_none()
            || after.applied != after.saved
            || after.instance.as_ref().is_none_or(|lease| {
                Some(lease.instance_id) == before.instance.as_ref().map(|old| old.instance_id)
                    || lease.generation.get() <= previous.generation.get()
                    || lease.expires_at_ms <= reopened_at_ms
            })
        {
            return Err(
                "automatic recovery changed operator identity/intent or reused expired authority"
                    .into(),
            );
        }
        Ok(())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    pub closed_at_ms: u64,
    pub reopened_at_ms: u64,
    pub rejected_node_requests: u64,
    pub response_status: u16,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Evidence {
    pub expired: Expired,
    pub recovered: Box<Status>,
    pub reopened_at_ms: u64,
    pub old_tcp_blocked_after_recovery: Option<bool>,
    pub old_sockets: Option<[std::net::SocketAddr; 4]>,
    pub old_circuits: Vec<String>,
    pub new_circuits: Vec<String>,
    pub traffic: Option<PacketEvidence>,
    pub delivered: Counts,
    pub relay_tun_received: Option<u64>,
}
impl Evidence {
    pub fn validate(&self, role: crate::signals::Role) -> Result<()> {
        self.expired
            .validate_recovered(&self.recovered, self.reopened_at_ms)?;
        match role {
            crate::signals::Role::A => {
                let packet = self
                    .traffic
                    .as_ref()
                    .ok_or("missing new OS flow evidence")?;
                let old = self
                    .old_sockets
                    .as_ref()
                    .ok_or("missing old OS socket identities")?;
                if self.expired.old_traffic_blocked != Some(true)
                    || self.old_tcp_blocked_after_recovery != Some(true)
                    || !packet.new_flows_after(old)
                    || self.old_circuits.is_empty()
                    || self.new_circuits.is_empty()
                    || self
                        .new_circuits
                        .iter()
                        .any(|cid| self.old_circuits.contains(cid))
                {
                    return Err(
                        "recovery lacks old-flow rejection or fresh socket/circuit evidence".into(),
                    );
                }
            }
            crate::signals::Role::B if !self.delivered.all_protocols() => {
                return Err("terminal did not receive new dual-stack UDP/TCP flows".into());
            }
            crate::signals::Role::R
                if self.relay_tun_received != Some(0) || !self.delivered.empty() =>
            {
                return Err("recovered relay delivered business plaintext".into());
            }
            _ => {}
        }
        Ok(())
    }
}

pub fn now_ms() -> Result<u64> {
    use std::time::{SystemTime, UNIX_EPOCH};
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis()
        .try_into()?)
}

/// Observed deadlines only: the instance qualifies reports and the netmap
/// lease bounds execution. Neither a read nor a missing status extends them.
pub(super) fn outage_deadlines(
    before: &meshway_protocol::InstanceLease,
    latest: Option<&meshway_protocol::InstanceLease>,
    netmap: &meshway_ipc::NetmapStatus,
) -> Result<(u64, u64)> {
    if latest.is_some_and(|lease| {
        lease.instance_id != before.instance_id || lease.generation != before.generation
    }) {
        return Err("pre-outage instance changed while sampling deadlines".into());
    }
    let instance_expiry = latest.map_or(before.expires_at_ms, |lease| {
        lease.expires_at_ms.max(before.expires_at_ms)
    });
    let execution_expiry = u64::try_from(netmap.expires_at.timestamp_millis())?;
    Ok((instance_expiry, execution_expiry))
}

pub async fn circuit_ids(
    cli: &crate::Cli,
    path: &[meshway_model::NodeId],
    after: Option<u64>,
) -> Result<Vec<String>> {
    let mut ids = std::collections::BTreeSet::new();
    for event in crate::actor::log_events(cli).await? {
        let Some(packet) = crate::fallback::packet_observation(&event)? else {
            continue;
        };
        if after.is_some_and(|after| packet.observed_at_ms < after) {
            continue;
        }
        let observed: Vec<meshway_model::NodeId> = serde_json::from_str(&packet.path_json)?;
        if observed != path {
            return Err("recovery packet bypassed the original complete path".into());
        }
        ids.insert(packet.cid.to_ascii_lowercase());
    }
    Ok(ids.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::get};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        sync::oneshot,
    };

    #[test]
    fn netmap_and_instance_deadlines_remain_independent_during_an_outage() {
        let before = meshway_protocol::InstanceLease {
            instance_id: meshway_model::InstanceId::new(uuid::Uuid::from_u128(1)).unwrap(),
            generation: meshway_model::Version::MIN,
            expires_at_ms: 120_000,
        };
        let latest = meshway_protocol::InstanceLease {
            expires_at_ms: 140_000,
            ..before.clone()
        };
        let netmap = meshway_ipc::NetmapStatus {
            netmap: meshway_model::NetmapRef {
                revision: meshway_model::Revision::MIN,
                serial: meshway_model::Version::MIN,
                digest: [1; 32].into(),
            },
            issued_at: "1970-01-01T00:00:01Z".parse().unwrap(),
            expires_at: "1970-01-01T00:05:01Z".parse().unwrap(),
        };
        assert_eq!(
            outage_deadlines(&before, Some(&latest), &netmap).unwrap(),
            (140_000, 301_000)
        );
        assert_eq!(
            outage_deadlines(&before, None, &netmap).unwrap(),
            (120_000, 301_000)
        );
        let replaced = meshway_protocol::InstanceLease {
            instance_id: meshway_model::InstanceId::new(uuid::Uuid::from_u128(2)).unwrap(),
            ..latest.clone()
        };
        assert!(outage_deadlines(&before, Some(&replaced), &netmap).is_err());
    }

    async fn request(address: std::net::SocketAddr, path: &str) -> String {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
            socket
                .write_all(
                    format!("GET {path} HTTP/1.1\r\nHost: fixture\r\nConnection: close\r\n\r\n")
                        .as_bytes(),
                )
                .await
                .unwrap();
            let mut response = String::new();
            socket.read_to_string(&mut response).await.unwrap();
            response
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn gate_drains_admitted_http_and_keeps_management_available() {
        let availability = Availability::new();
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let router = Router::new()
            .route(
                "/agent-api/v1/held",
                get({
                    let entered = entered.clone();
                    let release = release.clone();
                    move || {
                        let entered = entered.clone();
                        let release = release.clone();
                        async move {
                            entered.notify_one();
                            release.notified().await;
                            "node"
                        }
                    }
                }),
            )
            .route("/agent-api/v1/probe", get(|| async { "node" }))
            .route("/api/v1/probe", get(|| async { "management" }))
            .layer(axum::middleware::from_fn_with_state(
                availability.clone(),
                gate,
            ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, stopped) = oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        let held = tokio::spawn(request(address, "/agent-api/v1/held"));
        tokio::time::timeout(std::time::Duration::from_secs(3), entered.notified())
            .await
            .unwrap();
        let closed = availability.close();
        tokio::pin!(closed);
        tokio::select! {
            _ = &mut closed => panic!("gate returned before the admitted handler drained"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {},
        }
        assert!(
            request(address, "/agent-api/v1/probe")
                .await
                .starts_with("HTTP/1.1 503")
        );
        assert!(
            request(address, "/api/v1/probe")
                .await
                .starts_with("HTTP/1.1 200")
        );
        release.notify_one();
        assert!(held.await.unwrap().starts_with("HTTP/1.1 200"));
        let before = closed.await.unwrap();
        let (after, rejected) = availability.open().unwrap();
        assert!(after >= before && rejected == 1);
        assert!(
            request(address, "/agent-api/v1/probe")
                .await
                .starts_with("HTTP/1.1 200")
        );
        stop.send(()).unwrap();
        server.await.unwrap();
    }
}
