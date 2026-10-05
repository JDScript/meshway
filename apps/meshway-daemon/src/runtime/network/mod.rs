//! Local assembly only: Control owns authority, Host owns kernel state,
//! Manager owns adjacency, and SecureNode owns IP flows and circuit crypto.
#[cfg(target_os = "linux")]
mod discovery;
#[cfg(target_os = "linux")]
mod dns;
mod facts;
#[cfg(target_os = "linux")]
mod packets;
mod reporting;
mod requirements;
#[cfg(target_os = "linux")]
mod worker;

use chrono::{DateTime, Utc};
use meshway_control::{ApplicationTicket, Control, DataPlaneIdentity, ExecutionSnapshot};
use meshway_ipc::{DataPlane, Problem};
use meshway_model::ConfigurationRef;
use meshway_private_storage::Directory;
use meshway_protocol::{DemandRequest, RenewDemandRequest, execution::ApplyFailure};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, watch};

#[derive(Clone)]
pub(super) struct Report {
    pub plane: DataPlane,
    pub problem: Option<Problem>,
    pub guard: Option<ExecutionSnapshot>,
    pub dns_guard: Option<watch::Receiver<Option<meshway_dns::SystemDnsError>>>,
    pub ticket: Option<ApplicationTicket>,
    pub completion: Option<Result<(), ApplyFailure>>,
    pub observed_at: DateTime<Utc>,
    pub facts_observed_at: DateTime<Utc>,
    pub facts: Option<meshway_connectivity::Snapshot>,
    pub generation: meshway_model::Sequence,
    pub dependencies: std::collections::BTreeMap<meshway_model::NodeId, u32>,
}
impl Report {
    pub fn stopped() -> Self {
        Self {
            plane: DataPlane::Stopped,
            problem: None,
            guard: None,
            dns_guard: None,
            ticket: None,
            completion: None,
            observed_at: Utc::now(),
            facts_observed_at: Utc::now(),
            facts: None,
            generation: meshway_model::Sequence::MIN,
            dependencies: Default::default(),
        }
    }
    pub fn current(&self, control: &meshway_control::Status) -> bool {
        self.bound_to(control)
            && self
                .dns_guard
                .as_ref()
                .is_none_or(|guard| guard.has_changed().is_ok() && guard.borrow().is_none())
            && self
                .guard
                .as_ref()
                .is_none_or(|guard| guard.ensure_current().is_ok())
    }
    pub fn bound_to(&self, control: &meshway_control::Status) -> bool {
        let Some(ticket) = &self.ticket else {
            return false;
        };
        control.authenticated
            && control.instance.as_ref().is_some_and(|instance| {
                instance.instance_id == ticket.instance().instance_id
                    && instance.generation == ticket.instance().generation
                    && instance.target.as_ref() == Some(ticket.target())
            })
    }
    pub fn applied(&self) -> Option<ConfigurationRef> {
        match (&self.ticket, self.completion) {
            (Some(ticket), Some(Ok(()))) => Some(ticket.target().clone()),
            _ => None,
        }
    }
}

#[derive(Clone)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // Consumed by the Linux-only worker.
struct Target {
    identity: Option<DataPlaneIdentity>,
    ticket: ApplicationTicket,
    instance_expires_at_ms: u64,
    #[cfg(target_os = "linux")]
    dns: Option<dns::intents::Selection>,
    #[cfg(target_os = "linux")]
    retired_dns: std::collections::BTreeMap<meshway_model::RequestId, dns::intents::Retired>,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // Constructed by Linux packet ingress.
pub(super) enum Operation {
    Create(DemandRequest),
    Renew {
        request: RenewDemandRequest,
        prepared: bool,
    },
}
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // Consumed by Linux packet ingress.
pub(super) struct Reply {
    result: Result<(), ()>,
    renewal: Option<RenewDemandRequest>,
}
pub(super) enum Request {
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))] // Constructed by Linux packet ingress.
    Demand {
        operation: Operation,
        reply: oneshot::Sender<Reply>,
    },
    #[cfg(target_os = "linux")]
    Dns {
        name: String,
        deadline: tokio::time::Instant,
        reply: oneshot::Sender<
            Result<meshway_protocol::NodeDnsQueryResponse, meshway_dns::GatewayError>,
        >,
    },
}

