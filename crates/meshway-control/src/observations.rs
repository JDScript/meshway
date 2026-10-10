//! Observation RPCs share the held-key Control session. A subscription response
//! is feedback, never a permit: a task runs only while the source's own
//! netmap decides it (`meshway_policy::compile::netmap::observation_paths`).
use crate::{
    Control, Deadline, Error, Started,
    exchange::{Prepared, Response},
};
use meshway_control_client::{ClientError, *};
use meshway_model::{Digest, NetmapRef, ObservationSubscriptionId};
use meshway_protocol::{observations::*, *};
use std::collections::BTreeMap;

const LIMIT: usize = 128;

#[derive(Default)]
pub(super) struct Registry {
    creates: BTreeMap<ObservationSubscriptionId, Creation>,
    subscriptions: BTreeMap<ObservationSubscriptionId, NodeObservationResponse>,
    renewals: BTreeMap<ObservationSubscriptionId, Digest>,
    leases: BTreeMap<ObservationSubscriptionId, ObservationLease>,
}

/// Confirmed local subscription lifetime, independent of the netmap or another
/// subscriber's task lease. This does not authorize I/O.
#[derive(Clone)]
pub struct ObservationLease {
    instance: InstanceRequest,
    deadline: Deadline,
    cancelled: tokio_util::sync::CancellationToken,
}
impl ObservationLease {
    pub fn is_current(&self) -> bool {
        !self.is_cancelled() && self.deadline.valid()
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.is_cancelled()
    }
    pub fn is_expired(&self) -> bool {
        !self.deadline.valid()
    }
    pub fn expires_at(&self) -> std::time::Instant {
        self.deadline.expires_at()
    }
    pub fn expires_at_ms(&self) -> u64 {
        self.deadline.expires_at_ms()
    }
    pub async fn ended(&self) {
        self.deadline.ended(&self.cancelled).await;
    }
}
impl Drop for Registry {
    fn drop(&mut self) {
        for lease in self.leases.values() {
            lease.cancelled.cancel();
        }
    }
}
struct Creation {
    digest: Digest,
    interval_ms: u32,
}

impl Control {
    pub fn node_observation_lease(
        &self,
        id: ObservationSubscriptionId,
    ) -> Result<ObservationLease, Error> {
        let instance = self.current_instance()?;
        let lease = self.observations.leases.get(&id).ok_or(Error::Lifecycle)?;
        if lease.instance != instance {
            return Err(Error::Lifecycle);
        }
        if !lease.is_current() {
            return Err(Error::Expired);
        }
        Ok(lease.clone())
    }

    fn confirm_observation_lease(
        &mut self,
        id: ObservationSubscriptionId,
        value: &NodeObservationResponse,
        started: Started,
    ) -> Result<(), Error> {
        if !matches!(
            value.task.phase,
            ObservationPhase::Preparing | ObservationPhase::Running
        ) {
            if let Some(lease) = self.observations.leases.remove(&id) {
                lease.cancelled.cancel();
            }
            return Ok(());
        }
        let expires = value
            .task
            .subscriber_expires_at
            .ok_or(Error::Contract)?
            .timestamp_millis();
        let expires = u64::try_from(expires).map_err(|_| Error::Contract)?;
        let instance = self.current_instance()?;
        let previous = self.observations.leases.get(&id);
        if let Some(previous) = previous {
            if previous.instance != instance || !previous.is_current() {
                return Err(Error::Expired);
            }
            // Identical/replayed confirmations keep the first monotonic bound.
            if previous.expires_at_ms() == expires {
                return Ok(());
            }
        } else if self.observations.leases.len() >= LIMIT {
            return Err(Error::Pending);
        }
        let cancelled = previous.map_or_else(tokio_util::sync::CancellationToken::new, |old| {
            old.cancelled.clone()
        });
        let deadline = Deadline::from_response(started, expires)?;
        self.observations.leases.insert(
            id,
            ObservationLease {
                instance,
                deadline,
                cancelled,
            },
        );
        Ok(())
    }
    // A denied target or expired subscription must not tear down the business
    // session. Task endpoints also use 401 for a stale task scope, so credential
    // lifecycle is decided by the normal authentication/lease gateway instead.
    fn observation_response<T>(
        &mut self,
        result: Result<T, ClientError<ErrorResponse>>,
    ) -> Result<T, Error> {
        result.map_err(Error::Server)
    }

