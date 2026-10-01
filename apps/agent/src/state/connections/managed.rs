//! Generated-HTTP orchestration; the runtime watchdog never waits on HTTP.

use std::{path::PathBuf, time::Duration};

use meshway_protocol::{
    BeginConnectionRuntimeRequest, BeginConnectionRuntimeResponse,
    ConnectionRuntimeReason as Reason, GetConnectionRuntimeEpochResponse,
    ReportConnectionRuntimeRequest, ReportConnectionRuntimeResponse,
};
use tokio::time::{self, Instant};

use super::dataplane;
use super::{Agent, AgentError, HttpClient, cache_path, convert, local};
use crate::managed_connections::Manager;

struct RuntimeLease {
    instance_id: String,
    expected_epoch: Option<u32>,
    epoch: Option<u32>,
    sequence: u32,
}

impl Agent {
    /// IPv4-only coherent policy application. Never activates a cached grant at startup.
    pub async fn run_managed_dataplane(
        &mut self,
        profiles: Option<PathBuf>,
        interval: Duration,
        lease: Duration,
        options: crate::TunOptions,
    ) -> Result<(), AgentError> {
        options.validate()?;
        if !cfg!(target_os = "linux") {
            return Err(AgentError::Invalid("--apply-dataplane requires Linux"));
        }
        self.run_managed(profiles, interval, lease, Some(options))
            .await
    }

    /// Opt-in managed adjacencies. The ordinary saved-only sync API is unchanged.
    pub async fn run_managed_connections(
        &mut self,
        profiles: Option<PathBuf>,
        interval: Duration,
        lease: Duration,
    ) -> Result<(), AgentError> {
        self.run_managed(profiles, interval, lease, None).await
    }

    async fn run_managed(
        &mut self,
        profiles: Option<PathBuf>,
        interval: Duration,
        lease: Duration,
        options: Option<crate::TunOptions>,
    ) -> Result<(), AgentError> {
        if !(Duration::from_secs(10)..=Duration::from_secs(300)).contains(&lease)
            || interval.is_zero()
            || interval >= lease
        {
            return Err(AgentError::Invalid(
                "connection lease must be 10–300 seconds and exceed the polling interval",
            ));
        }
        let mut dataplane = options
            .as_ref()
            .map(|_| dataplane::State::load(self))
            .transpose()?;
        let mut manager = if let Some(options) = options {
            Manager::new_dataplane(
                profiles,
                lease,
                options,
                self.identity.controller.clone(),
                dataplane.as_ref().is_some_and(dataplane::State::has_cache),
            )
        } else {
            Manager::new(profiles, lease)
        };
        #[cfg(unix)]
        let control = crate::diagnostics::Server::bind(&self.path, &self._lock).await?;
        #[cfg(unix)]
        let prober = manager.prober();
        #[cfg(unix)]
        let result = tokio::select! {
            result = self.managed_loop(&mut manager, interval, &mut dataplane) => result,
            result = control.serve(prober) => result.map_err(AgentError::from),
            _ = tokio::signal::ctrl_c() => Ok(()),
        };
        #[cfg(not(unix))]
        let result = tokio::select! {
            result = self.managed_loop(&mut manager, interval, &mut dataplane) => result,
            _ = tokio::signal::ctrl_c() => Ok(()),
        };
        manager.withdraw(Reason::ShuttingDown).await;
        result
    }