struct Completion {
    ticket: ApplicationTicket,
    result: Result<(), ApplyFailure>,
    observed_at: DateTime<Utc>,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // Consumed by the Linux-only worker.
enum Command {
    Builder(oneshot::Sender<reqwest::ClientBuilder>),
    Sample(oneshot::Sender<reporting::Sample>),
    Stop {
        down: bool,
        reply: oneshot::Sender<Result<(), Problem>>,
    },
    Shutdown {
        keep_sink: bool,
    },
}

pub(super) struct Handle {
    commands: mpsc::Sender<Command>,
    target: watch::Sender<Option<Target>>,
    pub report: watch::Receiver<Report>,
    pub requests: mpsc::Receiver<Request>,
    worker: tokio::task::JoinHandle<Result<(), Problem>>,
    ticket: Option<ApplicationTicket>,
    outcome: Option<Result<(), ApplyFailure>>,
    retry: Retry,
    completions: mpsc::Receiver<Completion>,
    pending_completion: Option<Completion>,
    reporting: reporting::Schedule,
    #[cfg(target_os = "linux")]
    dns_plans: watch::Receiver<Option<dns::intents::Plan>>,
    #[cfg(target_os = "linux")]
    dns_intents: dns::intents::Registry,
}

impl Handle {
    pub async fn open(
        directory: Arc<Directory>,
        origin: &str,
        reports: watch::Sender<Report>,
        options: meshway_underlay::Options,
        dns_settings: meshway_dns::SystemDnsSettings,
    ) -> Result<Self, Problem> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (directory, origin, reports, options, dns_settings);
            Err(Problem::UnsupportedMode)
        }
        #[cfg(target_os = "linux")]
        {
            let mut options = options;
            // The owned file may still point at our stopped stub after a crash.
            // Recover its original resolver inputs before protected Host bootstrap.
            let reservation = if matches!(
                dns_settings.backend,
                meshway_dns::SystemDnsBackend::OwnedResolvConf { .. }
            ) {
                let reserved =
                    meshway_dns::SystemDnsOwner::reserve(dns_settings.clone(), directory.clone())
                        .await
                        .map_err(|_| Problem::Dns)?;
                if options.bootstrap_nameservers.is_none() && options.bootstrap_hosts.is_empty() {
                    options.bootstrap_nameservers = reserved
                        .bootstrap_nameservers()
                        .await
                        .map_err(|_| Problem::Dns)?;
                }
                Some(reserved)
            } else {
                None
            };
            let host = meshway_underlay::Host::open(directory.clone(), origin, options)
                .await
                .map_err(|error| {
                    tracing::warn!(%error, "Underlay startup failed");
                    Problem::Underlay
                })?;
            let (commands, received) = mpsc::channel(4);
            let (target, targets) = watch::channel(None);
            let report = reports.subscribe();
            let (reports, reporting) = reporting::channel(reports);
            let (gateway, requests) = mpsc::channel(16);
            let (completed, completions) = mpsc::channel(8);
            let (plans, dns_plans) = watch::channel(None);
            let dns = dns::Attachment::new(dns_settings, directory, reservation, gateway.clone());
            let worker = tokio::spawn(worker::run(
                host,
                received,
                targets,
                reports,
                gateway,
                completed,
                (dns, plans),
            ));
            Ok(Self {
                commands,
                target,
                report,
                requests,
                worker,
                ticket: None,
                outcome: None,
                retry: Retry::default(),
                completions,
                pending_completion: None,
                reporting,
                dns_plans,
                dns_intents: dns::intents::Registry::default(),
            })
        }
    }

    pub async fn client_builder(&self) -> Result<reqwest::ClientBuilder, Problem> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Builder(reply))
            .await
            .map_err(|_| Problem::Worker)?;
        result.await.map_err(|_| Problem::Worker)
    }

    pub async fn publish(&mut self, control: &mut Control) -> Result<(), meshway_control::Error> {
        #[cfg(target_os = "linux")]
        let dns = {
            let plan = self.dns_plans.borrow().clone();
            let applicable = plan.filter(|plan| {
                control
                    .status()
                    .instance
                    .as_ref()
                    .zip(control.cached_target())
                    .is_some_and(|(instance, target)| {
                        target.configuration.enabled
                            && plan.matches(
                                &meshway_protocol::InstanceRequest {
                                    instance_id: instance.instance_id,
                                    generation: instance.generation,
                                },
                                &target.configuration,
                            )
                    })
            });
            match applicable {
                Some(plan) => {
                    if self.dns_intents.advance(&plan, control).await? {
                        control.refresh_configuration().await?;
                    }
                    self.dns_intents.selection(&plan)
                }
                None => {
                    if control.cached_target().is_some_and(|target| {
                        !target.configuration.enabled
                            || (!requirements::dns_required(&target.configuration.dns.settings)
                                && matches!(
                                    target.configuration.dns.egress,
                                    meshway_configuration::execution::DnsEgress::NoExit
                                ))
                    }) {
                        self.dns_intents.deactivate();
                    }
                    None
                }
            }
        };
        #[cfg(target_os = "linux")]
        let retired_dns = self.dns_intents.retired(
            u64::try_from(Utc::now().timestamp_millis())
                .map_err(|_| meshway_control::Error::Expired)?,
        );
        let status = control.status();
        let Some(instance) = status.instance else {
            return Ok(());
        };
        let Some(reference) = instance.target else {
            return Ok(());
        };
        let disabled = control
            .cached_target()
            .is_some_and(|target| !target.configuration.enabled);
        let identity = if disabled {
            None
        } else {
            Some(control.dataplane_identity(control.execution_snapshot()?)?)
        };
        let same_target = self.ticket.as_ref().is_some_and(|ticket| {
            ticket.target() == &reference
                && ticket.instance().instance_id == instance.instance_id
                && ticket.instance().generation == instance.generation
        });
        if same_target {
            let installed_locally = {
                let report = self.report.borrow();
                report.completion == Some(Ok(()))
                    && report
                        .ticket
                        .as_ref()
                        .zip(self.ticket.as_ref())
                        .is_some_and(|(reported, ticket)| {
                            reported.instance() == ticket.instance()
                                && reported.target() == ticket.target()
                                && reported.attempt() == ticket.attempt()
                        })
            };
            if self.outcome == Some(Ok(())) && self.report.borrow().plane == DataPlane::Failed {
                // Runtime refresh failure does not rewrite the completed
                // installation receipt. Recovery is a separately saved attempt.
                self.outcome = Some(Err(if self.report.borrow().problem == Some(Problem::Dns) {
                    ApplyFailure::Dns
                } else {
                    ApplyFailure::Underlay
                }));
                self.retry.failed();
                return Ok(());
            }
            match self.outcome.or(installed_locally.then_some(Ok(()))) {
                None => {
                    #[cfg(target_os = "linux")]
                    if self
                        .target
                        .borrow()
                        .as_ref()
                        .is_some_and(|old| old.dns != dns)
                    {
                        self.target.send_replace(Some(Target {
                            identity,
                            ticket: self.ticket.as_ref().expect("same ticket").clone(),
                            instance_expires_at_ms: instance.expires_at_ms,
                            dns,
                            retired_dns,
                        }));
                    }
                    return Ok(());
                }
                Some(Err(failure)) => {
                    if !self.retry.ready(failure) {
                        return Ok(());
                    }
                }
                Some(Ok(())) => {
                    let previous = self.target.borrow().clone();
                    if let Some(previous) = previous {
                        match (&identity, &previous.identity) {
                            (None, None) => return Ok(()),
                            (Some(next), Some(old))
                                if next.snapshot().continuation_from(old.snapshot()).is_ok() =>
                            {
                                let changed = refresh_needed(next.snapshot(), old.snapshot());
                                #[cfg(target_os = "linux")]
                                let changed = changed || previous.dns != dns;
                                if changed {
                                    self.target.send_replace(Some(Target {
                                        identity,
                                        ticket: previous.ticket,
                                        instance_expires_at_ms: instance.expires_at_ms,
                                        #[cfg(target_os = "linux")]
                                        dns,
                                        #[cfg(target_os = "linux")]
                                        retired_dns,
                                    }));
                                }
                                return Ok(());
                            }
                            _ => {
                                // A real reinstall requires settling this
                                // attempt; a scope-only refresh above does not.
                                if self.outcome.is_none() {
                                    return Ok(());
                                }
                            }
                        }
                    }
                }
            }
        } else {
            self.retry = Retry::default();
        }
        self.ticket = Some(control.begin_application().await?);
        self.outcome = None;
        let ticket = self.ticket.as_ref().expect("application begun").clone();
        self.target.send_replace(Some(Target {
            identity,
            ticket,
            instance_expires_at_ms: instance.expires_at_ms,
            #[cfg(target_os = "linux")]
            dns,
            #[cfg(target_os = "linux")]
            retired_dns,
        }));
        Ok(())
    }

    pub async fn finish(&mut self, control: &mut Control) -> Result<(), meshway_control::Error> {
        // Once accepted, a completion belongs to this owner until its exact
        // ticket/timestamp is acknowledged or its instance is no longer ours.
        for _ in 0..8 {
            if self.pending_completion.is_none() {
                self.pending_completion = self.completions.try_recv().ok();
            }
            let Some(completion) = &self.pending_completion else {
                break;
            };
            let same_instance = control.status().instance.as_ref().is_some_and(|instance| {
                instance.instance_id == completion.ticket.instance().instance_id
                    && instance.generation == completion.ticket.instance().generation
            });
            if same_instance {
                control
                    .finish_application(
                        &completion.ticket,
                        completion.result,
                        completion.observed_at,
                    )
                    .await?;
                self.reporting.receipt_completed();
            }
            if self.ticket.as_ref().is_some_and(|ticket| {
                ticket.target() == completion.ticket.target()
                    && ticket.instance() == completion.ticket.instance()
                    && ticket.attempt() == completion.ticket.attempt()
            }) {
                self.outcome = Some(completion.result);
                if completion.result.is_err() {
                    self.retry.failed();
                }
            }
            self.pending_completion = None;
        }
        Ok(())
    }

    pub async fn stop(&mut self, down: bool) -> Result<(), Problem> {
        self.target.send_replace(None);
        self.ticket = None;
        self.outcome = None;
        self.retry = Retry::default();
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Stop { down, reply })
            .await
            .map_err(|_| Problem::Worker)?;
        result.await.map_err(|_| Problem::Worker)?
    }

    pub fn preparing_dns(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            self.report.borrow().plane == DataPlane::Preparing && self.dns_plans.borrow().is_some()
        }
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    }

    pub async fn shutdown(self, keep_sink: bool) -> Result<(), Problem> {
        self.target.send_replace(None);
        let _ = self.commands.send(Command::Shutdown { keep_sink }).await;
        self.worker.await.map_err(|_| Problem::Worker)?
    }

    async fn sample(&self) -> Result<reporting::Sample, ()> {
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            let (reply, sample) = oneshot::channel();
            self.commands
                .send(Command::Sample(reply))
                .await
                .map_err(|_| ())?;
            sample.await.map_err(|_| ())
        })
        .await
        .map_err(|_| ())?
    }

    pub async fn work(&mut self, report_changes: bool) -> Work {
        tokio::select! { biased;
            result = self.reporting.wait(), if report_changes => match result { Ok(()) => Work::Report, Err(()) => Work::Closed },
            request = self.requests.recv() => match request { Some(request) => Work::Request(request), None => Work::Closed },
        }
    }
}