    fn observation_task(&self, task: &ObservationTask) -> Result<(), Error> {
        let instance = self.current_instance()?;
        let source = self
            .record
            .binding
            .as_ref()
            .ok_or(Error::Lifecycle)?
            .node_id;
        let expiry =
            u64::try_from(task.expires_at.timestamp_millis()).map_err(|_| Error::Contract)?;
        if task.source_node_id != source
            || task.instance_id != instance.instance_id
            || task.source_node_id == task.target_node_id
            || !(1000..=30_000).contains(&task.interval_ms)
            || task.metrics.is_empty()
            || task
                .metrics
                .iter()
                .enumerate()
                .any(|(i, m)| task.metrics[..i].contains(m))
            || task
                .unsupported_metrics
                .iter()
                .any(|m| !task.metrics.contains(m))
            || task.renew_after > task.expires_at
            || task
                .subscriber_expires_at
                .is_some_and(|at| at > task.expires_at)
            || task
                .path
                .as_ref()
                .is_some_and(|path| !task_path(task, path))
            || (task.path.is_some() && task.path_segment.is_none())
            || (task.phase == ObservationPhase::Running && task.path.is_none())
            || (task.phase == ObservationPhase::Preparing && task.path.is_some())
            || task.path_segment.is_some_and(|segment| segment.get() == 0)
            || self
                .instance
                .as_ref()
                .is_none_or(|instance| expiry > instance.lease.expires_at_ms)
        {
            return Err(Error::Contract);
        }
        if matches!(
            task.phase,
            ObservationPhase::Preparing | ObservationPhase::Running
        ) && expiry <= self.clock.now()?
        {
            return Err(Error::Expired);
        }
        Ok(())
    }

    /// Observation results speak for the held, leased netmap `reference`.
    fn observation_netmap(
        &self,
        reference: NetmapRef,
    ) -> Result<std::sync::Arc<meshway_configuration::netmap::CompiledNetmap>, Error> {
        self.current_instance()?;
        self.held_netmap(reference)
    }

    fn subscription_response(
        &self,
        id: ObservationSubscriptionId,
        value: &NodeObservationResponse,
    ) -> Result<(), Error> {
        self.observation_task(&value.task)?;
        subscription_fields(
            id,
            self.observations
                .creates
                .get(&id)
                .map(|request| request.interval_ms),
            value,
        )?;
        if let Some(old) = self.observations.subscriptions.get(&id)
            && (old.task.id != value.task.id
                || old.task.target_node_id != value.task.target_node_id
                || old.task.service != value.task.service
                || old.task.selection != value.task.selection
                || old.task.metrics != value.task.metrics)
        {
            return Err(Error::Contract);
        }
        Ok(())
    }

    pub async fn create_node_observation(
        &mut self,
        request: &CreateNodeObservation,
    ) -> Result<NodeObservationResponse, Error> {
        if request.instance != self.current_instance()? {
            return Err(Error::Lifecycle);
        }
        let digest =
            meshway_protocol::node::request_digest(request).map_err(|_| Error::Contract)?;
        match self.observations.creates.get(&request.subscription_id) {
            Some(old) if old.digest != digest => return Err(Error::Contract),
            None if self.observations.creates.len() >= LIMIT => return Err(Error::Pending),
            _ => {}
        }
        self.observations.creates.insert(
            request.subscription_id,
            Creation {
                digest,
                interval_ms: request.interval_ms,
            },
        );
        let started = Started::now(&self.clock)?;
        let response = self
            .client
            .create_node_observation(request, self.token()?)
            .await;
        if matches!(
            &response,
            Err(ClientError::Http {
                status: 400 | 401 | 403 | 404 | 409 | 422,
                ..
            })
        ) {
            self.observations.creates.remove(&request.subscription_id);
        }
        let value = self.observation_response(response)?;
        self.subscription_response(request.subscription_id, &value)?;
        let target_matches = if let Some(previous) = self
            .observations
            .subscriptions
            .get(&request.subscription_id)
        {
            // A successfully bound selector remains pinned to its NodeId when
            // later views refresh the hostname or assigned Mesh addresses.
            previous.target.node_id == value.target.node_id
        } else {
            match &request.target {
                NodeObservationTarget::Name { name } => {
                    let name = name.trim_end_matches('.').to_ascii_lowercase();
                    let hostname = value.target.hostname.to_ascii_lowercase();
                    let domain = self
                        .netmap_base()
                        .map(|netmap| netmap.netmap.projection.configuration.dns.domain.as_str());
                    name == hostname
                        || domain.is_some_and(|domain| name == format!("{hostname}.{domain}"))
                }
                NodeObservationTarget::MeshIp { address } => address
                    .parse::<std::net::IpAddr>()
                    .ok()
                    .is_some_and(|ip| match ip {
                        std::net::IpAddr::V4(ip) => value.target.addresses.ipv4 == Some(ip),
                        std::net::IpAddr::V6(ip) => value.target.addresses.ipv6 == Some(ip),
                    }),
            }
        };
        if !target_matches
            || value.task.service != request.service
            || value.task.selection != request.selection
            || value.task.metrics != request.metrics
        {
            return Err(Error::Contract);
        }
        self.confirm_observation_lease(request.subscription_id, &value, started)?;
        self.observations
            .subscriptions
            .insert(request.subscription_id, value.clone());
        Ok(value)
    }