    pub(super) async fn managed_loop(
        &mut self,
        manager: &mut Manager,
        interval: Duration,
        dataplane: &mut Option<dataplane::State>,
    ) -> Result<(), AgentError> {
        let mut lease = RuntimeLease {
            instance_id: local::credential()?,
            expected_epoch: None,
            epoch: None,
            sequence: 0,
        };
        let mut backoff = 1_u64;
        loop {
            let mut control_synced = false;
            let result = self
                .managed_tick(manager, &mut lease, &mut control_synced, dataplane)
                .await;
            // Recovery belongs to the validated, durable configuration sync,
            // not to the later best-effort observation report. The first probe
            // of a replacement can legitimately fence that report. Retaining an
            // outage backoff longer than the lease would expire every new
            // generation before its next tick and trap recovery indefinitely.
            if control_synced {
                backoff = 1;
            }
            let delay = match result {
                Ok(()) => {
                    backoff = 1;
                    interval
                }
                Err(AgentError::Http {
                    status: 422,
                    retryable: true,
                }) => {
                    // Revision/digest changed during report. Withdraw and pull;
                    // never label the old channels current or renew by reporting.
                    manager.withdraw(Reason::InvalidConfiguration).await;
                    Duration::from_secs(1)
                }
                Err(error) if error.retryable() => {
                    let delay = Duration::from_secs(backoff);
                    backoff = (backoff * 2).min(30);
                    eprintln!("{error}; retrying in {}s", delay.as_secs());
                    delay
                }
                Err(error) => {
                    manager.withdraw(Reason::InternalError).await;
                    return Err(error);
                }
            };
            time::sleep(delay).await;
        }
    }

