//! Bounded resolver intent registry, driven only by the existing Control owner.
use meshway_configuration::{
    DnsConfiguration, DnsTransport,
    execution::{BusinessTarget, DnsEgress, ExecutionAddress, NodeConfiguration},
};
use meshway_control::Control;
use meshway_dns::{ResolverDemand, ResolverSource};
use meshway_model::RequestId;
use meshway_protocol::{DemandRequest, InstanceRequest, RenewDemandRequest};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    time::Duration,
};
use tokio::time::Instant;

pub(in crate::runtime::network) const PREPARE_TIMEOUT: Duration = Duration::from_secs(180);
const MAX_ACTIVE: usize = 256;
const MAX_TOTAL: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::runtime::network) struct Resolver {
    source: ResolverSource,
    endpoint: SocketAddr,
    transport: DnsTransport,
    profile: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::runtime::network) struct Plan {
    instance: InstanceRequest,
    settings: DnsConfiguration,
    profile: String,
    egress: DnsEgress,
    resolvers: Vec<Resolver>,
}
impl Plan {
    pub fn new(
        instance: InstanceRequest,
        cfg: &NodeConfiguration,
        demands: Vec<ResolverDemand>,
    ) -> Self {
        Self {
            instance,
            settings: cfg.dns.settings.clone(),
            profile: cfg.dns.profile.clone(),
            egress: cfg.dns.egress.clone(),
            resolvers: demands
                .into_iter()
                .map(|d| Resolver {
                    source: d.source,
                    endpoint: d.endpoint,
                    transport: d.transport,
                    profile: d.profile,
                })
                .collect(),
        }
    }
    pub fn matches(&self, instance: &InstanceRequest, cfg: &NodeConfiguration) -> bool {
        &self.instance == instance
            && self.settings == cfg.dns.settings
            && self.profile == cfg.dns.profile
            && self.egress == cfg.dns.egress
    }
    pub fn empty(&self) -> bool {
        self.resolvers.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::runtime::network) struct Selection {
    pub plan: Plan,
    pub ids: BTreeMap<ResolverSource, RequestId>,
    pub failed: bool,
}
struct Intent {
    resolver: Resolver,
    request: DemandRequest,
    created: bool,
    renewal: Option<RenewDemandRequest>,
    // Scheduling acknowledgement only; the installed permit remains the sole
    // DNS I/O authority. Older snapshots cannot undo a completed renewal RPC.
    renewed_until: u64,
    expires_at_ms: u64,
    retry: Instant,
}

impl Intent {
    fn prepare_renewal(&mut self, now_ms: u64, instance_expiry: u64) {
        if !self.created || self.renewal.is_some() {
            return;
        }
        let known_expiry = self.expires_at_ms.max(self.renewed_until);
        if known_expiry > now_ms.saturating_add(40_000) {
            return;
        }
        let expires_at_ms = now_ms.saturating_add(120_000).min(instance_expiry);
        if expires_at_ms > known_expiry {
            self.renewal = Some(RenewDemandRequest {
                instance: self.request.instance.clone(),
                request_id: self.request.request_id,
                expires_at_ms,
            });
        }
    }