    pub async fn get_node_observation(
        &mut self,
        subscription_id: ObservationSubscriptionId,
        query: &NodeObservationQuery,
    ) -> Result<NodeObservationResponse, Error> {
        let instance = self.current_instance()?;
        if query.instance_id != instance.instance_id || query.generation != instance.generation {
            return Err(Error::Lifecycle);
        }
        let started = Started::now(&self.clock)?;
        let result = self
            .client
            .get_node_observation(
                &GetNodeObservationPath { subscription_id },
                &GetNodeObservationQuery {
                    instance_id: query.instance_id,
                    generation: query.generation,
                },
                self.token()?,
            )
            .await;
        let value = self.observation_response(result)?;
        self.subscription_response(subscription_id, &value)?;
        self.confirm_observation_lease(subscription_id, &value, started)?;
        Ok(value)
    }

    pub async fn renew_node_observation(
        &mut self,
        subscription_id: ObservationSubscriptionId,
        request: &RenewNodeObservation,
    ) -> Result<NodeObservationResponse, Error> {
        if request.instance != self.current_instance()? {
            return Err(Error::Lifecycle);
        }
        if !self
            .observations
            .subscriptions
            .contains_key(&subscription_id)
        {
            return Err(Error::Lifecycle);
        }
        let digest =
            meshway_protocol::node::request_digest(request).map_err(|_| Error::Contract)?;
        if self
            .observations
            .renewals
            .get(&subscription_id)
            .is_some_and(|old| *old != digest)
        {
            return Err(Error::Contract);
        }
        self.observations.renewals.insert(subscription_id, digest);
        let started = Started::now(&self.clock)?;
        let result = self
            .client
            .renew_node_observation(
                &RenewNodeObservationPath { subscription_id },
                request,
                self.token()?,
            )
            .await;
        let value = self.observation_response(result)?;
        self.subscription_response(subscription_id, &value)?;
        if value.task.subscriber_expires_at.is_none_or(|at| {
            at.timestamp_millis() > i64::try_from(request.expires_at_ms).unwrap_or(i64::MAX)
        }) {
            return Err(Error::Contract);
        }
        self.confirm_observation_lease(subscription_id, &value, started)?;
        self.observations
            .subscriptions
            .insert(subscription_id, value.clone());
        self.observations.renewals.remove(&subscription_id);
        Ok(value)
    }

    pub async fn cancel_node_observation(
        &mut self,
        subscription_id: ObservationSubscriptionId,
        request: &InstanceRequest,
    ) -> Result<NodeObservationCancelled, Error> {
        if *request != self.current_instance()? {
            return Err(Error::Lifecycle);
        }
        if let Some(lease) = self.observations.leases.remove(&subscription_id) {
            lease.cancelled.cancel();
        }
        let result = self
            .client
            .cancel_node_observation(
                &CancelNodeObservationPath { subscription_id },
                request,
                self.token()?,
            )
            .await;
        if matches!(
            &result,
            Err(ClientError::Http {
                status: 400 | 401 | 403 | 404 | 409 | 422,
                ..
            })
        ) {
            self.observations.creates.remove(&subscription_id);
            self.observations.subscriptions.remove(&subscription_id);
            self.observations.renewals.remove(&subscription_id);
        }
        let value = self.observation_response(result)?;
        self.current_instance()?;
        if let Some(task) = &value.task {
            self.observation_task(task)?;
            if self
                .observations
                .subscriptions
                .get(&subscription_id)
                .is_some_and(|old| {
                    old.task.id != task.id
                        || old.task.target_node_id != task.target_node_id
                        || old.task.service != task.service
                })
                || task.subscription_id.is_some()
                || task.subscriber_expires_at.is_some()
            {
                return Err(Error::Contract);
            }
        }
        self.observations.creates.remove(&subscription_id);
        self.observations.subscriptions.remove(&subscription_id);
        self.observations.renewals.remove(&subscription_id);
        Ok(value)
    }

