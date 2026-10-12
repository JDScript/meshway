//! Stable netmap projection and on-demand local routing evaluation.
mod capture;
mod dns;
mod edge_constraints;
mod exit;
mod matching;
pub mod netmap;
mod paths;
mod plan;
mod routing;
pub mod rules;
mod selection;
mod transports;

use crate::{SubjectContext, topology_allows};
pub use capture::{
    EgressPartition, MAX_CAPTURE_RANGES, MAX_CAPTURE_ROUTES, MAX_WORK as MAX_CAPTURE_WORK,
    Partition, retained_capture, summary as capture_summary, unavailable_capture,
};
pub use edge_constraints::MAX_EDGE_TRANSPORT_ISSUES;
pub(crate) use edge_constraints::validate_edge_transports;
pub use exit::ExitInputs;
pub use matching::profile_allowed;
use meshway_configuration::{execution::*, *};
use meshway_model::{NetworkId, NodeId, Revision};
pub use plan::{PlanDestination, PlanRequest, plan, plan_with_exit};
pub use rules::{Artifacts, RuleSets};
pub use selection::routing_selection;
use std::collections::BTreeMap;

pub const MAX_PATHS: usize = 4;
/// Ordered exits have a separate budget from alternatives per exit.
pub const MAX_EXIT_CANDIDATES: usize = 16;
pub const MAX_HOPS: usize = 8;

/// Stable graph and authenticated declarations. Runtime facts only annotate
/// management evaluation; they cannot issue paths or permissions.
#[derive(Clone, Copy)]
pub struct Topology<'a> {
    pub configuration: &'a NetworkConfiguration,
    pub subjects: &'a SubjectContext,
    pub inventory: &'a BTreeMap<NodeId, TrustedNode>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CompileError {
    #[error("compilation budget exceeded")]
    Budget,
    #[error("inconsistent trusted compilation input")]
    InvalidInput,
    #[error("complete transport material is missing or incompatible")]
    Material,
    #[error(
        "/topology/{rule}/transports: edge {a}--{b} has no allowed transport in either dial direction"
    )]
    EdgeTransportUnavailable { rule: usize, a: NodeId, b: NodeId },
    #[error("execution dependency is unavailable")]
    UnavailableDependency,
    #[error("compilation serialization failed")]
    Serialization,
}
impl From<meshway_configuration::SerializationError> for CompileError {
    fn from(_: meshway_configuration::SerializationError) -> Self {
        Self::Serialization
    }
}

fn effective_profile(
    configuration: &NetworkConfiguration,
    subjects: &SubjectContext,
    selected: &BTreeMap<NodeId, String>,
    node: NodeId,
) -> Result<String, CompileError> {
    match selected.get(&node) {
        Some(profile) if matching::profile_allowed(configuration, subjects, node, profile)? => {
            Ok(profile.clone())
        }
        _ => matching::default_profile(configuration, subjects, node),
    }
}