pub(super) enum Work {
    Report,
    Request(Request),
    Closed,
}

fn refresh_needed(next: &ExecutionSnapshot, old: &ExecutionSnapshot) -> bool {
    // Atomic control refresh may return a later lease on every poll. Keep the
    // installed scope's original deadlines until its renewal margin, rather
    // than reopening physical channels for each otherwise unchanged response.
    // A new publication or shorter grant is always installed immediately.
    let remaining = old
        .expires_at()
        .saturating_duration_since(std::time::Instant::now());
    let remaining_ms = old
        .expires_at_ms()
        .saturating_sub(next.now_ms().unwrap_or(u64::MAX));
    !next.same_execution(old)
        || next.expires_at_ms() < old.expires_at_ms()
        || (next.expires_at_ms() > old.expires_at_ms()
            && (remaining <= std::time::Duration::from_secs(40) || remaining_ms <= 40_000))
}

/// Two timed retries for transient installation failures. Unsupported business
/// configuration requires a new target or explicit operator work (up revision).
#[derive(Default)]
struct Retry {
    failures: u8,
    at: Option<tokio::time::Instant>,
}
impl Retry {
    fn failed(&mut self) {
        self.failures = self.failures.saturating_add(1);
        self.at = Some(
            tokio::time::Instant::now()
                + std::time::Duration::from_secs(if self.failures == 1 { 5 } else { 15 }),
        );
    }
    fn ready(&self, failure: ApplyFailure) -> bool {
        !matches!(
            failure,
            ApplyFailure::Dns | ApplyFailure::Validation | ApplyFailure::Dataplane
        ) && self.failures < 3
            && self.at.is_some_and(|at| tokio::time::Instant::now() >= at)
    }
}