    pub async fn get_observation_tasks(
        &mut self,
        netmap: NetmapRef,
    ) -> Result<ObservationList, Error> {
        let prepared = self.prepare_observation_tasks(netmap)?;
        self.exchange(prepared, Self::apply_observation_tasks).await
    }

    pub(crate) fn prepare_observation_tasks(
        &mut self,
        netmap: NetmapRef,
    ) -> Result<Prepared<NetmapRef, ObservationList>, Error> {
        self.observation_netmap(netmap)?;
        self.prepare(netmap, |client, token| async move {
            client.get_node_tasks(&token).await
        })
    }

    pub(crate) fn apply_observation_tasks(
        &mut self,
        netmap: NetmapRef,
        result: Response<ObservationList>,
    ) -> Result<ObservationList, Error> {
        let value = self.observation_response(result)?;
        self.observation_netmap(netmap)?;
        if value.items.len() > LIMIT {
            return Err(Error::Contract);
        }
        for (index, task) in value.items.iter().enumerate() {
            self.observation_task(task)?;
            if task.subscription_id.is_some()
                || task.subscriber_expires_at.is_some()
                || value.items[..index].iter().any(|old| old.id == task.id)
            {
                return Err(Error::Contract);
            }
        }
        Ok(value)
    }

    pub async fn report_observation_result(
        &mut self,
        task: &ObservationTask,
        result: &ObservationResult,
    ) -> Result<ObservationTask, Error> {
        let prepared = self.prepare_observation_result(task, result)?;
        self.exchange(prepared, Self::apply_observation_result)
            .await
    }

    /// Structural checks only: the path was bound and measured by the
    /// dataplane on the installed content, which decides observability.
    pub(crate) fn prepare_observation_result(
        &mut self,
        task: &ObservationTask,
        result: &ObservationResult,
    ) -> Result<Prepared<(ObservationTask, ObservationResult), ObservationTask>, Error> {
        self.observation_netmap(result.netmap)?;
        self.observation_task(task)?;
        if result.instance_id != self.current_instance()?.instance_id {
            return Err(Error::Contract);
        }
        if let ObservationUpdate::Sample {
            path,
            path_segment,
            sample,
        } = &result.result
            && (!task_path(task, path)
                || path_segment.get() == 0
                || task.path_segment.is_some_and(|high| {
                    *path_segment < high
                        || (*path_segment == high && task.path.as_ref() != Some(path))
                })
                || sample.partial
                || sample.window_start >= sample.window_end
                || sample.window_end > task.expires_at
                || sample.traffic.is_some()
                || sample.probe.as_ref().is_some_and(|probe| {
                    probe.measurement_id != sample.measurement_id
                        || probe.path != *path
                        || probe.netmap != result.netmap
                }))
        {
            return Err(Error::Contract);
        }
        let (path, body) = (ReportTaskResultPath { task_id: task.id }, result.clone());
        self.prepare((task.clone(), result.clone()), move |client, token| async move {
            client.report_task_result(&path, &body, &token).await
        })
    }

    pub(crate) fn apply_observation_result(
        &mut self,
        (task, result): (ObservationTask, ObservationResult),
        response: Response<ObservationTask>,
    ) -> Result<ObservationTask, Error> {
        let (task, result) = (&task, &result);
        let value = self.observation_response(response)?;
        self.observation_netmap(result.netmap)?;
        self.observation_task(&value)?;
        if value.id != task.id
            || value.target_node_id != task.target_node_id
            || value.service != task.service
            || value.selection != task.selection
            || value.metrics != task.metrics
        {
            return Err(Error::Contract);
        }
        if let ObservationUpdate::Sample {
            path,
            path_segment,
            sample,
        } = &result.result
            && (value.latest.as_ref() != Some(sample.as_ref())
                || value.path.as_ref() != Some(path)
                || value.path_segment != Some(*path_segment))
        {
            return Err(Error::Contract);
        }
        Ok(value)
    }
}