    fn acknowledged(&mut self) {
        self.created = true;
        if let Some(renewal) = self.renewal.take() {
            self.renewed_until = renewal.expires_at_ms;
        }
    }
}

/// Inert owner-created metadata retained only until its original bounded
/// lifetime. This type never enters Scope or a resolver socket route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::runtime::network) struct Retired {
    request: DemandRequest,
    expires_at_ms: u64,
}
impl Retired {
    pub fn matches(
        &self,
        source: meshway_model::NodeId,
        permit: &meshway_configuration::execution::LocalEgressPermit,
        now_ms: u64,
    ) -> bool {
        self.expires_at_ms > now_ms
            && permit.expires_at_ms > now_ms
            && permit.expires_at_ms <= self.expires_at_ms
            && permit.demand_id == self.request.request_id
            && permit.source == source
            && permit.target == self.request.target
            && permit.protocol == self.request.protocol
            && permit.port == self.request.port
            && permit.profile == self.request.profile
    }
}
impl From<Intent> for Retired {
    fn from(intent: Intent) -> Self {
        Self {
            expires_at_ms: intent
                .renewal
                .as_ref()
                .map_or(intent.expires_at_ms, |renewal| {
                    renewal.expires_at_ms.max(intent.expires_at_ms)
                })
                .max(intent.renewed_until),
            request: intent.request,
        }
    }
}
#[derive(Default)]
pub(in crate::runtime::network) struct Registry {
    plan: Option<Plan>,
    intents: Vec<Intent>,
    retired: Vec<Retired>,
    deadline: Option<Instant>,
    failed: bool,
    published: BTreeSet<RequestId>,
}
impl Registry {
    fn reconcile(&mut self, plan: &Plan, now_ms: u64, instance_expiry: u64) {
        if self.plan.as_ref() == Some(plan) {
            return;
        }
        self.retired.retain(|entry| entry.expires_at_ms > now_ms);
        let previous = self.plan.replace(plan.clone());
        let same_instance = previous
            .as_ref()
            .is_some_and(|old| old.instance == plan.instance);
        let mut retained = Vec::new();
        for intent in self.intents.drain(..) {
            if same_instance && plan.resolvers.contains(&intent.resolver) {
                retained.push(intent);
            } else {
                // Cancelling a demand fences all packet snapshots. Let unused,
                // bounded old intents expire without disrupting unrelated flows.
                self.retired.push(intent.into());
            }
        }
        self.intents = retained;
        self.retired.retain(|entry| entry.expires_at_ms > now_ms);
        self.deadline = Some(Instant::now() + PREPARE_TIMEOUT);
        self.failed = plan.resolvers.len() > MAX_ACTIVE
            || plan.resolvers.len() + self.retired.len() > MAX_TOTAL;
        if self.failed {
            return;
        }
        for resolver in &plan.resolvers {
            if self
                .intents
                .iter()
                .any(|intent| &intent.resolver == resolver)
            {
                continue;
            }
            let Some(request_id) = super::super::packets::request_id() else {
                self.failed = true;
                return;
            };
            let Ok(port) = resolver.endpoint.port().try_into() else {
                self.failed = true;
                return;
            };
            self.intents.push(Intent {
                resolver: resolver.clone(),
                request: DemandRequest {
                    instance: plan.instance.clone(),
                    request_id,
                    target: BusinessTarget::Ip {
                        address: ExecutionAddress(resolver.endpoint.ip()),
                    },
                    protocol: match resolver.transport {
                        DnsTransport::Udp => {
                            meshway_configuration::execution::BusinessProtocol::Udp
                        }
                        DnsTransport::Tcp => {
                            meshway_configuration::execution::BusinessProtocol::Tcp
                        }
                    },
                    port: Some(port),
                    profile: resolver.profile.clone(),
                    duration_seconds: 120,
                },
                created: false,
                renewal: None,
                renewed_until: 0,
                expires_at_ms: now_ms.saturating_add(120_000).min(instance_expiry),
                retry: Instant::now(),
            });
        }
    }
    /// At most eight HTTP operations per cycle. Interrupted/unknown delivery
    /// keeps the exact request and absolute renewal deadline for the retry.
    pub async fn advance(
        &mut self,
        plan: &Plan,
        control: &mut Control,
    ) -> Result<(), meshway_control::Error> {
        let snapshot = control.execution_snapshot()?;
        let now_ms = snapshot.now_ms()?;
        let Some(instance) = control.status().instance else {
            return Ok(());
        };
        self.reconcile(plan, now_ms, instance.expires_at_ms);
        if self.failed {
            return Ok(());
        }
        let cfg = snapshot.configuration()?;
        self.published = cfg.local_egress.keys().copied().collect();
        let mut budget = 8;
        for intent in &mut self.intents {
            if let Some(permit) = cfg.local_egress.get(&intent.request.request_id) {
                intent.expires_at_ms = permit.expires_at_ms;
            }
            if now_ms >= intent.expires_at_ms.max(intent.renewed_until) {
                self.failed = true;
                break;
            }
            if budget == 0 || Instant::now() < intent.retry {
                continue;
            }
            intent.prepare_renewal(now_ms, instance.expires_at_ms);
            if intent.created && intent.renewal.is_none() {
                continue;
            }
            budget -= 1;
            intent.retry = Instant::now() + Duration::from_secs(2);
            let result = tokio::time::timeout(Duration::from_secs(5), async {
                if !intent.created {
                    control.create_demand(&intent.request).await.map(|_| ())
                } else {
                    control
                        .renew_demand(intent.renewal.as_ref().expect("prepared renewal"))
                        .await
                        .map(|_| ())
                }
            })
            .await;
            if matches!(result, Ok(Ok(()))) {
                intent.acknowledged();
            }
        }
        let waiting = self
            .intents
            .iter()
            .any(|intent| !intent.created || !self.published.contains(&intent.request.request_id));
        if waiting && self.deadline.is_none() {
            self.deadline = Some(Instant::now() + PREPARE_TIMEOUT);
        }
        if !waiting {
            self.deadline = None;
        }
        if waiting && self.deadline.is_some_and(|at| Instant::now() >= at) {
            self.failed = true;
        }
        Ok(())
    }
    pub fn selection(&self, plan: &Plan) -> Option<Selection> {
        if self.plan.as_ref() != Some(plan) {
            return None;
        }
        if !self.failed
            && self.intents.iter().any(|intent| {
                !intent.created || !self.published.contains(&intent.request.request_id)
            })
        {
            return None;
        }
        Some(Selection {
            plan: plan.clone(),
            ids: self
                .intents
                .iter()
                .map(|intent| (intent.resolver.source.clone(), intent.request.request_id))
                .collect(),
            failed: self.failed,
        })
    }

    pub fn deactivate(&mut self) {
        self.retired
            .extend(self.intents.drain(..).map(Retired::from));
        self.plan = None;
        self.published.clear();
        self.deadline = None;
        self.failed = false;
    }