pub(super) async fn request(
    control: &mut Control,
    request: Request,
) -> Result<(), meshway_control::Error> {
    let (operation, reply) = match request {
        Request::Demand { operation, reply } => (operation, reply),
        #[cfg(target_os = "linux")]
        Request::Dns {
            name,
            deadline,
            mut reply,
        } => {
            if reply.is_closed() || tokio::time::Instant::now() >= deadline {
                return Ok(());
            }
            let result = tokio::select! { biased;
                _ = reply.closed() => return Ok(()),
                response = tokio::time::timeout_at(deadline, control.query_dns(&name)) => match response {
                    Ok(response) => response,
                    Err(_) => { let _ = reply.send(Err(meshway_dns::GatewayError)); return Ok(()); }
                },
            };
            return match result {
                Ok(response) => {
                    let _ = reply.send(Ok(response));
                    Ok(())
                }
                Err(error) => {
                    let _ = reply.send(Err(meshway_dns::GatewayError));
                    Err(error)
                }
            };
        }
    };
    let (result, renewal) = match operation {
        Operation::Create(value) => (
            control.create_demand(&value).await.map(|response| {
                if let meshway_configuration::execution::DemandDecision::Permitted { permit } =
                    response.decision
                {
                    tracing::debug!(
                        request_id = %value.request_id,
                        operation = "create",
                        protocol = ?value.protocol,
                        port = value.port.map(|port| port.get()),
                        expires_at_ms = permit.expires_at_ms,
                        "Demand request acknowledged"
                    );
                }
            }),
            None,
        ),
        Operation::Renew {
            mut request,
            prepared,
        } => {
            // The single Control owner chooses the first absolute deadline at
            // dispatch, using its current instance rather than a worker's old
            // execution lease. Failed delivery returns this exact retry value.
            if !prepared
                && let Some(instance) = control.status().instance
                && instance.instance_id == request.instance.instance_id
                && instance.generation == request.instance.generation
            {
                request.expires_at_ms = instance.expires_at_ms;
            }
            (
                control.renew_demand(&request).await.map(|response| {
                    if let meshway_configuration::execution::DemandDecision::Permitted { permit } =
                        response.decision
                    {
                        tracing::debug!(
                            request_id = %request.request_id,
                            operation = "renew",
                            expires_at_ms = permit.expires_at_ms,
                            "Demand request acknowledged"
                        );
                    }
                }),
                Some(request),
            )
        }
    };
    let _ = reply.send(Reply {
        result: result.as_ref().map(|_| ()).map_err(|_| ()),
        renewal,
    });
    result
}

#[cfg(test)]
mod tests {
    use super::Retry;
    use meshway_protocol::execution::ApplyFailure;

    #[test]
    fn installation_failure_has_a_finite_retry_budget_and_dns_needs_new_work() {
        let mut retry = Retry::default();
        for failure in 1..=3 {
            retry.failed();
            assert!(!retry.ready(ApplyFailure::Underlay));
            retry.at = Some(tokio::time::Instant::now() - std::time::Duration::from_secs(1));
            assert_eq!(retry.ready(ApplyFailure::Underlay), failure < 3);
            assert!(!retry.ready(ApplyFailure::Dns));
            assert!(!retry.ready(ApplyFailure::Dataplane));
        }
        for _ in 0..100 {
            assert!(!retry.ready(ApplyFailure::Underlay));
        }
    }
}