    async fn managed_tick(
        &mut self,
        manager: &mut Manager,
        lease: &mut RuntimeLease,
        control_synced: &mut bool,
        dataplane: &mut Option<dataplane::State>,
    ) -> Result<(), AgentError> {
        self.heartbeat().await?;
        let (mut receipt, mut packet_snapshot) = self.managed_sync_mode(manager, dataplane).await?;
        if lease.epoch.is_none() {
            if lease.expected_epoch.is_none() {
                let response: GetConnectionRuntimeEpochResponse =
                    convert(self.runtime_client().get_connection_runtime_epoch().await?)?;
                // Pin once, together with the process nonce. A lost begin reply
                // must never refresh this CAS and steal a newer process's lease.
                lease.expected_epoch = Some(response.epoch);
            }
            let response = self
                .runtime_client()
                .begin_connection_runtime(convert(BeginConnectionRuntimeRequest {
                    instance_id: lease.instance_id.clone(),
                    expected_epoch: lease.expected_epoch.expect("startup epoch pinned"),
                })?)
                .await;
            let response = match response {
                Err(error) if error.api().is_some_and(|error| error.status == 409) => {
                    manager.withdraw(Reason::LeaseSuperseded).await;
                    return Err(AgentError::Invalid(
                        "runtime lease superseded; refusing to reacquire",
                    ));
                }
                Err(error) => return Err(error.into()),
                Ok(response) => response,
            };
            let response: BeginConnectionRuntimeResponse = convert(response)?;
            if lease.expected_epoch.and_then(|epoch| epoch.checked_add(1)) != Some(response.epoch) {
                return Err(AgentError::Invalid("invalid runtime lease response"));
            }
            lease.epoch = Some(response.epoch);
            // Pull while holding the runtime lease, not merely before begin.
            (receipt, packet_snapshot) = self.managed_sync_mode(manager, dataplane).await?;
        }
        let fresh_at = Instant::now();
        let state = self
            .connections
            .as_ref()
            .expect("freshly synced connections");
        state.check(&self.path, &self.identity)?;
        local::ensure_durable(&cache_path(&self.path))?;
        let key = state.keys.application_key(&self.path, &self.identity)?;
        let snapshot = state
            .cache
            .as_ref()
            .expect("fresh saved cache")
            .snapshot
            .clone();
        *control_synced = true;
        if let Some((snapshot, fresh_at)) = packet_snapshot {
            manager.reconcile_dataplane(snapshot, key, fresh_at).await?;
        } else {
            manager.reconcile(snapshot, key, fresh_at).await;
        }
        println!(
            "{}",
            serde_json::to_string(&receipt)
                .map_err(|_| AgentError::Invalid("cannot encode saved receipt"))?
        );
        lease.sequence = lease
            .sequence
            .checked_add(1)
            .ok_or(AgentError::Invalid("runtime report sequence exhausted"))?;
        let snapshot = manager.snapshot.as_ref().expect("reconciled snapshot");
        let (observed, mut guard) = manager.report_snapshot();
        let request = ReportConnectionRuntimeRequest {
            epoch: lease.epoch.expect("runtime lease acquired"),
            sequence: lease.sequence,
            revision: snapshot.payload.revision,
            digest: snapshot.digest.clone(),
            state: observed.state,
            reason: observed.reason,
            listeners: observed.listeners,
            peers: observed.peers,
        };
        let client = self.runtime_client();
        let body = convert(&request)?;
        if guard.as_ref().is_some_and(|guard| !guard.current()) {
            return Err(AgentError::RuntimeObservationChanged);
        }
        let response = if let Some(guard) = &mut guard {
            tokio::select! {
                biased;
                _ = guard.changed() => return Err(AgentError::RuntimeObservationChanged),
                response = client.report_connection_runtime(body) => response,
            }
        } else {
            client.report_connection_runtime(body).await
        };
        let response = match response {
            Err(error) if error.api().is_some_and(|error| error.status == 409) => {
                manager.withdraw(Reason::LeaseSuperseded).await;
                return Err(AgentError::Invalid(
                    "runtime lease superseded; refusing to reacquire",
                ));
            }
            Err(error) if error.api().is_some_and(|error| error.status == 422) => {
                return Err(AgentError::Http {
                    status: 422,
                    retryable: true,
                });
            }
            Err(error) => return Err(error.into()),
            Ok(response) => convert::<ReportConnectionRuntimeResponse>(response)?,
        };
        if response.epoch != request.epoch || response.sequence != request.sequence {
            return Err(AgentError::Invalid("runtime report response mismatch"));
        }
        if guard.as_ref().is_some_and(|guard| !guard.current()) {
            return Err(AgentError::RuntimeObservationChanged);
        }
        #[derive(serde::Serialize)]
        struct RuntimeReceipt<'a> {
            runtime: &'a ReportConnectionRuntimeRequest,
            reported_at: chrono::DateTime<chrono::Utc>,
        }
        println!(
            "{}",
            serde_json::to_string(&RuntimeReceipt {
                runtime: &request,
                reported_at: response.reported_at
            })
            .map_err(|_| AgentError::Invalid("cannot encode runtime receipt"))?
        );
        Ok(())
    }

    fn runtime_client(&self) -> HttpClient {
        HttpClient::new()
            .with_base_url(&self.identity.controller)
            .with_api_key(&self.identity.node_credential)
    }

    async fn managed_sync_mode(
        &mut self,
        manager: &mut Manager,
        dataplane: &mut Option<dataplane::State>,
    ) -> Result<
        (
            super::ControlSyncReceipts,
            Option<(meshway_protocol::AgentDataplaneSnapshot, Instant)>,
        ),
        AgentError,
    > {
        if let Some(dataplane) = dataplane {
            let synced = dataplane.sync(self, manager).await?;
            Ok((synced.receipt, Some((synced.snapshot, synced.fresh_at))))
        } else {
            Ok((self.managed_sync(manager).await?, None))
        }
    }

    async fn managed_sync(
        &mut self,
        manager: &Manager,
    ) -> Result<super::ControlSyncReceipts, AgentError> {
        self.publish_connection_identity().await?;
        let configuration = self.sync().await?;
        #[cfg(unix)]
        manager.set_probe_inventory(
            self.cache
                .as_ref()
                .expect("freshly synced configuration")
                .public_inventory(),
        );
        let connections = self
            .sync_published_connections_observe(|snapshot| manager.invalidate_if_changed(snapshot))
            .await?;
        Ok(super::ControlSyncReceipts {
            configuration,
            connections,
        })
    }
}