fn task_path(task: &ObservationTask, path: &[meshway_model::NodeId]) -> bool {
    path.len() >= 2
        && path.len() <= 32
        && path.first() == Some(&task.source_node_id)
        && path.last() == Some(&task.target_node_id)
        && !path
            .iter()
            .enumerate()
            .any(|(i, node)| path[..i].contains(node))
        && match &task.selection {
            ObservationPath::Auto => true,
            ObservationPath::Explicit { path: selected } => selected == path,
        }
}

/// Validate subscription feedback independently of execution permits: an
/// Unsupported response intentionally has no DiagnosticPermit to install.
fn subscription_fields(
    id: ObservationSubscriptionId,
    requested_interval_ms: Option<u32>,
    value: &NodeObservationResponse,
) -> Result<(), Error> {
    if value.task.subscription_id != Some(id)
        || value.task.subscriber_expires_at.is_none()
        || value.target.node_id != value.task.target_node_id
        || !meshway_configuration::valid_node_hostname(&value.target.hostname)
        || requested_interval_ms.is_some_and(|minimum| value.task.interval_ms < minimum)
    {
        return Err(Error::Contract);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response() -> NodeObservationResponse {
        let id = "00000000-0000-4000-8000-000000000001";
        let target = "00000000-0000-4000-8000-000000000002".parse().unwrap();
        let now = chrono::Utc::now();
        NodeObservationResponse {
            task: ObservationTask {
                id: id.parse().unwrap(),
                source_node_id: id.parse().unwrap(),
                target_node_id: target,
                service: meshway_configuration::execution::ObservationServiceScope::IcmpEcho,
                selection: ObservationPath::Auto,
                path: None,
                path_segment: None,
                netmap: meshway_model::NetmapRef {
                    revision: meshway_model::Revision::MIN,
                    serial: meshway_model::Version::MIN,
                    digest: [1; 32].into(),
                },
                instance_id: id.parse().unwrap(),
                metrics: vec![meshway_model::observations::MetricKind::Rtt],
                interval_ms: 2000,
                phase: ObservationPhase::Preparing,
                unsupported_metrics: Vec::new(),
                expires_at: now + chrono::Duration::seconds(30),
                renew_after: now + chrono::Duration::seconds(10),
                latest: None,
                subscriber_expires_at: Some(now + chrono::Duration::seconds(30)),
                subscription_id: Some(id.parse().unwrap()),
            },
            target: NodeObservationTargetView {
                node_id: target,
                hostname: "probe-target".into(),
                addresses: meshway_configuration::NodeAddresses {
                    ipv4: None,
                    ipv6: None,
                },
            },
        }
    }

    #[test]
    fn subscription_feedback_accepts_slower_cadence_but_never_faster_than_requested() {
        let mut value = response();
        let id = value.task.subscription_id.unwrap();
        assert!(subscription_fields(id, Some(1000), &value).is_ok());
        assert!(subscription_fields(id, Some(2000), &value).is_ok());
        value.task.interval_ms = 1000;
        assert!(matches!(
            subscription_fields(id, Some(2000), &value),
            Err(Error::Contract)
        ));
        value.task.phase = ObservationPhase::Unsupported;
        value.task.unsupported_metrics = value.task.metrics.clone();
        assert!(subscription_fields(id, Some(1000), &value).is_ok());
    }

    #[test]
    fn subscription_feedback_rejects_noncanonical_or_terminal_control_hostnames() {
        let mut value = response();
        let id = value.task.subscription_id.unwrap();
        for hostname in [
            "target.mesh.example",
            "UPPERCASE",
            "-target",
            "target-",
            "target\nforged",
            "\u{1b}[31mtarget",
            "",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ] {
            value.target.hostname = hostname.into();
            assert!(
                matches!(
                    subscription_fields(id, Some(1000), &value),
                    Err(Error::Contract)
                ),
                "{hostname:?}"
            );
        }
    }
}