    pub fn retired(&mut self, now_ms: u64) -> BTreeMap<RequestId, Retired> {
        self.retired.retain(|entry| entry.expires_at_ms > now_ms);
        self.retired
            .iter()
            .map(|entry| (entry.request.request_id, entry.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> Plan {
        Plan {
            instance: InstanceRequest {
                instance_id: "00000000-0000-4000-8000-000000000001".parse().unwrap(),
                generation: meshway_model::Version::MIN,
            },
            settings: serde_json::from_value(serde_json::json!({"domain":"mesh.test"})).unwrap(),
            profile: "default".into(),
            egress: DnsEgress::NoExit,
            resolvers: vec![Resolver {
                source: ResolverSource::Configured("primary".into()),
                endpoint: "192.0.2.53:53".parse().unwrap(),
                transport: DnsTransport::Udp,
                profile: "default".into(),
            }],
        }
    }
    #[test]
    fn stable_retries_do_not_extend_original_expiry_or_replace_ids() {
        let plan = plan();
        let mut registry = Registry::default();
        registry.reconcile(&plan, 1_000, 90_000);
        let id = registry.intents[0].request.request_id;
        let deadline = registry.deadline;
        registry.reconcile(&plan, 30_000, 150_000);
        assert_eq!(registry.intents[0].request.request_id, id);
        assert_eq!(registry.intents[0].expires_at_ms, 90_000);
        assert_eq!(registry.deadline, deadline);
        assert!(registry.selection(&plan).is_none());
    }
    #[test]
    fn changed_endpoint_retires_old_id_without_reusing_authority() {
        let mut plan = plan();
        let mut registry = Registry::default();
        registry.reconcile(&plan, 1_000, 90_000);
        let id = registry.intents[0].request.request_id;
        plan.resolvers[0].endpoint = "192.0.2.54:53".parse().unwrap();
        registry.reconcile(&plan, 2_000, 90_000);
        assert_ne!(registry.intents[0].request.request_id, id);
        assert_eq!(registry.retired.len(), 1);
        assert_eq!(registry.retired[0].expires_at_ms, 90_000);
        assert_eq!(registry.retired[0].request.request_id, id);
    }

    #[test]
    fn acknowledged_renewal_is_not_reissued_while_its_publication_is_pending() {
        let plan = plan();
        let mut registry = Registry::default();
        registry.reconcile(&plan, 1_000, 90_000);
        let intent = &mut registry.intents[0];
        intent.acknowledged();
        intent.prepare_renewal(60_000, 180_000);
        let renewal = intent.renewal.clone().unwrap();
        assert_eq!(renewal.expires_at_ms, 180_000);
        intent.acknowledged();
        // publish_after_finish sees the same original execution permit. That
        // must neither extend its authority nor send another renewal HTTP.
        assert_eq!(intent.expires_at_ms, 90_000);
        intent.prepare_renewal(65_000, 185_000);
        assert!(intent.renewal.is_none());
        intent.expires_at_ms = 180_000;
        intent.prepare_renewal(145_000, 265_000);
        let next = intent.renewal.as_ref().unwrap();
        assert_eq!(next.request_id, renewal.request_id);
        assert_eq!(next.expires_at_ms, 265_000);
    }

    #[test]
    fn uncertain_renewal_keeps_its_absolute_deadline_across_plan_updates() {
        let mut plan = plan();
        let mut registry = Registry::default();
        registry.reconcile(&plan, 1_000, 90_000);
        let renewal = RenewDemandRequest {
            instance: plan.instance.clone(),
            request_id: registry.intents[0].request.request_id,
            expires_at_ms: 130_000,
        };
        registry.intents[0].created = true;
        registry.intents[0].renewal = Some(renewal.clone());
        registry.reconcile(&plan, 60_000, 180_000);
        plan.settings.search_domains.push("other.test".into());
        registry.reconcile(&plan, 65_000, 185_000);
        assert_eq!(registry.intents[0].renewal.as_ref(), Some(&renewal));
        assert_eq!(registry.intents[0].request.request_id, renewal.request_id);
    }

    #[test]
    fn retired_dns_bindings_are_exact_expiring_metadata_not_active_selection() {
        let plan = plan();
        let mut registry = Registry::default();
        registry.reconcile(&plan, 1_000, 90_000);
        let request = registry.intents[0].request.clone();
        registry.deactivate();
        assert!(registry.selection(&plan).is_none());
        let retired = registry.retired(2_000);
        let binding = &retired[&request.request_id];
        let source = "00000000-0000-4000-8000-000000000003".parse().unwrap();
        let mut permit = meshway_configuration::execution::LocalEgressPermit {
            demand_id: request.request_id,
            source,
            target: request.target,
            protocol: request.protocol,
            port: request.port,
            profile: request.profile,
            action: meshway_configuration::execution::LocalEgressAction::Direct,
            expires_at_ms: 90_000,
        };
        assert!(binding.matches(source, &permit, 2_000));
        permit.port = Some(443.try_into().unwrap());
        assert!(!binding.matches(source, &permit, 2_000));
        permit.port = Some(53.try_into().unwrap());
        permit.expires_at_ms += 1;
        assert!(!binding.matches(source, &permit, 2_000));
        assert!(registry.retired(90_000).is_empty());
    }
}
