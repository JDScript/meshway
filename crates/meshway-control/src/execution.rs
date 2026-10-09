use crate::{Control, Deadline, Error, Instance, SavedReceipt, Started, now, random_id, reference};
use meshway_configuration::execution::{CompiledNode, apply_configuration_delta};
use meshway_model::{
    ConfigurationRef, InstanceId, Sequence, Version,
    state::{Capability, ConfigurationState, NodeState, SystemState},
};
use meshway_protocol::{
    AcquireInstanceRequest, EnrollmentResponse, InstanceLease, NodeConfigurationRequest,
    NodeConfigurationResponse, ReceiptRequest, StateAccepted, execution::ReceiptPhase,
};
use std::collections::BTreeMap;

pub(super) fn validate_target(
    target: &CompiledNode,
    binding: &EnrollmentResponse,
) -> Result<(), Error> {
    let cfg = &target.configuration;
    if cfg.network_id != binding.network_id
        || cfg.node_id != binding.node_id
        || cfg.self_keys.control_key != binding.keys.control
        || cfg.self_keys.dataplane_key != binding.keys.dataplane
        || meshway_configuration::digest(cfg).map_err(|_| Error::Contract)? != target.digest
    {
        return Err(Error::Contract);
    }
    Ok(())
}

/// The enclosing lease binds local exit responsibilities. A valid digest alone
/// cannot make another instance's role executable by this Control owner.
pub(super) fn validate_exit_binding(
    cfg: &meshway_configuration::execution::NodeConfiguration,
    instance: InstanceId,
) -> Result<(), Error> {
    if cfg
        .exit_role
        .as_ref()
        .is_some_and(|role| !cfg.exit_enabled || role.instance_id != instance)
        || cfg.permits.values().any(|permit| {
            permit
                .exit
                .as_ref()
                .and_then(|exit| exit.roles.get(&cfg.node_id))
                .is_some_and(|role| cfg.exit_role.as_ref() != Some(role))
        })
    {
        return Err(Error::Contract);
    }
    Ok(())
}

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
        let instance = match instance(lease, started, request.capabilities) {
            Ok(instance) => instance,
            Err(error) => {
                self.fence();
                return Err(error);
            }
        };
        let lease = instance.lease.clone();
        self.invalidate_snapshots();
        self.instance = Some(instance);
        self.acquiring = None;
        Ok(lease)
    }

    pub async fn renew_instance(&mut self) -> Result<InstanceLease, Error> {
        let request = self.current_instance()?;
        let started = Started::now(&self.clock)?;
        let response = self
            .client
            .renew_node_instance(&request, self.token()?)
            .await;
        let lease = self.registered(response)?;
        self.update_lease(lease.clone(), started)?;
        Ok(lease)
    }

    /// Validate a complete target or shared-library delta and fsync it before
    /// creating a saved receipt. Query/replay never acknowledges installation.
    pub async fn sync_configuration(&mut self) -> Result<Option<ConfigurationRef>, Error> {
        let request = NodeConfigurationRequest {
            instance: self.current_instance()?,
            base: self.record.saved.as_ref().map(reference),
        };
        let started = Started::now(&self.clock)?;
        let response = self
            .client
            .query_node_configuration(&request, self.token()?)
            .await;
        let response = self.registered(response)?;
        self.receive_configuration(response, started).await
    }

    /// Explicitly renew and save the exact granted execution target from one
    /// Server transaction, without observing an intermediate lease-only target.
    pub async fn refresh_configuration(&mut self) -> Result<Option<ConfigurationRef>, Error> {
        let request = NodeConfigurationRequest {
            instance: self.current_instance()?,
            base: self.record.saved.as_ref().map(reference),
        };
        let started = Started::now(&self.clock)?;
        let response = self
            .client
            .refresh_node_configuration(&request, self.token()?)
            .await;
        let response = self.registered(response)?;
        self.receive_configuration(response, started).await
    }

    async fn receive_configuration(
        &mut self,
        response: NodeConfigurationResponse,
        started: Started,
    ) -> Result<Option<ConfigurationRef>, Error> {
        let target = match (response.full, response.delta) {
            (Some(full), None) => Some(full),
            (None, Some(delta)) => {
                let Some(base) = self.record.saved.as_ref() else {
                    self.fence();
                    return Err(Error::Contract);
                };
                match apply_configuration_delta(base, delta) {
                    Ok(target) => Some(target),
                    Err(_) => {
                        self.fence();
                        return Err(Error::Contract);
                    }
                }
            }
            (None, None) => None,
            _ => {
                self.fence();
                return Err(Error::Contract);
            }
        };
        if let Some(target) = &target {
            if validate_target(
                target,
                self.record.binding.as_ref().ok_or(Error::Lifecycle)?,
            )
            .is_err()
                || validate_exit_binding(
                    &target.configuration,
                    self.current_instance()?.instance_id,
                )
                .is_err()
                || Some(reference(target)) != response.lease.target
                || self
                    .instance
                    .as_ref()
                    .is_none_or(|i| i.capabilities != target.configuration.self_keys.capabilities)
            {
                self.fence();
                return Err(Error::Contract);
            }
        } else if response.lease.target.is_some()
            && self.record.saved.as_ref().map(reference) != response.lease.target
        {
            self.fence();
            return Err(Error::Contract);
        }
        self.update_lease(response.lease, started)?;
        if let Some(target) = target
            && self.record.saved.as_ref() != Some(&target)
        {
            self.record.saved = Some(target);
            self.receipt = None;
            // A possibly accepted State report keeps its sequence/sample until
            // its exact retry is acknowledged or this instance is fenced.
            self.persist_target().await?;
        }
        let desired = self.instance.as_ref().and_then(|i| i.lease.target.clone());
        if let Some(target) = &desired {
            if self
                .receipt
                .as_ref()
                .is_none_or(|receipt| &receipt.request.target != target)
            {
                self.receipt = Some(SavedReceipt {
                    request: ReceiptRequest {
                        instance: self.current_instance()?,
                        target: target.clone(),
                        phase: ReceiptPhase::Saved,
                        attempt: Version::MIN,
                        sequence: Sequence::MIN,
                        reported_at_ms: now()?,
                        failure: None,
                    },
                    acknowledged: false,
                });
            }
        } else {
            self.receipt = None;
        }
        Ok(desired)
    }

    /// Retries reuse the original attempt, sequence and report timestamp.
    pub async fn acknowledge_saved(&mut self) -> Result<(), Error> {
        let current = self.current_instance()?;
        let saved = self.receipt.as_ref().ok_or(Error::Lifecycle)?;
        let receipt = &saved.request;
        if receipt.instance != current
            || self.record.saved.as_ref().map(reference).as_ref() != Some(&receipt.target)
            || self
                .instance
                .as_ref()
                .is_none_or(|i| i.lease.target.as_ref() != Some(&receipt.target))
        {
            return Err(Error::Expired);
        }
        if saved.acknowledged {
            return Ok(());
        }
        let response = self
            .client
            .submit_node_configuration_receipt(receipt, self.token()?)
            .await;
        if let Err(error) = self.receipt_response(response) {
            if matches!(error, Error::ReceiptRejected(_)) {
                let saved = self.receipt.as_mut().ok_or(Error::Lifecycle)?;
                saved.request.attempt = saved
                    .request
                    .attempt
                    .checked_next()
                    .map_err(|_| Error::Lifecycle)?;
                saved.request.reported_at_ms = now()?;
            }
            return Err(error);
        }
        self.receipt.as_mut().ok_or(Error::Lifecycle)?.acknowledged = true;
        Ok(())
    }

    /// Report only facts this component owns. There is no installed transport,
    /// applied target, probe or link in a control-only run. System facts are
    /// supplied by the host. Use retry_state_report after uncertain delivery.
    pub async fn report_control_state(
        &mut self,
        system: SystemState,
    ) -> Result<StateAccepted, Error> {
        self.current_instance()?;
        let active = self.instance.as_ref().ok_or(Error::Lifecycle)?;
        let saved = self
            .record
            .saved
            .as_ref()
            .map(reference)
            .filter(|r| Some(r) == active.lease.target.as_ref());
        let state = NodeState {
            system,
            capabilities: active.capabilities.clone(),
            configuration: ConfigurationState {
                saved,
                applied: None,
                progress: None,
            },
            listeners: BTreeMap::new(),
            sockets: BTreeMap::new(),
            links: BTreeMap::new(),
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
        let sequence = report.sequence;
        let response = self.client.report_node_state(report, self.token()?).await;
        // Only this validated reporting-domain rejection is non-fencing. The
        // sample is not rewritten/reused with its sequence; a fresh observation
        // advances report_sequence after normal configuration synchronization.
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
        let accepted = self.registered(response)?;
        if accepted.sequence != sequence {
            self.fence();
            return Err(Error::Contract);
        }
        self.report = None;
        Ok(accepted)
    }

    fn update_lease(&mut self, lease: InstanceLease, started: Started) -> Result<(), Error> {
        let current = self.current_instance()?;
        if lease.instance_id != current.instance_id || lease.generation != current.generation {
            self.fence();
            return Err(Error::Contract);
        }
        let capabilities = self
            .instance
            .as_ref()
            .ok_or(Error::Lifecycle)?
            .capabilities
            .clone();
        match instance(lease, started, capabilities) {
            Ok(instance) => {
                let old = self.instance.as_ref().ok_or(Error::Lifecycle)?;
                let replaced = old.lease.target != instance.lease.target;
                let predecessor = old.lease.target.clone();
                // A replacement target is published before its execution lease
                // is explicitly renewed. Preserve only this direct handoff gap;
                // a same-target withdrawal is a hard continuity barrier.
                let pending = self.publication_pending_grant && old.execution_deadline.is_none();
                let barrier = !old.deadline.valid()
                    || (!old.execution_deadline.as_ref().is_some_and(Deadline::valid) && !pending)
                    || old.lease.target.is_none()
                    || instance.lease.target.is_none()
                    || instance.lease.expires_at_ms < old.lease.expires_at_ms
                    || (!replaced
                        && !pending
                        && instance.lease.execution_expires_at_ms
                            < old.lease.execution_expires_at_ms);
                if barrier {
                    self.invalidate_snapshots();
                } else if replaced {
                    self.invalidate_guards();
                }
                if replaced {
                    self.predecessor = if barrier {
                        None
                    } else {
                        predecessor.map(|reference| (self.publication.clone(), reference))
                    };
                    self.publication = std::sync::Arc::new(());
                }
                self.publication_pending_grant =
                    !barrier && instance.execution_deadline.is_none() && (replaced || pending);
                self.instance = Some(instance);
                Ok(())
            }
            Err(error) => {
                self.fence();
                Err(error)
            }
        }
    }
}

fn instance(
    lease: InstanceLease,
    started: Started,
    capabilities: BTreeMap<String, Capability>,
) -> Result<Instance, Error> {
    if lease
        .execution_expires_at_ms
        .is_some_and(|expires| expires > lease.expires_at_ms || lease.target.is_none())
    {
        return Err(Error::Contract);
    }
    let deadline = Deadline::from_response(started.clone(), lease.expires_at_ms)?;
    let execution_deadline = lease
        .execution_expires_at_ms
        .map(|expires| Deadline::from_response(started, expires))
        .transpose()?;
    Ok(Instance {
        lease,
        deadline,
        execution_deadline,
        capabilities,
    })
}
