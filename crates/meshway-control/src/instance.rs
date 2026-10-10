//! Runtime instance qualification and control-only State. The instance binds
//! State, receipts, coordination and observation results; it never bounds
//! netmap execution authority.
use crate::{Control, Deadline, Error, Instance, Started, random_id};
use meshway_model::{
    InstanceId,
    state::{Capability, ConfigurationState, NodeState, SystemState},
};
use meshway_protocol::{AcquireInstanceRequest, InstanceLease, StateAccepted};
use std::collections::BTreeMap;

impl Control {
    /// Capabilities come from the host's actual loaded implementations. The
    /// control-only component itself declares no transport capability.
    pub async fn acquire_instance(
        &mut self,
        capabilities: BTreeMap<String, Capability>,
    ) -> Result<InstanceLease, Error> {
        self.token()?;
        if let Some(instance) = &self.instance {
            if instance.deadline.valid() {
                if instance.capabilities != capabilities {
                    return Err(Error::Pending);
                }
                return Ok(instance.lease.clone());
            }
            self.fence();
        }
        if let Some(request) = &self.acquiring {
            if request.capabilities != capabilities {
                return Err(Error::Pending);
            }
        } else {
            self.acquiring = Some(AcquireInstanceRequest {
                instance_id: InstanceId::new(random_id()?).map_err(|_| Error::Cryptography)?,
                capabilities,
            });
        }
        let request = self.acquiring.as_ref().ok_or(Error::Lifecycle)?.clone();
        let started = Started::now(&self.clock)?;
        let response = self
            .client
            .acquire_node_instance(&request, self.token()?)
            .await;
        let lease = self.registered(response)?;
        if lease.instance_id != request.instance_id {
            self.fence();
            return Err(Error::Contract);
        }
        self.token()?;
        let deadline = match Deadline::from_response(started, lease.expires_at_ms) {
            Ok(deadline) => deadline,
            Err(error) => {
                self.fence();
                return Err(error.into());
            }
        };
        self.instance = Some(Instance {
            lease: lease.clone(),
            deadline,
            capabilities: request.capabilities,
        });
        self.acquiring = None;
        Ok(lease)
    }

    /// Extend the current instance. A shorter or foreign lease fences it.
    pub async fn renew_instance(&mut self) -> Result<InstanceLease, Error> {
        let request = self.current_instance()?;
        let started = Started::now(&self.clock)?;
        let response = self
            .client
            .renew_node_instance(&request, self.token()?)
            .await;
        let lease = self.registered(response)?;
        let current = self.current_instance()?;
        let old = self.instance.as_ref().ok_or(Error::Lifecycle)?;
        if lease.instance_id != current.instance_id
            || lease.generation != current.generation
            || lease.expires_at_ms < old.lease.expires_at_ms
        {
            self.fence();
            return Err(Error::Contract);
        }
        let deadline = match Deadline::from_response(started, lease.expires_at_ms) {
            Ok(deadline) => deadline,
            Err(error) => {
                self.fence();
                return Err(error.into());
            }
        };
        let instance = self.instance.as_mut().ok_or(Error::Lifecycle)?;
        instance.lease = lease.clone();
        instance.deadline = deadline;
        Ok(lease)
    }

    /// Report only facts this component owns. There is no installed transport,
    /// applied netmap, probe or link in a control-only run. System facts are
    /// supplied by the host. Use retry_state_report after uncertain delivery.
    pub async fn report_control_state(
        &mut self,
        system: SystemState,
    ) -> Result<StateAccepted, Error> {
        self.current_instance()?;
        let active = self.instance.as_ref().ok_or(Error::Lifecycle)?;
        let state = NodeState {
            system,
            capabilities: active.capabilities.clone(),
            configuration: ConfigurationState {
                saved: self.netmap_reference(),
                applied: None,
                progress: None,
            },
            listeners: BTreeMap::new(),
            sockets: BTreeMap::new(),
            links: BTreeMap::new(),
            adjacencies: Default::default(),
            paths: BTreeMap::new(),
            selection: Default::default(),
            probes: Vec::new(),
            problems: Vec::new(),
            exit_capability: None,
        };
        self.report_runtime_state(state, chrono::Utc::now()).await
    }

    pub async fn retry_state_report(&mut self) -> Result<StateAccepted, Error> {
        let instance = self.current_instance()?;
        let report = self.report.as_ref().ok_or(Error::Lifecycle)?;
        if report.instance_id != instance.instance_id {
            return Err(Error::Lifecycle);
        }
        // An identical retry past the Server's report lifetime can only be
        // rejected; the next report is a fresh observation instead.
        if report.observed_at
            + chrono::Duration::seconds(meshway_protocol::STATE_REPORT_TTL_SECONDS)
            <= chrono::Utc::now()
        {
            self.report = None;
            return Err(Error::Lifecycle);
        }
        let sequence = report.sequence;
        let response = self.client.report_node_state(report, self.token()?).await;
        // Only this validated reporting-domain rejection is non-fencing. The
        // sample is not rewritten/reused with its sequence; a fresh observation
        // advances report_sequence after the next sync.
        if matches!(
            &response,
            Err(meshway_control_client::ClientError::Http {
                status: 409,
                problem: Some(meshway_protocol::ErrorResponse {
                    status: 409,
                    problem: meshway_protocol::Problem::StateScopeChanged { .. },
                    ..
                }),
                ..
            })
        ) {
            self.report = None;
            return Err(Error::StateScopeChanged);
        }
        // A decoded 400 rejects this body (for example an exit capability
        // observation the Server no longer accepts as fresh). Retrying the
        // identical body cannot succeed; the host observes again.
        if matches!(
            &response,
            Err(meshway_control_client::ClientError::Http {
                status: 400,
                problem: Some(meshway_protocol::ErrorResponse { status: 400, .. }),
            })
        ) {
            self.report = None;
            return Err(Error::StaleReport);
        }
        let accepted = self.registered(response)?;
        if accepted.sequence != sequence {
            self.fence();
            return Err(Error::Contract);
        }
        self.report = None;
        Ok(accepted)
    }
}
